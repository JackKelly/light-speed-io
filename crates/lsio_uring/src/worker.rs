use io_uring::{cqueue, squeue};
use lsio_io::Output;
use lsio_threadpool::WorkerThread;

use crate::{
    operation::{NextStep, Operation, UringOperation},
    tracker::Tracker,
    user_data::UringUserData,
};

const MAX_ENTRIES_AT_ONCE: usize = 2;
const SQ_RING_SIZE: usize = 64;

pub struct UringWorker {
    uring: io_uring::IoUring,
    ops_in_flight: Tracker<Operation>,
    worker_thread: WorkerThread<Operation>,
    output_tx: crossbeam_channel::Sender<anyhow::Result<Output>>,
}

impl UringWorker {
    pub(crate) fn new(
        worker_thread: WorkerThread<Operation>,
        output_tx: crossbeam_channel::Sender<anyhow::Result<Output>>,
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
            worker_thread,
            output_tx,
        }
    }

    /// The main loop for the thread.
    pub(crate) fn run(&mut self) {
        while self.worker_thread.keep_running() {
            if self.uring_is_full() {
                // Our io_uring is full! So we have no choice: we *have* to wait for some SQEs to complete:
                self.uring.submit_and_wait(1);
            } else {
                match self.worker_thread.find_task() {
                    Some(mut operation) => {
                        // Submit first step of `operation`, and track `operation`:
                        let index_of_op = self.ops_in_flight.get_next_index().unwrap();
                        operation.submit_first_step(index_of_op, &mut self.uring.submission());
                        self.ops_in_flight.put(index_of_op, operation);
                    }
                    None => {
                        // There are no new operations to submit, so let's work out if we need to
                        // park or process the completion queue.
                        if self.ops_in_flight.is_empty() {
                            // There's nothing to do! So we have to sleep:
                            self.worker_thread.park();
                            // When we wake, there definitely won't be anything in our uring, so
                            // continue to the top of the while loop:
                            continue;
                        }
                    }
                }
            }

            for cqe in unsafe { self.uring.completion_shared() } {
                let idx_and_opcode = UringUserData::from(cqe.user_data());
                let idx_of_op = idx_and_opcode.index_of_op() as usize;
                let op = self.ops_in_flight.as_mut(idx_of_op).unwrap();
                let next_step = op.process_opcode_and_submit_next_step(
                    &idx_and_opcode,
                    cqe.result(),
                    &mut unsafe { self.uring.submission_shared() },
                    &self.worker_thread,
                    &mut self.output_tx,
                );
                match next_step {
                    NextStep::Pending(op) | NextStep::ReplaceWith(op) => {
                        self.ops_in_flight.put(idx_of_op, op);
                    }
                    NextStep::Done => {
                        self.ops_in_flight.remove(idx_of_op).unwrap();
                    }
                };
            }
        }
        assert!(self.ops_in_flight.is_empty());
    }

    fn sq_len_plus_cq_len(&self) -> usize {
        unsafe { self.uring.submission_shared().len() + self.uring.completion_shared().len() }
    }

    fn uring_is_full(&self) -> bool {
        self.sq_len_plus_cq_len() >= SQ_RING_SIZE - MAX_ENTRIES_AT_ONCE
    }
}
