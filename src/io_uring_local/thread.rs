use io_uring::IoUring;
use std::sync::mpsc::{Receiver, RecvError, TryRecvError};

use crate::io_uring_local::prep_operation_for_io_uring::prepare_io_uring_entry;
use crate::operation_future::SharedState;

pub(crate) fn worker_thread_func(rx: Receiver<SharedState>) {
    const CQ_RING_SIZE: u32 = 16; // TODO: Enable the user to configure this.
    let mut ring = IoUring::new(CQ_RING_SIZE).unwrap();
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;

    'outer: loop {
        // Keep io_uring's submission queue topped up:
        // TODO: Extract this inner loop into a separate function!
        'inner: loop {
            let shared_state = match n_tasks_in_flight_in_io_uring {
                0 => match rx.recv() {
                    // There are no tasks in flight in io_uring, so all that's
                    // left to do is to wait for more tasks.
                    Ok(s) => s,
                    Err(RecvError) => break 'outer, // The caller hung up.
                },
                CQ_RING_SIZE.. => break 'inner, // The CQ is full!
                _ => match rx.try_recv() {
                    Ok(s) => s,
                    Err(TryRecvError::Empty) => break 'inner,
                    Err(TryRecvError::Disconnected) => break 'outer,
                },
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
