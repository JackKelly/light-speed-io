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
    const MAX_FILES_TO_REGISTER: usize = 14;
    const MAX_ENTRIES_PER_CHAIN: usize = 3; // Maximum number of io_uring entries per io_uring chain.
    const SQ_RING_SIZE: usize = MAX_FILES_TO_REGISTER * MAX_ENTRIES_PER_CHAIN; // TODO: Allow the user to configure SQ_RING_SIZE.
    assert!(MAX_ENTRIES_PER_CHAIN < SQ_RING_SIZE);
    let mut ring: IoUring<squeue::Entry, cqueue::Entry> = io_uring::IoUring::builder()
        .setup_sqpoll(1000) // The kernel sqpoll thread will sleep after this many milliseconds.
        // TODO: Allow the user to decide whether sqpoll is used.
        .build(SQ_RING_SIZE as _)
        .expect("Failed to initialise io_uring.");

    // Register "fixed" file descriptors, for use in chaining SQ entries:
    ring.submitter()
        .register_files_sparse(16)
        .expect("Failed to register files!");

    // io_uring supports a max of 16 registered ring descriptors. See:
    // https://manpages.debian.org/unstable/liburing-dev/io_uring_register.2.en.html#IORING_REGISTER_RING_FDS

    // Counters
    let mut n_sqes_in_flight_in_io_uring: usize = 0;
    let mut n_user_tasks_completed: u32 = 0;
    let mut n_files_registered: usize = 0;
    let mut n_ops_received_from_user: u32 = 0;

    // These are the `squeue::Entry`s generated within this thread.
    // Each inner `Vec<Entry>` will be submitted in one go. Each chain of linked entries
    // must be in its own inner `Vec<Entry>`.
    let mut internal_op_queue: VecDeque<Vec<squeue::Entry>> = VecDeque::with_capacity(SQ_RING_SIZE);

    // These are the tasks that the user submits via `rx`.
    let mut user_tasks_in_flight = Tracker::new(SQ_RING_SIZE);

    'outer: loop {
        // Keep io_uring's submission queue topped up from this thread's internal queue.
        // The internal queue always takes precedence over tasks from the user.
        // TODO: Extract this inner loop into a separate function!
        'inner: while n_sqes_in_flight_in_io_uring < SQ_RING_SIZE {
            match internal_op_queue.pop_front() {
                None => break 'inner,
                Some(entries) => {
                    if (entries.len() + n_sqes_in_flight_in_io_uring) > SQ_RING_SIZE {
                        internal_op_queue.push_front(entries);
                        break 'inner;
                    }

                    unsafe {
                        ring.submission()
                            .push_multiple(entries.as_slice())
                            .unwrap_or_else(|err| {
                                panic!(
                                    "submission queue is full {err} {n_sqes_in_flight_in_io_uring}"
                                )
                            });
                    }
                    n_sqes_in_flight_in_io_uring += entries.len();
                }
            }
        }
        ring.submission().sync();

        // Keep io_uring's submission queue topped up with tasks from the user:
        // TODO: Extract this inner loop into a separate function!
        'inner: while n_files_registered < MAX_FILES_TO_REGISTER
            && n_sqes_in_flight_in_io_uring < (SQ_RING_SIZE - MAX_ENTRIES_PER_CHAIN)
        {
            let mut op_with_callback = if n_sqes_in_flight_in_io_uring == 0 {
                // There are no tasks in flight in io_uring, so all that's
                // left to do is to wait for more `Operations` from the user.
                match rx.recv() {
                    Ok(s) => s,
                    Err(RecvError) => break 'outer, // The caller hung up.
                }
            } else {
                match rx.try_recv() {
                    Ok(s) => s,
                    Err(TryRecvError::Empty) => {
                        break 'inner;
                    }
                    Err(TryRecvError::Disconnected) => break 'outer,
                }
            };

            // Convert `Operation` to one or more `squeue::Entry`, and submit to io_uring.
            let index_of_op = user_tasks_in_flight.get_next_index().unwrap();
            let entries = create_sq_entries_for_op(&mut op_with_callback, index_of_op);
            user_tasks_in_flight.put(index_of_op, op_with_callback);
            unsafe {
                ring.submission()
                    .push_multiple(entries.as_slice())
                    .unwrap_or_else(|err| {
                        panic!("submission queue is full {err} {n_sqes_in_flight_in_io_uring}")
                    });
            }
            n_sqes_in_flight_in_io_uring += entries.len();
            n_ops_received_from_user += 1;
            n_files_registered += 1; // TODO: When we support more `Operations` than just `get`,
                                     // we'll need a way to only increment this when appropriate.
        }

        assert_ne!(n_sqes_in_flight_in_io_uring, 0);

        if ring.completion().is_empty() {
            ring.submit_and_wait(1).unwrap();
        } else {
            // `ring.submit()` is basically a no-op if the kernel's sqpoll thread is still awake.
            ring.submit().unwrap();
        }

        for cqe in ring.completion() {
            n_sqes_in_flight_in_io_uring -= 1;

            // user_data holds the io_uring opcode in the lower 32 bits,
            // and holds the index_of_op in the upper 32 bits.
            let uring_opcode = (cqe.user_data() & 0xFFFFFFFF) as u8;
            let index_of_op = (cqe.user_data() >> 32) as usize;

            // Handle errors reported by io_uring:
            if cqe.result() < 0 {
                let err = nix::Error::from_i32(-cqe.result());
                println!(
                    "Error from CQE: {err:?}. opcode={uring_opcode}; index_of_op={index_of_op}; fd={:?}",
                    user_tasks_in_flight.as_ref(index_of_op).unwrap().fixed_fd,
                );
                // TODO: This error needs to be sent to the user and, ideally, associated with a filename.
                // Something like: `Err(err.into())`. See issue #45.
            }

            match uring_opcode {
                opcode::OpenAt::CODE => {
                    let op_with_callback = user_tasks_in_flight.as_mut(index_of_op).unwrap();
                    op_with_callback.fixed_fd = Some(types::Fixed(cqe.result() as u32));
                    let entries =
                        create_sq_entries_for_read_and_close(op_with_callback, index_of_op);
                    internal_op_queue.push_back(entries);
                }
                opcode::Read::CODE => {
                    let op_with_callback = user_tasks_in_flight.as_mut(index_of_op).unwrap();
                    op_with_callback.execute_callback();
                }
                opcode::Close::CODE => {
                    user_tasks_in_flight.remove(index_of_op).unwrap();
                    n_user_tasks_completed += 1;
                    n_files_registered -= 1;
                }
                _ => panic!("Unrecognised opcode!"),
            };
        }
    }
    assert_eq!(n_ops_received_from_user, n_user_tasks_completed);
}

fn create_sq_entries_for_op(
    op_with_callback: &mut OperationWithCallback,
    index_of_op: usize,
) -> Vec<squeue::Entry> {
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
    op_with_callback: &OperationWithCallback,
    index_of_op: usize,
) -> Vec<squeue::Entry> {
    // TODO: Test for these:
    // - opcode::OpenAt2::CODE
    // - opcode::Close::CODE
    // - opcode::Socket::CODE // to ensure fixed table support

    let path = match op_with_callback.get_operation().as_ref().unwrap() {
        Operation::Get { path: location, .. } => location,
    };

    // Convert the index_of_op into a u64, and bit-shift it left.
    // We do this so the u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
    // and represents the io_uring opcode CODE in the right-most 32 bits.
    let index_of_op: u64 = (index_of_op as u64) << 32;

    // Prepare the "open" opcode:
    // This code block is adapted from:
    // https://github.com/tokio-rs/io-uring/blob/e3fa23ad338af1d051ac82e18688453a9b3d8376/io-uring-test/src/tests/fs.rs#L288-L295
    let path_ptr = path.as_ptr();
    let file_index = types::DestinationSlot::auto_target();
    let open_op = opcode::OpenAt::new(
        types::Fd(-1), // dirfd is ignored if the pathname is absolute. See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        path_ptr,
    )
    .file_index(Some(file_index))
    .flags(libc::O_RDONLY) // | libc::O_DIRECT) // TODO: Re-enable O_DIRECT.
    .build()
    .user_data(index_of_op | (opcode::OpenAt::CODE as u64));

    vec![open_op]
}

fn create_sq_entries_for_read_and_close(
    op_with_callback: &mut OperationWithCallback,
    index_of_op: usize,
) -> Vec<squeue::Entry> {
    let fixed_fd = op_with_callback.fixed_fd.unwrap();
    let (path, buffer) = match op_with_callback.get_mut_operation().as_mut().unwrap() {
        Operation::Get {
            path: location,
            buffer,
            ..
        } => (location, buffer),
    };

    // Convert the index_of_op into a u64, and bit-shift it left.
    // We do this so the u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
    // and represents the io_uring opcode CODE in the right-most 32 bits.
    let index_of_op: u64 = (index_of_op as u64) << 32;

    // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
    let filesize_bytes = get_filesize_bytes(path.as_c_str());

    // Allocate vector:
    // TODO: Don't initialise to all-zeros. Issue #46.
    // See https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#initializing-an-array-element-by-element
    let mut buf = Vec::with_capacity(filesize_bytes as _);

    // Prepare the "read" opcode:
    let read_op = opcode::Read::new(fixed_fd, buf.as_mut_ptr(), filesize_bytes as u32)
        .build()
        .user_data(index_of_op | (opcode::Read::CODE as u64))
        .flags(squeue::Flags::IO_LINK);

    unsafe {
        buf.set_len(filesize_bytes as _);
    }
    let _ = *buffer.insert(Ok(buf));

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(fixed_fd)
        .build()
        .user_data(index_of_op | (opcode::Close::CODE as u64));

    vec![read_op, close_op]
}
