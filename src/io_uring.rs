use anyhow::Result;
use io_uring::squeue::PushError;
use io_uring::{opcode, types, IoUring};
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

struct OperationDescriptor {
    buf: Vec<u8>,
    task_i: usize,
    path: PathBuf,
    cqe: Option<io_uring::cqueue::Entry>,

    // Keeping the file descriptor in this struct is just a quick hack to ensure that
    // we keep the file descriptor open until io_uring has finished with this task.
    // TODO: Remove the file descriptor from this struct once we let io_uring open files!
    fd: fs::File,
}

fn submit_and_process(tasks: &[PathBuf]) {
    // TODO: Return type should be -> Vec<Result<Vec<u8>>>
    const CQ_RING_SIZE: u32 = 16;
    let mut ring = IoUring::new(CQ_RING_SIZE).unwrap();
    let n_tasks_in_flight = Arc::new(AtomicU32::new(0));

    // Send tasks to threadpool:
    let mut task_i = 0;
    rayon::scope(|s| {
        while task_i < tasks.len() {
            // Keep io_uring submission queue topped up. But don't overload io_uring!
            while task_i < tasks.len() && n_tasks_in_flight.load(Ordering::SeqCst) < CQ_RING_SIZE {
                let task = &tasks[task_i];
                println!("task_i={}, path={:?}", task_i, task);

                // TODO: Open file using io_uring. See issue #1
                let fd = fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_DIRECT)
                    .open(task)
                    .unwrap();

                // Save information (so the processing thread can access this information).
                // `into_raw()` consumes the object (but doesn't de-allocated it), which is exactly
                // what we want. We mustn't touch buffer until it re-emerges from the kernel.
                // And we do want Rayon's worker thread (that processes the CQE) to decide whether
                // to drop the buffer (after moving data elsewhere) or keep the buffer
                // (if we're passing the buffer back to the user).
                let mut op_descriptor = Box::new(OperationDescriptor {
                    // TODO: Allocate the correct sized buffer for the task.
                    //       Or don't allocate at all, if the user has already allocated.
                    buf: vec![0u8; 1024],
                    path: task.clone(),
                    task_i,
                    cqe: None,
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

                // Increment counters
                task_i += 1;
                n_tasks_in_flight.fetch_add(1, Ordering::SeqCst);
            }

            ring.submit_and_wait(1).unwrap(); // TODO: Handle error!

            // Spawn tasks to the Rayon ThreadPool to process data:
            for cqe in ring.completion() {
                // Prepare data for thread:
                let mut op_descriptor =
                    unsafe { Box::from_raw(cqe.user_data() as *mut OperationDescriptor) };
                op_descriptor.cqe = Some(cqe);
                let n_tasks_in_flight_for_thread = n_tasks_in_flight.clone();

                // Spawn task to Rayon's ThreadPool:
                s.spawn(move |_| {
                    do_something(op_descriptor);
                    n_tasks_in_flight_for_thread.fetch_sub(1, Ordering::SeqCst);
                });
            }
        }
    });
    // TODO: Figure out how to return vectors (or errors)!
    assert!(n_tasks_in_flight.load(Ordering::SeqCst) == 0);
}

fn do_something(op_descriptor: Box<OperationDescriptor>) {
    println!("Reading {:?}", op_descriptor.path);
    let cqe = op_descriptor.cqe.unwrap();

    // Handle return value from read():
    if cqe.result() < 0 {
        // An error has occurred!
        let err = nix::Error::from_i32(-cqe.result());
        println!(
            "Error reading file! Return value = {}. Error = {}",
            cqe.result(),
            err
        );
        // TODO: return;
    }
    // TODO: Handle when the number of bytes read is less than the number of bytes requested

    let buf = op_descriptor.buf;
    println!("{:?}", buf);
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn it_works() {
        let tasks = [PathBuf::from_str("/home/jack/dev/rust/light-speed-io/README.md").unwrap()];
        submit_and_process(&tasks);
    }
}
