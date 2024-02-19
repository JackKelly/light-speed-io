use io_uring::cqueue;
use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use io_uring::IoUring;
use nix::sys::stat::stat;
use nix::NixPath;
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::{Receiver, RecvError};

use crate::{operation::Operation, operation::OperationWithCallback, tracker::Tracker};

pub(crate) fn worker_thread_func(rx: Receiver<OperationWithCallback>) {
    const MAX_FILES_IN_FLIGHT: usize = 14;
    const MAX_ENTRIES_PER_CHAIN: usize = 3; // Maximum number of io_uring entries per io_uring chain.
    const SQ_RING_SIZE: usize = MAX_FILES_IN_FLIGHT * MAX_ENTRIES_PER_CHAIN; // TODO: Allow the user to configure SQ_RING_SIZE.
    assert!(MAX_ENTRIES_PER_CHAIN < SQ_RING_SIZE);
    const MAX_ENTRIES_BEFORE_BREAKING_LOOP: usize = SQ_RING_SIZE - MAX_ENTRIES_PER_CHAIN;
    let mut ring: IoUring<squeue::Entry, cqueue::Entry> = io_uring::IoUring::builder()
        .setup_sqpoll(1000) // The kernel sqpoll thread will sleep after this many milliseconds.
        // TODO: Allow the user to decide whether sqpoll is used.
        .build(SQ_RING_SIZE as _)
        .expect("Failed to initialise io_uring.");

    // Register "fixed" file descriptors, for use in chaining open, read, close:
    // TODO: Only register enough FDs for the files in flight in io_uring. We should re-use SQ_RING_SIZE FDs! See issue #54.
    let _ = ring.submitter().unregister_files(); // Cleanup all fixed files (if any)
    ring.submitter()
        .register_files_sparse(16) // TODO: Only register enough direct FDs for the ops in flight!
        .expect("Failed to register files!");
    // io_uring supports a max of 16 registered ring descriptors. See:
    // https://manpages.debian.org/unstable/liburing-dev/io_uring_register.2.en.html#IORING_REGISTER_RING_FDS
    let mut next_file_descriptor: VecDeque<u32> = (0..MAX_FILES_IN_FLIGHT as u32).collect();
    let mut fds_to_unregister = Vec::new();

    // Counters
    let mut n_tasks_in_flight_in_io_uring: usize = 0;
    let mut n_ops_received_from_user: u32 = 0;
    let mut n_user_ops_completed: u32 = 0;
    let mut rx_might_have_more_data_waiting: bool;

    // Track ops in flight
    let mut op_tracker = Tracker::new(SQ_RING_SIZE);

    'outer: loop {
        // Keep io_uring's submission queue topped up:
        // TODO: Extract this inner loop into a separate function!
        'inner: loop {
            let (mut op_with_callback, fd) = match (
                n_tasks_in_flight_in_io_uring,
                next_file_descriptor.pop_front(),
            ) {
                (0, Some(fd)) => match rx.recv() {
                    // There are no tasks in flight in io_uring, so all that's
                    // left to do is to wait for more `Operations` from the user.
                    Ok(s) => (s, fd),
                    Err(RecvError) => break 'outer, // The caller hung up.
                },
                (MAX_ENTRIES_BEFORE_BREAKING_LOOP.., _) => {
                    // The SQ is full!
                    rx_might_have_more_data_waiting = true;
                    break 'inner;
                }
                (_, Some(fd)) => match rx.try_recv() {
                    Ok(s) => (s, fd),
                    Err(TryRecvError::Empty) => {
                        rx_might_have_more_data_waiting = false;
                        break 'inner;
                    }
                    Err(TryRecvError::Disconnected) => break 'outer,
                },
                (_, None) => {
                    // We can't handle any more files right now!
                    rx_might_have_more_data_waiting = true;
                    break 'inner;
                }
            };

            let index_of_op = op_tracker.get_next_index().unwrap();
            println!("Using {fd}");
            op_with_callback.fd = Some(fd);

            // Convert `Operation` to one or more `squeue::Entry`, and submit to io_uring.
            let entries = create_sq_entries_for_op(&mut op_with_callback, index_of_op);
            op_tracker.put(index_of_op, op_with_callback);
            for entry in entries {
                unsafe {
                    ring.submission().push(&entry).unwrap_or_else(|err| {
                        panic!("submission queue is full {err} {n_tasks_in_flight_in_io_uring}")
                    });
                }
                n_tasks_in_flight_in_io_uring += 1;
            }

            // Increment counter:
            n_ops_received_from_user += 1;
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

            // user_data holds the io_uring opcode in the lower 32 bits,
            // and holds the index_of_op in the upper 32 bits.
            let user_data = cqe.user_data();
            let uring_opcode = (user_data & 0xFFFFFFFF) as u8;
            let index_of_op = (user_data >> 32) as usize;

            // Handle errors reported by io_uring:
            if cqe.result() < 0 {
                let err = nix::Error::from_i32(-cqe.result());
                println!(
                    "Error from CQE: {err:?}. opcode={uring_opcode}; index_of_op={index_of_op}; fd={:?}",
                    op_tracker.as_ref(index_of_op).unwrap().fd,
                );
                // TODO: This error needs to be sent to the user and, ideally, associated with a filename.
                // Something like: `Err(err.into())`. See issue #45.
            }

            match uring_opcode {
                opcode::OpenAt::CODE => (), // Ignore OpenAt for now.
                opcode::Read::CODE => {
                    //let op_with_callback = op_tracker.as_mut(index_of_op).unwrap();
                    //op_with_callback.execute_callback();
                }
                opcode::Close::CODE => {
                    let mut op_with_callback = op_tracker.remove(index_of_op).unwrap();
                    let fd = op_with_callback.fd.unwrap();
                    next_file_descriptor.push_back(fd);
                    fds_to_unregister.push(fd);
                    op_with_callback.execute_callback();
                    n_user_ops_completed += 1;
                }
                _ => panic!("Unrecognised opcode!"),
            };

            // if rx_might_have_more_data_waiting && i > (SQ_RING_SIZE / 2) as _ {
            //     // Break, to keep the SQ topped up.
            //     break;
            // }
        }

        for fd in &fds_to_unregister {
            println!("Unregistering {}", *fd);
            let n_updates = ring.submitter().register_files_update(*fd, &[-1]).unwrap();
            assert_eq!(n_updates, 1);
        }
        ring.submit().unwrap();
        fds_to_unregister.clear();
    }
    assert_eq!(n_ops_received_from_user, n_user_ops_completed);
}

fn create_sq_entries_for_op(
    op_with_callback: &mut OperationWithCallback,
    index_of_op: usize,
) -> [squeue::Entry; 3] {
    let op = op_with_callback.get_mut_operation().as_ref().unwrap();
    match op {
        Operation::Get { .. } => create_sq_entry_for_get_op(op_with_callback, index_of_op),
    }
}

fn get_filesize_bytes<P>(path: &P) -> i64
where
    P: ?Sized + NixPath,
{
    stat(path).expect("Failed to get filesize!").st_size
}

fn create_sq_entry_for_get_op(
    op_with_callback: &mut OperationWithCallback,
    index_of_op: usize,
) -> [squeue::Entry; 3] {
    // TODO: Test for these:
    // - opcode::OpenAt2::CODE
    // - opcode::Close::CODE
    // - opcode::Socket::CODE // to ensure fixed table support

    let fixed_fd = op_with_callback.fd.unwrap();
    let (path, buffer) = match op_with_callback.get_mut_operation().as_mut().unwrap() {
        Operation::Get {
            path: location,
            buffer,
            ..
        } => (location, buffer),
    };

    // See this comment for more information on chaining open, read, close in io_uring:
    // https://github.com/JackKelly/light-speed-io/issues/1#issuecomment-1939244204

    // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
    let filesize_bytes = get_filesize_bytes(path.as_c_str());

    // Allocate vector:
    // TODO: Don't initialise to all-zeros. Issue #46.
    // See https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#initializing-an-array-element-by-element
    let mut buf = Vec::with_capacity(filesize_bytes as _);

    // Convert the index_of_op into a u64, and bit-shift it left.
    // We do this so the u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
    // and represents the io_uring opcode CODE in the right-most 32 bits.
    let index_of_op: u64 = (index_of_op as u64) << 32;

    // Prepare the "open" opcode:
    // This code block is adapted from:
    // https://github.com/tokio-rs/io-uring/blob/e3fa23ad338af1d051ac82e18688453a9b3d8376/io-uring-test/src/tests/fs.rs#L288-L295
    let path_ptr = path.as_ptr();
    let file_index = types::DestinationSlot::try_from_slot_target(fixed_fd)
        .expect("Could not allocate target slot. fixed_fd={fixed_fd}");
    dbg!(file_index);
    let open_op = opcode::OpenAt::new(
        types::Fd(-1), // dirfd is ignored if the pathname is absolute. See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        path_ptr,
    )
    .file_index(Some(file_index))
    .flags(libc::O_RDONLY) // | libc::O_DIRECT) // TODO: Re-enable O_DIRECT.
    .build()
    .flags(squeue::Flags::IO_LINK)
    .user_data(index_of_op | (opcode::OpenAt::CODE as u64));

    // Prepare the "read" opcode:
    let read_op = opcode::Read::new(
        types::Fixed(fixed_fd),
        buf.as_mut_ptr(),
        filesize_bytes as _,
    )
    .build()
    .flags(squeue::Flags::IO_LINK)
    .user_data(index_of_op | (opcode::Read::CODE as u64));

    unsafe {
        buf.set_len(filesize_bytes as _);
    }
    let _ = *buffer.insert(Ok(buf));

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(types::Fixed(fixed_fd))
        .build()
        .user_data(index_of_op | (opcode::Close::CODE as u64));

    [open_op, read_op, close_op]
}
