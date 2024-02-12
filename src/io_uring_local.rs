use io_uring::cqueue;
use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use io_uring::IoUring;
use nix::sys::stat::stat;
use std::fs;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvError, TryRecvError};

use crate::operation::{Operation, OperationWithCallback};

pub(crate) fn worker_thread_func(rx: Receiver<OperationWithCallback>) {
    const SQ_RING_SIZE: u32 = 32; // TODO: Allow the user to configure this SQ_RING_SIZE.
    let mut ring: IoUring<squeue::Entry, cqueue::Entry> = io_uring::IoUring::builder()
        .setup_sqpoll(1000) // The kernel sqpoll thread will sleep after this many milliseconds.
        // TODO: Allow the user to decide whether sqpoll is used.
        .build(SQ_RING_SIZE)
        .unwrap();
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;
    let mut n_ops_received_from_user: u32 = 0;
    let mut n_ops_completed: u32 = 0;

    // Performance counters. Not needed for correct operation.
    let mut completion_was_empty = 0;
    let mut cum_tasks_in_flight: u64 = 0;
    let mut outer_loop_iterations = 0;

    'outer: loop {
        // Keep io_uring's submission queue topped up:
        // TODO: Extract this inner loop into a separate function!
        'inner: loop {
            let op_with_callback = match n_tasks_in_flight_in_io_uring {
                0 => match rx.recv() {
                    // There are no tasks in flight in io_uring, so all that's
                    // left to do is to wait for more `Operations` from the user.
                    Ok(s) => s,
                    Err(RecvError) => break 'outer, // The caller hung up.
                },
                n if n < SQ_RING_SIZE => match rx.try_recv() {
                    Ok(s) => s,
                    Err(TryRecvError::Empty) => break 'inner,
                    Err(TryRecvError::Disconnected) => break 'outer,
                },
                _ => break 'inner, // The CQ is full!
            };

            // We need `op_with_callback` to remain in memory after this `loop` because
            // we send a raw pointer to `op_with_callback` through io_uring, so we can
            // access the appropriate `op_with_callback` associated with this io_uring op
            // when the io_uring operation completes.
            let mut op_with_callback = Box::new(op_with_callback);

            // Convert `Operation` to a `squeue::Entry`.
            let sq_entry = op_with_callback
                .get_mut_operation()
                .as_mut()
                .unwrap()
                .to_iouring_entry()
                .user_data(Box::into_raw(op_with_callback) as u64);

            // Submit to io_uring!
            unsafe {
                ring.submission()
                    .push(&sq_entry)
                    .expect("io_uring submission queue full")
            }

            // Increment counter:
            n_tasks_in_flight_in_io_uring += 1;
            n_ops_received_from_user += 1;
        }

        assert_ne!(n_tasks_in_flight_in_io_uring, 0);
        cum_tasks_in_flight += n_tasks_in_flight_in_io_uring as u64;
        outer_loop_iterations += 1;

        if ring.completion().is_empty() {
            ring.submit_and_wait(1).unwrap();
            completion_was_empty += 1;
        } else {
            // `ring.submit()` is basically a no-op if the kernel's sqpoll thread is still awake.
            ring.submit().unwrap();
        }

        for (i, cqe) in ring.completion().enumerate() {
            n_tasks_in_flight_in_io_uring -= 1;
            n_ops_completed += 1;

            // Handle errors reported by io_uring:
            if cqe.result() < 0 {
                let err = nix::Error::from_i32(-cqe.result());
                println!("Error from CQE: {:?}", err);
                // TODO: This error needs to be sent to the user. See issue #45.
                // Something like: `Err(err.into())`
            };

            // Get the associated `OperationWithCallback` and call `execute_callback()`!
            let mut op_with_callback =
                unsafe { Box::from_raw(cqe.user_data() as *mut OperationWithCallback) };
            op_with_callback.execute_callback();

            if i > (SQ_RING_SIZE / 2) as _ {
                // Break, so we keep the SQ topped up.
                // TODO: Ideally, we'd only break here if `rx.try_recv()` has data.
                // But maybe it's fine to just check `rx.try_recv()` at the top of this loop.
                break;
            }
        }
    }
    assert_eq!(n_ops_received_from_user, n_ops_completed);
    println!("completion_was_empty: {completion_was_empty}");
    println!(
        "average tasks in flight: {}",
        cum_tasks_in_flight / (outer_loop_iterations as u64)
    );
}

impl Operation {
    fn to_iouring_entry(&mut self) -> squeue::Entry {
        match *self {
            Operation::Get {
                ref location,
                ref mut buffer,
                ref mut fd,
            } => create_sq_entry_for_get_op(location, buffer, fd),
        }
    }
}

fn get_filesize_bytes(location: &std::path::Path) -> i64 {
    stat(location).expect("Failed to get filesize!").st_size
}

fn create_sq_entry_for_get_op(
    location: &PathBuf,
    buffer: &mut Option<object_store::Result<Vec<u8>>>,
    fd: &mut Option<std::fs::File>,
) -> squeue::Entry {
    // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
    let filesize_bytes = get_filesize_bytes(location);

    // Allocate vector:
    // TODO: Don't initialise to all-zeros. Issue #46.
    // See https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#initializing-an-array-element-by-element
    let _ = *buffer.insert(Ok(vec![0; filesize_bytes as _]));

    // Create squeue::Entry
    // TODO: Open file using io_uring. See issue #1
    *fd = Some(
        fs::OpenOptions::new()
            .read(true)
            // TODO: Use DIRECT mode to open files. And allow the user to choose.
            // I'll worry about DIRECT mode after we open file using io_uring. Issue #1.
            // .custom_flags(libc::O_DIRECT)
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
