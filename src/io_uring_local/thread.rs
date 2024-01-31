use io_uring::IoUring;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::io_uring_local::prep_operation_for_io_uring::prepare_io_uring_entry;
use crate::operation_future::SharedState;

pub(crate) fn worker_thread_func(rx: Receiver<SharedState>) {
    const CQ_RING_SIZE: u32 = 16; // TODO: This should be user-configurable.
    let mut ring = IoUring::new(CQ_RING_SIZE).unwrap();
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;

    'outer: loop {
        // Keep io_uring's submission queue topped up:
        'inner: while n_tasks_in_flight_in_io_uring < CQ_RING_SIZE {
            let shared_state = match rx.try_recv() {
                Ok(s) => s,
                Err(TryRecvError::Empty) => break 'inner,
                Err(TryRecvError::Disconnected) => break 'outer,
            };

            // Convert `Operation` to a `PreparedEntry`.
            let prepared_entry = prepare_io_uring_entry(&shared_state);
            println!("Submitting PreparedEntry={:?}", prepared_entry);
            let sq_entry = prepared_entry.sq_entry.user_data(todo!()); // TODO: Add user data!
            unsafe {
                ring.submission()
                    .push(&sq_entry)
                    .expect("io_uring submission queue full")
            }

            // Increment counters
            n_tasks_in_flight_in_io_uring += 1;
        }

        ring.submit_and_wait(1).unwrap(); // TODO: Handle error!

        println!("After ring.submit_and_wait");

        // Spawn tasks to the Rayon ThreadPool to process data:
        for cqe in ring.completion() {
            n_tasks_in_flight_in_io_uring -= 1;
            todo!(); // TODO: Get the associated Future and `set_result_and_wake()`
        }
    }
}
