use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use io_uring::IoUring;
use nix::sys::stat::stat;
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
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
            let mut op_with_callback = match n_tasks_in_flight_in_io_uring {
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

            // Convert `Operation` to a `squeue::Entry`.
            let sq_entry = op_with_callback
                .get_mut_operation()
                .as_mut()
                .unwrap()
                .to_iouring_entry()
                .user_data(123); // TODO: Add user data!

            // Submit to io_uring!
            unsafe {
                ring.submission()
                    .push(&sq_entry)
                    .expect("io_uring submission queue full")
            }

            // Increment counter:
            n_tasks_in_flight_in_io_uring += 1;
        }

        ring.submit_and_wait(1).unwrap(); // TODO: Handle error!

        println!("After ring.submit_and_wait");

        // Spawn tasks to the Rayon ThreadPool to process data:
        for cqe in ring.completion() {
            n_tasks_in_flight_in_io_uring -= 1;
            todo!();
            // TODO:
            // - Handle any errors. See https://github.com/JackKelly/light-speed-io/blob/main/src/io_uring.rs#L115-L120
            // - Get the associated `OperationWithCallback` and call `execute_callback()`!
        }
    }
}

impl Operation {
    fn to_iouring_entry(&mut self) -> squeue::Entry {
        match *self {
            Operation::Get {
                ref location,
                ref mut buffer,
                ref mut fd,
            } => {
                // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
                let location = object_store_path_to_std_path(location);
                let filesize_bytes = stat(location).unwrap().st_size;

                // Allocate vector:
                *buffer = Some(Ok(Vec::with_capacity(filesize_bytes as _)));

                // Create squeue::Entry
                // TODO: Open file using io_uring. See issue #1
                *fd = Some(
                    fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_DIRECT)
                        .open(location)
                        .unwrap(),
                );

                // Note that the developer needs to ensure
                // that the entry pushed into submission queue is valid (e.g. fd, buffer).
                opcode::Read::new(
                    types::Fd(fd.as_ref().unwrap().as_raw_fd()),
                    buffer.as_mut().unwrap().as_mut().unwrap().as_mut_ptr(),
                    filesize_bytes as _,
                )
                .build()
            }
        }
    }
}

fn object_store_path_to_std_path(location: &object_store::path::Path) -> &std::path::Path {
    let location = location.as_ref();
    std::path::Path::new(location)
}
