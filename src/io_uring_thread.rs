use std::{
    sync::{mpsc::{Receiver, Sender}, Arc},
    thread::JoinHandle,
};
use io_uring::{cqueue, opcode, types, IoUring};

use crate::operation_future::{Operation, SharedState};

#[derive(Debug)]
pub(crate) struct WorkerThread {
    pub(crate) handle: JoinHandle<()>,
    pub(crate) sender: Sender<Arc<SharedState>>, // Channel to send ops to the worker thread
}

pub(crate) fn worker_thread_func(rx: Receiver<Arc<SharedState>>) {
    const CQ_RING_SIZE: u32 = 16;  // TODO: This should be user-configurable.
    let mut ring = IoUring::new(CQ_RING_SIZE).unwrap();
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;

    loop {
        // Keep io_uring's submission queue topped up:
        // `try_iter` "will return None if there are no more pending values or if the channel has
        // hung up. The iterator will never panic! or block"
        for shared_state in rx.try_iter() {
            if n_tasks_in_flight_in_io_uring >= CQ_RING_SIZE {
                break;
            }

            let op = shared_state.get_operation();
            let entry = op.prepare_io_uring_entry();
            println!("Submitting op={:?}", op);
            submit_to_io_uring(entry, &mut ring);

            // Increment counters
            n_tasks_in_flight_in_io_uring += 1;
        }

        ring.submit_and_wait(1).unwrap(); // TODO: Handle error!

        println!("After ring.submit_and_wait");

        // Spawn tasks to the Rayon ThreadPool to process data:
        for cqe in ring.completion() {
            n_tasks_in_flight_in_io_uring -= 1;
            todo!();  // TODO: Get the associated Future and `set_result_and_wake()`
        }
    }
}

fn submit_operation_to_io_uring(op: Operation, ring: &mut IoUring) {
    // TODO: Open file using io_uring. See issue #1
    let fd = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECT)
        .open(task)
        .unwrap();

    // Save information about this task in an OperationDescriptor on the heap,
    // so the processing thread can access this information later.
    // Later, we'll get a raw pointer to this OperationDescriptor, and pass this raw pointer
    // through to the worker thread, via io_uring's `user_data` (which is what `user_data`
    // is mostly intended for, according to the `io_uring` docs). We get a raw pointer by calling
    // `into_raw()`, which consumes the OperationDescriptor but doesn't de-allocated it, which is exactly
    // what we want. We want ownership of the OperationDescriptor to "tunnel through" io_uring.
    // Rust will guarantee that we can't touch the buffer until it re-emerges from io_uring.
    // And we want Rayon's worker thread (that processes the CQE) to decide whether
    // to drop the buffer (after moving data elsewhere) or keep the buffer
    // (if we're passing the buffer back to the user).
    let mut op_descriptor = Box::new(OperationDescriptor {
        // TODO: Allocate the correct sized buffer for the task.
        //       Or don't allocate at all, if the user has already allocated.
        buf: vec![0u8; 1024],
        path: task.clone(),
        task_i,
        fd,
    });

    // Note that the developer needs to ensure
    // that the entry pushed into submission queue is valid (e.g. fd, buffer).
    let read_e = opcode::Read::new(
        types::Fd(op_descriptor.fd.as_raw_fd()),
        op_descriptor.buf.as_mut_ptr(),
        op_descriptor.buf.len() as _,
    )
    .build()
    .user_data(Box::into_raw(op_descriptor) as u64);

    unsafe {
        ring.submission()
            .push(&read_e)
            .expect("submission queue full")
    }
}