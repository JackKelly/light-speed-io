use io_uring::cqueue;
use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use io_uring::types::Fixed;
use io_uring::types::OpenHow;
use io_uring::IoUring;
use nix::sys::stat::stat;
use std::fs;
use std::os::fd::AsRawFd;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::{Receiver, RecvError};

use crate::operation::{Operation, OperationWithCallback};

pub(crate) fn worker_thread_func(rx: Receiver<Box<OperationWithCallback>>) {
    const SQ_RING_SIZE: u32 = 32; // TODO: Allow the user to configure SQ_RING_SIZE.
    const MAX_ENTRIES_PER_CHAIN: u32 = 3; // Maximum number of io_uring entries per io_uring chain.
    assert!(MAX_ENTRIES_PER_CHAIN < SQ_RING_SIZE);
    const MAX_ENTRIES_BEFORE_BREAKING_LOOP: u32 = SQ_RING_SIZE - MAX_ENTRIES_PER_CHAIN;
    let mut ring: IoUring<squeue::Entry, cqueue::Entry> = io_uring::IoUring::builder()
        .setup_sqpoll(1000) // The kernel sqpoll thread will sleep after this many milliseconds.
        // TODO: Allow the user to decide whether sqpoll is used.
        .build(SQ_RING_SIZE)
        .unwrap();
    let submitter = ring.submitter();

    // Register "fixed" file descriptors, for use in chaining open, read, close:
    // TODO: We only need unique FDs for each file in flight in io_uring. WE should re-use SQ_RING_SIZE FDs!
    let fds: Vec<RawFd> = (0..1000).map(|fd| RawFd::from(fd)).collect();
    submitter.register_files(&fds).unwrap();

    // Counters
    let mut n_tasks_in_flight_in_io_uring: u32 = 0;
    let mut n_ops_received_from_user: u32 = 0;
    let mut n_ops_completed: u32 = 0;
    let mut rx_might_have_more_data_waiting: bool;
    let mut fixed_fd: u32 = 0;

    'outer: loop {
        // Keep io_uring's submission queue topped up:
        // TODO: Extract this inner loop into a separate function!
        'inner: loop {
            let mut op_with_callback = match n_tasks_in_flight_in_io_uring {
                0 => match rx.recv() {
                    // There are no tasks in flight in io_uring, so all that's
                    // left to do is to wait for more `Operations` from the user.
                    Ok(s) => s,
                    Err(RecvError) => break 'outer, // The caller hung up.
                },
                MAX_ENTRIES_BEFORE_BREAKING_LOOP => {
                    // The SQ is full!
                    rx_might_have_more_data_waiting = true;
                    break 'inner;
                }
                _ => match rx.try_recv() {
                    Ok(s) => s,
                    Err(TryRecvError::Empty) => {
                        rx_might_have_more_data_waiting = false;
                        break 'inner;
                    }
                    Err(TryRecvError::Disconnected) => break 'outer,
                },
            };

            // Convert `Operation` to one or more `squeue::Entry`.
            let sq_entries = create_sq_entries(op_with_callback, fixed_fd);

            // Submit to io_uring!
            for sq_entry in sq_entries {
                unsafe {
                    ring.submission()
                        .push(&sq_entry)
                        .expect("io_uring submission queue full")
                }
                n_tasks_in_flight_in_io_uring += 1;
            }

            // Increment counter:
            n_ops_received_from_user += 1;
            fixed_fd +1;
        }

        assert_ne!(n_tasks_in_flight_in_io_uring, 0);

        if ring.completion().is_empty() {
            submitter.submit_and_wait(1).unwrap();
        } else {
            // `ring.submit()` is basically a no-op if the kernel's sqpoll thread is still awake.
            submitter.submit().unwrap();
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

            if rx_might_have_more_data_waiting && i > (SQ_RING_SIZE / 2) as _ {
                // Break, to keep the SQ topped up.
                break;
            }
        }
    }
    assert_eq!(n_ops_received_from_user, n_ops_completed);
}

fn create_sq_entries(mut op_with_callback: Box<OperationWithCallback>, fixed_fd: u32) -> Vec<squeue::Entry> {
    let op = op_with_callback.get_mut_operation().as_mut().unwrap();

    match op {
        Operation::Get {
            ref location,
            ref mut buffer,
        } => create_sq_entry_for_get_op(location, buffer, fixed_fd),
    }

    // entry.user_data(Box::into_raw(op_with_callback) as u64);
}

fn get_filesize_bytes(location: &std::path::Path) -> i64 {
    stat(location).expect("Failed to get filesize!").st_size
}

fn create_sq_entry_for_get_op(
    location: &PathBuf,
    buffer: &mut Option<object_store::Result<Vec<u8>>>,
    fixed_fd: u32,
) -> Vec<squeue::Entry> {
    // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
    let filesize_bytes = get_filesize_bytes(location);

    // Allocate vector:
    // TODO: Don't initialise to all-zeros. Issue #46.
    // See https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#initializing-an-array-element-by-element
    let _ = *buffer.insert(Ok(vec![0; filesize_bytes as _]));

    let mut entries = Vec::with_capacity(3); // 3 Entries: open, read, close

    // Prepare to open the file.
    // This is a work in progress, and doesn't currently compile! See issue #1.
    let open_how = OpenHow::new().flags(libc::O_DIRECT as u64); // TODO: I'm worried about this cast to u64!
    entries.push(
        opcode::OpenAt2::new(-1 as _, location.as_os_str().as_encoded_bytes().as_ptr() as _, &open_how)
    );

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
        types::Fd(fixed_fd),
        buffer.as_mut().unwrap().as_mut().unwrap().as_mut_ptr(),
        filesize_bytes as _,
    )
    .build()

    entries
}
