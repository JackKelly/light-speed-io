use io_uring::squeue;
use io_uring::IoUring;
use std::sync::mpsc::{Receiver, RecvError, TryRecvError};

use crate::operation::{Operation, OperationWithCallback};

pub(crate) fn worker_thread_func(rx: Receiver<OperationWithCallback>) {
    const CQ_RING_SIZE: u32 = 16; // TODO: Enable the user to configure this.
    let mut ring = IoUring::new(CQ_RING_SIZE).unwrap();
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;

    'outer: loop {
        // Keep io_uring's submission queue topped up:
        // TODO: Extract this inner loop into a separate function!
        'inner: loop {
            let op_with_callback = match n_tasks_in_flight_in_io_uring {
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
            let sq_entry = op_with_callback.get_operation().as_ref().unwrap().to_iouring_entry();
            let sq_entry = sq_entry.user_data(todo!()); // TODO: Add user data!
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
            todo!(); // TODO: Get the associated `OperationWithCallback` and call `execute_callback()`!
        }
    }
}

impl Operation {
    pub(crate) fn to_iouring_entry(&self) -> squeue::Entry {
        match self {
            Operation::Get { location, .. } => {
                // TODO:
                // 1. Get filesize. (DON'T do this in the critical section of the Mutex!)
                // 2. Allocate buffer, and assign it to InnerState.output
                // 3. Create squeue::Entry
                // 4. Return a PreparedEntry
                todo!();
            }
        }
    }
}
