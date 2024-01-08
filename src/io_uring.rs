use io_uring::{opcode, types, IoUring};
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

struct OperationDescriptor {
    buf: Vec<u8>,
    task_i: usize,
    path: PathBuf,

    // Keeping the file descriptor in this struct is just a quick hack to ensure that
    // we keep the file descriptor open until io_uring has finished with this task.
    // TODO: Remove the file descriptor from this struct once we let io_uring open files!
    fd: fs::File,
}

// TODO: Refactor this function! Extract code into separate functions.
fn submit_and_process(tasks: &[PathBuf], transform: fn(anyhow::Result<OperationDescriptor>)) {
    const CQ_RING_SIZE: u32 = 16;
    let mut ring = IoUring::new(CQ_RING_SIZE).unwrap();
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;
    let n_tasks = tasks.len();

    // Keep io_uring's submission queue topped up, and process chunks:
    let mut task_i = 0;
    rayon::scope(|s| {
        while task_i < n_tasks {
            // Keep io_uring submission queue topped up. But don't overload io_uring!
            while task_i < n_tasks && n_tasks_in_flight_in_io_uring < CQ_RING_SIZE {
                let task = &tasks[task_i];
                println!("task_i={}, path={:?}", task_i, task);
                submit_task(task, task_i, &mut ring);

                // Increment counters
                task_i += 1;
                n_tasks_in_flight_in_io_uring += 1;
            }

            ring.submit_and_wait(1).unwrap(); // TODO: Handle error!

            // Spawn tasks to the Rayon ThreadPool to process data:
            for cqe in ring.completion() {
                n_tasks_in_flight_in_io_uring -= 1;

                // Prepare data for thread:
                let op_descriptor =
                    unsafe { Box::from_raw(cqe.user_data() as *mut OperationDescriptor) };
                let op_descriptor = if cqe.result() < 0 {
                    // An error has occurred!
                    let err = nix::Error::from_i32(-cqe.result());
                    Err(err.into())
                    // TODO: Handle when the number of bytes read is less than the number of bytes requested
                } else {
                    Ok(*op_descriptor)
                };

                // Spawn task to Rayon's ThreadPool:
                s.spawn(move |_| {
                    transform(op_descriptor);
                });
            }
        }
    });
    assert!(n_tasks_in_flight_in_io_uring == 0);
}

fn submit_task(task: &PathBuf, task_i: usize, ring: &mut IoUring) {
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn it_works() {
        let tasks = [PathBuf::from_str("/home/jack/dev/rust/light-speed-io/README.md").unwrap()];

        // Start a thread which is responsible for storing results in a Vector.
        // TODO: Consider using crossbeam::ArrayQueue to store the finished OpDescriptors. See issue #17.
        // let n_tasks = tasks.len();
        // let (tx, rx) = mpsc::sync_channel(64);
        // let store_thread = thread::spawn(move || {
        //     let mut results = Vec::with_capacity(n_tasks);
        //     for _ in 0..n_tasks {
        //         let op_descriptor: OperationDescriptor =
        //             rx.recv().expect("Unable to receive from channel");
        //         // TODO: Initialise `results` and use task_i as the index into `results`.
        //         //       Or, don't do that! And, instead, sort the returned vector.
        //         //       Or, leave as-is, and let the user sort the vector if they want!
        //         //       Or, don't return _any_ buffers?!
        //         //       See issue #17.
        //         results.push(op_descriptor);
        //     }
        //     results
        // });

        // TODO: Figure out how to share `transform` between threads if `transform`
        // is a closure. We want `transform` to be a closure so it can capture surrounding state.
        // In this example, we want to clone `tx` for each thread. Elsewhere, we might want
        // to share access to a single numpy array (to write each chunk into the array).
        // This conversation might provide the answer:
        // https://users.rust-lang.org/t/how-to-send-function-closure-to-another-thread/43549
        // Then we can re-enable the commented out code in the body of `it_works()`.
        // See Issue #19.
        fn transform(op_descriptor: anyhow::Result<OperationDescriptor>) {
            let op_descriptor = op_descriptor.unwrap();
            let buf = &op_descriptor.buf;
            println!("{:?}", std::str::from_utf8(buf).unwrap());
            // tx.clone().send(op_descriptor).expect("Unable to send on channel");
        }

        submit_and_process(&tasks, transform);

        // let results = store_thread
        //     .join()
        //     .expect("The receiver thread has panicked");

        // println!("results.len()={}", results.len());
    }
}
