use io_uring::cqueue;
use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use io_uring::types::OpenHow;
use io_uring::IoUring;
use nix::sys::stat::stat;
use std::ffi::CString;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
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
        .expect("Failed to initialise io_uring.");

    // Register "fixed" file descriptors, for use in chaining open, read, close:
    // TODO: We only need unique FDs for each file in flight in io_uring. WE should re-use SQ_RING_SIZE FDs!
    // let _ = ring.submitter().unregister_files(); // Cleanup all fixed files (if any)
    // let fds: Vec<RawFd> = (5..10).collect(); // .map(RawFd::from)
    //ring.submitter()
    //    .register_files(&[RawFd::from(3i32)]) // fds.as_slice()) // Huh. 3 or higher doesn't work?!?
    //    .expect("Failed to register files with io_uring!");

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
            let op_with_callback = match n_tasks_in_flight_in_io_uring {
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
            fixed_fd += 1;
        }

        assert_ne!(n_tasks_in_flight_in_io_uring, 0);

        if ring.completion().is_empty() {
            ring.submit_and_wait(1).unwrap();
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
                // TODO: This error needs to be sent to the user and, ideally, associated with a filename.
                // Something like: `Err(err.into())`. See issue #45.
            };

            if cqe.user_data() == 0 {
                // This is an `open` or `close` operation. For now, we ignore these.
                // TODO: Keep track of `open` and `close` operations. See issue #54.
                continue;
            }

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

fn create_sq_entries(
    mut op_with_callback: Box<OperationWithCallback>,
    fixed_fd: u32,
) -> Vec<squeue::Entry> {
    let op = op_with_callback.get_mut_operation().as_ref().unwrap();
    match op {
        Operation::Get { .. } => create_sq_entry_for_get_op(op_with_callback, fixed_fd),
    }
}

fn get_filesize_bytes(location: &std::path::Path) -> i64 {
    stat(location).expect("Failed to get filesize!").st_size
}

fn create_sq_entry_for_get_op(
    mut op_with_callback: Box<OperationWithCallback>,
    fixed_fd: u32,
) -> Vec<squeue::Entry> {
    // TODO: Test for these:
    // - opcode::OpenAt2::CODE
    // - opcode::Close::CODE
    // - opcode::Socket::CODE // to ensure fixed table support

    let (path, buffer) = match op_with_callback.get_mut_operation().as_mut().unwrap() {
        Operation::Get {
            location, buffer, ..
        } => (location, buffer),
    };

    // See this comment for more information on chaining open, read, close in io_uring:
    // https://github.com/JackKelly/light-speed-io/issues/1#issuecomment-1939244204

    // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
    let filesize_bytes = get_filesize_bytes(path);

    // Allocate vector:
    // TODO: Don't initialise to all-zeros. Issue #46.
    // See https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#initializing-an-array-element-by-element
    let _ = *buffer.insert(Ok(vec![0; filesize_bytes as _]));

    let mut entries = Vec::with_capacity(3); // 3 Entries: open, read, close

    // Prepare the "open" opcode:
    // This code block is adapted from:
    // https://github.com/tokio-rs/io-uring/blob/e3fa23ad338af1d051ac82e18688453a9b3d8376/io-uring-test/src/tests/fs.rs#L288-L295
    let path = CString::new(path.as_os_str().as_bytes())
        .expect("Could not convert path '{path}' to CString.");
    // let open_how = OpenHow::new().flags(libc::O_DIRECT as u64); // TODO: I'm worried about this cast to u64!
    let file_index = types::DestinationSlot::try_from_slot_target(fixed_fd)
        .expect("Could not allocate target slot. fixed_fd={fixed_fd}");
    let open_op = opcode::OpenAt::new(
        types::Fd(0), // dirfd is ignored if the pathname is absolute. See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        path.as_ptr(),
        // &open_how,
    )
    .file_index(Some(file_index))
    .flags(libc::O_DIRECT)
    .mode(libc::O_RDONLY as _)
    .build()
    .flags(squeue::Flags::IO_LINK)
    .user_data(0); // TODO: user_data should refer to the Operation. See issue #54.
    entries.push(open_op);

    // Prepare the "read" opcode:
    let read_op = opcode::Read::new(
        types::Fixed(fixed_fd),
        buffer.as_mut().unwrap().as_mut().unwrap().as_mut_ptr(),
        filesize_bytes as _,
    )
    .build()
    .flags(squeue::Flags::IO_LINK)
    .user_data(Box::into_raw(op_with_callback) as _);
    entries.push(read_op);

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(types::Fixed(fixed_fd))
        .build()
        .user_data(0); // TODO: user_data should refer to the Operation. See issue #54.
    entries.push(close_op);

    entries
}
