use std::iter;

use crossbeam::deque;
use io_uring::{cqueue, squeue};

use crate::{
    operation::{NextStep, Operation},
    tracker::Tracker,
};

const MAX_ENTRIES_AT_ONCE: usize = 2;
const SQ_RING_SIZE: usize = 32;

pub struct UringWorker<'a> {
    uring: io_uring::IoUring,
    ops_in_flight: Tracker<Operation>,

    // Queues for work stealing
    global_queue: &'a deque::Injector<Operation>,
    local_queue: deque::Worker<Operation>,
    stealers: &'a [deque::Stealer<Operation>],
}

impl<'a> UringWorker<'a> {
    pub fn new(
        global_queue: &'a deque::Injector<Operation>,
        local_queue: deque::Worker<Operation>,
        stealers: &'a [deque::Stealer<Operation>],
    ) -> Self {
        assert!(MAX_ENTRIES_AT_ONCE < SQ_RING_SIZE);

        let ring: io_uring::IoUring<squeue::Entry, cqueue::Entry> = io_uring::IoUring::builder()
            // TODO: Allow the user to decide whether sqpoll is used.
            .setup_sqpoll(1000) // The kernel sqpoll thread will sleep after this many milliseconds.
            .build(SQ_RING_SIZE as _)
            .expect("Failed to initialise io_uring.");

        assert_eq!(ring.params().cq_entries(), ring.params().sq_entries() * 2);

        Self {
            uring: ring,
            ops_in_flight: Tracker::new(SQ_RING_SIZE),
            global_queue,
            local_queue,
            stealers,
        }
    }

    /// The main loop for the thread.
    pub(crate) fn worker_thread_func(&mut self) {
        loop {
            let op = self.find_op();
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
        assert!(self.ops_in_flight.is_empty());
    }

    /// Keep io_uring's submission queue topped up from this thread's internal queue.
    /// The internal queue always takes precedence over tasks from the user.
    fn move_entries_from_internal_queue_to_uring_sq(&mut self) {
        while !self.uring_is_full() {
            match self.internal_op_queue.pop_front() {
                None => break,
                Some(entries) => {
                    unsafe {
                        self.uring
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
            let op = if self.ops_in_flight.is_empty() {
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

            let mut op: Box<dyn uring::Operation + Send> = match op.op {
                operation::Operation::GetRange { path, range } => {
                    Box::new(uring::GetRange::new(path, range, op.output_channel))
                }
                _ => panic!("Not implemented yet!"),
            };

            // Build one or more `squeue::Entry`, submit to io_uring, and stash the op for access later.
            let index_of_op = self.ops_in_flight.get_next_index().unwrap();
            let entries = match op.next_step(index_of_op) {
                uring::NextStep::SubmitEntries {
                    entries,
                    register_file: registers_file,
                } => {
                    if registers_file {
                        self.n_files_registered += 1;
                    }
                    entries
                }
                _ => panic!("next_step should only return first entries here!"),
            };
            unsafe {
                self.uring
                    .submission()
                    .push_multiple(entries.as_slice())
                    .unwrap()
            };
            self.ops_in_flight.put(index_of_op, op);
        }
        Ok(())
    }

    fn submit_and_maybe_wait(&mut self) {
        if self.uring.completion().is_empty() && !self.ops_in_flight.is_empty() {
            self.uring.submit_and_wait(1).unwrap();
        } else {
            // We need to call `ring.submit()` the first time we submit. And, if sqpoll is enabled, then
            // we also need to call `ring.submit()` to waken the kernel polling thread.
            // `ring.submit()` is basically a no-op if the kernel's sqpoll thread is still awake.
            self.uring.submit().unwrap();
        }
    }

    fn process_uring_cq(&mut self) {
        for cqe in self.uring.completion() {
            // user_data holds the io_uring opcode in the lower 32 bits,
            // and holds the index_of_op in the upper 32 bits.
            let index_of_op = (cqe.user_data() >> 32) as usize;
            let op = self.ops_in_flight.as_mut(index_of_op).unwrap();
            op.process_cqe(cqe);
            match op.next_step(index_of_op) {
                uring::NextStep::SubmitEntries {
                    entries,
                    register_file: false,
                } => {
                    self.internal_op_queue.push_back(entries);
                }
                uring::NextStep::SubmitEntries {
                    entries: _,
                    register_file: true,
                } => panic!("registers_file should not be true for a subsequent SQE!"),
                uring::NextStep::Done {
                    unregister_file: unregisters_file,
                } => {
                    self.ops_in_flight.remove(index_of_op).unwrap();
                    if unregisters_file {
                        self.n_files_registered -= 1;
                    }
                }
                uring::NextStep::Pending => (),
            }
        }
    }

    fn sq_len_plus_cq_len(&self) -> usize {
        unsafe { self.uring.submission_shared().len() + self.uring.completion_shared().len() }
    }

    fn uring_is_full(&self) -> bool {
        self.sq_len_plus_cq_len() >= SQ_RING_SIZE - MAX_ENTRIES_AT_ONCE
    }
}
