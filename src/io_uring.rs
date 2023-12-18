use anyhow::Result;
use io_uring::squeue::PushError;
use io_uring::{opcode, types, IoUring};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::{fs, io};

fn submit_and_process(tasks: &[PathBuf]) -> Vec<Result<Vec<u8>>> {
    const CQ_RING_SIZE: u32 = 16;
    let mut ring = IoUring::builder().build(CQ_RING_SIZE).unwrap();
    let n_tasks_in_flight = Arc::new(AtomicU32::new(0));
    let pool = rayon::ThreadPoolBuilder::new().build().unwrap();
    let mut results: Vec<Result<Vec<u8>>> = Vec::with_capacity(tasks.len());

    // Send tasks to threadpool:
    let mut task_i = 0;
    while task_i < tasks.len() && n_tasks_in_flight.load(Ordering::SeqCst) > 0 {
        // Keep io_uring submission queue topped up:
        while n_tasks_in_flight.load(Ordering::SeqCst) < CQ_RING_SIZE {
            let task = &tasks[task_i];
            let mut buf = vec![0u8; 1024];

            let fd = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECT)
                .open(task)
                .unwrap(); // TODO: Handle error!

            submit_read(
                &mut ring,
                &fd,
                buf.as_mut_ptr(),
                buf.len() as _,
                task_i as u64,
            )
            .expect("submission queue full"); // TODO: Handle PushError:

            results[task_i] = Ok(buf);
            task_i += 1;
            n_tasks_in_flight.fetch_add(1, Ordering::SeqCst);
        }

        ring.submit_and_wait(1).unwrap(); // TODO: Handle error!

        // Spawn tasks to the Rayon ThreadPool to process data:
        for cqe in ring.completion() {
            // Prepare data for thread:
            // Handle return value from read():
            let return_value = cqe.result();
            if return_value == -1 {
                // An error has occurred!
                results[cqe.user_data() as usize] = Err(anyhow::Error::new(io::Error::new(
                    ErrorKind::Other,
                    "io_uring reported an error",
                )));
                n_tasks_in_flight.fetch_sub(1, Ordering::SeqCst);
            } else {
                let n_tasks_in_flight_for_thread = n_tasks_in_flight.clone();
                pool.spawn(move || {
                    // TODO: Handle when the number of bytes read is less than the number of bytes requested
                    // TODO: process(cqe);
                    n_tasks_in_flight_for_thread.fetch_sub(1, Ordering::SeqCst);
                });
            }
        }
    }
    // TODO: Need to wait for threads to finish! There's no pool.wait(). Maybe use Rayon's scope?
    results
}

/// Note that the developer needs to ensure
/// that the entry pushed into submission queue is valid (e.g. fd, buffer).
fn submit_read(
    ring: &mut IoUring,
    fd: &fs::File,
    buf: *mut u8,
    len: u32,
    user_data: u64,
) -> Result<(), PushError> {
    let read_e = opcode::Read::new(types::Fd(fd.as_raw_fd()), buf, len)
        .build()
        .user_data(user_data);

    unsafe { ring.submission().push(&read_e) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = io_uring();
        assert!(result.is_ok(), "Error: {}", result.unwrap_err());
    }
}
