use io_uring::cqueue;
use io_uring::squeue;
use io_uring::IoUring;
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::{Receiver, RecvError};

use crate::object_store_adapter::OpAndOutputChan;
use crate::tracker::Tracker;
use crate::{operation, uring, uring::Operation};

const MAX_FILES_TO_REGISTER: usize = 16;
const MAX_ENTRIES_PER_CHAIN: usize = 2; // Maximum number of io_uring entries per io_uring chain.
const SQ_RING_SIZE: usize = MAX_FILES_TO_REGISTER * MAX_ENTRIES_PER_CHAIN; // TODO: Allow the user to configure SQ_RING_SIZE.

pub struct Worker {
    ring: IoUring,
    n_files_registered: usize,

    // These are the `squeue::Entry`s generated within this thread.
    // Each inner `Vec<Entry>` will be submitted in one go. Each chain of linked entries
    // must be in its own inner `Vec<Entry>`.
    internal_op_queue: VecDeque<Vec<squeue::Entry>>,

    // These are the tasks that the user submits via `rx`.
    user_tasks_in_flight: Tracker<Box<dyn uring::Operation + Send>>,
}

impl Worker {
    #[allow(clippy::assertions_on_constants)]
    pub fn new() -> Self {
        assert!(MAX_ENTRIES_PER_CHAIN < SQ_RING_SIZE);

        let ring: IoUring<squeue::Entry, cqueue::Entry> = io_uring::IoUring::builder()
            .setup_sqpoll(1000) // The kernel sqpoll thread will sleep after this many milliseconds.
            // TODO: Allow the user to decide whether sqpoll is used.
            .build(SQ_RING_SIZE as _)
            .expect("Failed to initialise io_uring.");

        assert_eq!(ring.params().cq_entries(), ring.params().sq_entries() * 2);

        // Register "fixed" file descriptors, for use in chaining SQ entries.
        // io_uring supports a max of 16 registered ring descriptors. See:
        // https://manpages.debian.org/unstable/liburing-dev/io_uring_register.2.en.html#IORING_REGISTER_RING_FDS
        ring.submitter()
            .register_files_sparse(MAX_FILES_TO_REGISTER as _)
            .expect("Failed to register files!");

        Self {
            ring,
            n_files_registered: 0,
            internal_op_queue: VecDeque::with_capacity(SQ_RING_SIZE),
            user_tasks_in_flight: Tracker::new(SQ_RING_SIZE),
        }
    }

    /// The main loop for the thread.
    pub(crate) fn worker_thread_func(&mut self, mut rx: Receiver<OpAndOutputChan>) {
        loop {
            // The internal queue always takes precedence over the injector queue.
            self.move_entries_from_internal_queue_to_uring_sq();

            // If there's space in io_uring's SQ, then add SQEs from the injector queue:
            if self
                .move_entries_from_injector_queue_to_uring_sq(&mut rx)
                .is_err()
            {
                break;
            }

            self.submit_and_maybe_wait();
            self.process_uring_cq();
        }
        assert!(self.user_tasks_in_flight.is_empty());
    }

    /// Keep io_uring's submission queue topped up from this thread's internal queue.
    /// The internal queue always takes precedence over tasks from the user.
    fn move_entries_from_internal_queue_to_uring_sq(&mut self) {
        while !self.uring_is_full() {
            match self.internal_op_queue.pop_front() {
                None => break,
                Some(entries) => {
                    unsafe {
                        self.ring
                            .submission()
                            .push_multiple(entries.as_slice())
                            .unwrap()
                    };
                }
            }
        }
    }

    /// Keep io_uring's submission queue topped up with tasks from the user.
    fn move_entries_from_injector_queue_to_uring_sq(
        &mut self,
        rx: &mut Receiver<OpAndOutputChan>,
    ) -> Result<(), RecvError> {
        // TODO: The `n_files_registered < MAX_FILES_TO_REGISTER` check is only appropriate while
        // Operations are only ever `get` Operations.
        while !self.uring_is_full() && self.n_files_registered < MAX_FILES_TO_REGISTER {
            let op = if self.user_tasks_in_flight.is_empty() {
                // There are no tasks in flight in io_uring, so all that's
                // left to do is to block and wait for more `Operations` from the user.
                match rx.recv() {
                    Ok(s) => s,
                    Err(RecvError) => return Err(RecvError), // The caller hung up.
                }
            } else {
                match rx.try_recv() {
                    Ok(s) => s,
                    Err(TryRecvError::Empty) => return Ok(()),
                    Err(TryRecvError::Disconnected) => return Err(RecvError), // The caller hung up.
                }
            };

            let mut op = match op.op {
                operation::Operation::Get { path } => uring::Get::new(path, op.output_channel),
                _ => panic!("Not implemented yet!"),
            };

            // Build one or more `squeue::Entry`, submit to io_uring, and stash the op for access later.
            let index_of_op = self.user_tasks_in_flight.get_next_index().unwrap();
            let entries = match op.next_step(index_of_op) {
                uring::NextStep::SubmitEntries {
                    entries,
                    registers_file,
                } => {
                    if registers_file {
                        self.n_files_registered += 1;
                    }
                    entries
                }
                _ => panic!("next_step should only return first entries here!"),
            };
            unsafe {
                self.ring
                    .submission()
                    .push_multiple(entries.as_slice())
                    .unwrap()
            };
            self.user_tasks_in_flight.put(index_of_op, Box::new(op));
        }
        Ok(())
    }

    fn submit_and_maybe_wait(&mut self) {
        if self.ring.completion().is_empty() && !self.user_tasks_in_flight.is_empty() {
            self.ring.submit_and_wait(1).unwrap();
        } else {
            // We need to call `ring.submit()` the first time we submit. And, if sqpoll is enabled, then
            // we also need to call `ring.submit()` to waken the kernel polling thread.
            // `ring.submit()` is basically a no-op if the kernel's sqpoll thread is still awake.
            self.ring.submit().unwrap();
        }
    }

    fn process_uring_cq(&mut self) {
        for cqe in self.ring.completion() {
            // user_data holds the io_uring opcode in the lower 32 bits,
            // and holds the index_of_op in the upper 32 bits.
            let index_of_op = (cqe.user_data() >> 32) as usize;
            let op = self.user_tasks_in_flight.as_mut(index_of_op).unwrap();
            op.process_cqe(cqe);
            match op.next_step(index_of_op) {
                uring::NextStep::SubmitEntries {
                    entries,
                    registers_file: false,
                } => {
                    self.internal_op_queue.push_back(entries);
                }
                uring::NextStep::SubmitEntries {
                    entries: _,
                    registers_file: true,
                } => panic!("registers_file should not be true for a subsequent SQE!"),
                uring::NextStep::MaybeDone {
                    unregisters_file,
                    done,
                    ..
                } => {
                    if done {
                        self.user_tasks_in_flight.remove(index_of_op).unwrap();
                    }
                    if unregisters_file {
                        self.n_files_registered -= 1;
                    }
                }
                _ => (),
            }
        }
    }

    fn sq_len_plus_cq_len(&self) -> usize {
        unsafe { self.ring.submission_shared().len() + self.ring.completion_shared().len() }
    }

    fn uring_is_full(&self) -> bool {
        self.sq_len_plus_cq_len() >= SQ_RING_SIZE - MAX_ENTRIES_PER_CHAIN
    }
}
