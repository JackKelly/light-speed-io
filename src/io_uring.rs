use crossbeam::queue;
use io_uring::{cqueue, opcode, types, IoUring};
use rayon::Scope;
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
struct OperationDescriptor {
    buf: Vec<u8>,
    task_i: usize,
    path: PathBuf,

    // Keeping the file descriptor in this struct is just a quick hack to ensure that
    // we keep the file descriptor open until io_uring has finished with this task.
    // TODO: Remove the file descriptor from this struct once we let io_uring open files!
    fd: fs::File,
}

/// The `transform` function must handle the case when the number of
/// bytes read is less than the number of bytes requested.
fn submit_and_process<F>(tasks: &[PathBuf], transform: F)
where
    F: Fn(anyhow::Result<OperationDescriptor>) + Send + Sync + Copy,
{
    const CQ_RING_SIZE: u32 = 16;
    let mut ring = IoUring::new(CQ_RING_SIZE).unwrap();
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;
    let n_tasks = tasks.len();

    //let transform = Arc::new(transform);

    // Keep io_uring's submission queue topped up, and process chunks:
    let mut task_i = 0;
    rayon::scope(|scope| {
        while task_i < n_tasks {
            // Keep io_uring submission queue topped up. But don't overload io_uring!
            while task_i < n_tasks && n_tasks_in_flight_in_io_uring < CQ_RING_SIZE {
                let task = &tasks[task_i];
                println!("Submitting task_i={}, path={:?}", task_i, task);
                submit_task_to_io_uring(task, task_i, &mut ring);

                // Increment counters
                task_i += 1;
                n_tasks_in_flight_in_io_uring += 1;
            }

            ring.submit_and_wait(1).unwrap(); // TODO: Handle error!

            println!("After ring.submit_and_wait");

            // Spawn tasks to the Rayon ThreadPool to process data:
            for cqe in ring.completion() {
                n_tasks_in_flight_in_io_uring -= 1;
                spawn_processing_task(&cqe, transform, scope);
            }
        }
    });
    assert!(n_tasks_in_flight_in_io_uring == 0);
}

fn submit_task_to_io_uring(task: &PathBuf, task_i: usize, ring: &mut IoUring) {
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

fn spawn_processing_task<'scope, F>(cqe: &cqueue::Entry, transform: F, scope: &Scope<'scope>)
where
    F: Fn(anyhow::Result<OperationDescriptor>) + Send + Sync + 'scope,
{
    // Turn `op_descriptor` into a `Result<OperationDescriptor>` depending on `cqe.result()`:
    let op_descriptor = unsafe { Box::from_raw(cqe.user_data() as *mut OperationDescriptor) };
    let op_descriptor = if cqe.result() < 0 {
        let err = nix::Error::from_i32(-cqe.result());
        Err(err.into())
    } else {
        Ok(*op_descriptor)
    };

    // Spawn task to Rayon's ThreadPool:
    scope.spawn(move |_| {
        transform(op_descriptor);
    });
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn it_works() {
        let tasks = [
            PathBuf::from_str("/home/jack/dev/rust/light-speed-io/README.md").unwrap(),
            PathBuf::from_str("/home/jack/dev/rust/light-speed-io/design.md").unwrap(),
        ];

        // Use an crossbeam::ArrayQueue to store the finished `Result<OperationDescriptor>`s.
        let n_tasks = tasks.len();
        let results_queue = queue::ArrayQueue::new(n_tasks);

        // We want `transform` to be a closure so it can capture surrounding state (`results_queue`).
        // Elsewhere, we might want to share access to a single numpy array
        // (to write each chunk into the array). See Issue #19.
        let transform = |op_descriptor: anyhow::Result<OperationDescriptor>| {
            results_queue.push(op_descriptor).unwrap();
        };

        submit_and_process(&tasks, transform);

        // Get results back out of the ArrayQueue:
        println!("**************** Final loop ******************");
        for op_descriptor in results_queue {
            let op_descriptor = op_descriptor.unwrap();
            let buf = &op_descriptor.buf;
            println!("{:?}", std::str::from_utf8(buf).unwrap());
        }
    }
}
