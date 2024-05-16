use io_uring::{cqueue, squeue};
use lsio_io::Output;
use lsio_threadpool::WorkerThread;

use crate::{
    operation::{NextStep, Operation, UringOperation},
    tracker::Tracker,
    user_data::UringUserData,
};

/// `MAX_SQ_ENTRIES_PER_ITERATION` describes the most SQEs that will be submitted to the io_uring SQ by
/// a single iteration of the `run` loop. This constant is used to make sure we have enough
/// headroom in the SQ before each iteration of the `run` loop.
const MAX_SQ_ENTRIES_PER_ITERATION: usize = 2;

/// Size of the io_uring submission queue (SQ).
const SQ_RING_SIZE: usize = 64;

/// We keep filling the SQ until we hit the "high water line" before we start draining the
/// completion queue. This ensures that we allow io_uring to process as many operations in parallel
/// as possible.
const HIGH_WATER_LINE: usize = SQ_RING_SIZE / 2;

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
        assert!(MAX_SQ_ENTRIES_PER_ITERATION < SQ_RING_SIZE);

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
                if self.uring.completion().is_empty() {
                    // The SQ is full but no completion events are ready! So we have no choice:
                    // We *have* to wait for some completion events to to complete:
                    self.uring.submit_and_wait(1).unwrap();
                }
                // If the CQ is not empty, then we fall through to the CQ processing loop.
            } else {
                match self.worker_thread.find_task() {
                    Some(mut operation) => {
                        // Submit first step of `operation`, and track `operation`:
                        let index_of_op = self.ops_in_flight.get_next_index().unwrap();
                        operation
                            .submit_first_step(index_of_op, &mut self.uring.submission())
                            .unwrap();
                        // TODO: Instead of calling `submit()` on every loop, we should keep our
                        // own check on how long has elapsed since we last submitted to the SQ,
                        // and only call `submit()` when we know the SQ has gone to sleep.
                        // `submit()` loads an AtomicBool twice (with Acquire memory ordering).
                        self.uring.submitter().submit().unwrap();
                        self.ops_in_flight.put(index_of_op, operation);
                        if self.sq_len_plus_cq_len() < HIGH_WATER_LINE {
                            // We want to "top up" the SQ before we process any CQEs.
                            // Without this, we run the risk of submitting one SQE, then draining
                            // that CQE, then submitting another SQE, and training that CQE, etc.
                            // In other words, we run the risk of not letting io_uring handle
                            // multiple SQEs at once!
                            continue;
                        }
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
                let mut op_guard = self.ops_in_flight.get(idx_of_op).unwrap();
                let next_step = op_guard.as_mut().process_opcode_and_submit_next_step(
                    &idx_and_opcode,
                    cqe.result(),
                    &mut unsafe { self.uring.submission_shared() },
                    &self.worker_thread,
                    &mut self.output_tx,
                );
                match next_step {
                    NextStep::Pending => (), // By default, op_guard will keep the operation.
                    NextStep::ReplaceWith(op) => op_guard.replace(op),
                    NextStep::Done => {
                        let _ = op_guard.remove();
                    }
                };
            }
        }
        assert!(self.ops_in_flight.is_empty());
    }

    /// io_uring submission queue (SQ) length plus the io_uring completion queue (CQ) length:
    fn sq_len_plus_cq_len(&self) -> usize {
        unsafe { self.uring.submission_shared().len() + self.uring.completion_shared().len() }
    }

    fn uring_is_full(&self) -> bool {
        self.sq_len_plus_cq_len() >= SQ_RING_SIZE - MAX_SQ_ENTRIES_PER_ITERATION
    }
}
