use io_uring::cqueue;
use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use io_uring::IoUring;
use nix::sys::stat::stat;
use nix::NixPath;
use std::collections::VecDeque;
use std::ffi::CString;
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::{Receiver, RecvError};

use crate::operation::OperationWithChannel;
use crate::{operation::Operation, tracker::Tracker};

type VecEntries = Vec<squeue::Entry>;

pub(crate) fn worker_thread_func(rx: Receiver<OperationWithChannel>) {
    const MAX_FILES_TO_REGISTER: usize = 16;
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
        .register_files_sparse(MAX_FILES_TO_REGISTER as _)
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
    let mut internal_op_queue: VecDeque<VecEntries> = VecDeque::with_capacity(SQ_RING_SIZE);

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
            let op_with_chan = if n_sqes_in_flight_in_io_uring == 0 {
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
            let entries = match &op_with_chan.operation {
                Operation::Get { path, .. } => create_openat_sqe(path, index_of_op),
            };
            user_tasks_in_flight.put(index_of_op, op_with_chan);
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
            let op_with_chan = user_tasks_in_flight.as_mut(index_of_op).unwrap();

            // Handle errors reported by io_uring:
            if cqe.result() < 0 {
                let err = nix::Error::from_i32(-cqe.result());
                println!(
                    "Error from CQE: {err:?}. opcode={uring_opcode}; index_of_op={index_of_op}; op={op_with_chan:?}"
                );
                op_with_chan.send_error(err.into());
            }

            let error_has_occurred = op_with_chan.error_has_occurred();

            match &mut op_with_chan.operation {
                Operation::Get {
                    path,
                    buffer,
                    fixed_fd,
                    ..
                } => 'get: {
                    match uring_opcode {
                        opcode::OpenAt::CODE => {
                            if error_has_occurred {
                                break 'get;
                            };
                            *fixed_fd = Some(types::Fixed(cqe.result() as u32));
                            let entries = create_linked_read_close_sqes(
                                path,
                                buffer,
                                fixed_fd.as_ref().unwrap(),
                                index_of_op,
                            );
                            internal_op_queue.push_back(entries);
                        }
                        opcode::Read::CODE => {
                            if error_has_occurred {
                                break 'get;
                            };
                            op_with_chan.send_result();
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
        }
    }
    assert_eq!(n_ops_received_from_user, n_user_tasks_completed);
}

fn get_filesize_bytes<P>(path: &P) -> i64
where
    P: ?Sized + NixPath,
{
    stat(path).expect("Failed to get filesize!").st_size
}

fn create_openat_sqe(path: &CString, index_of_op: usize) -> VecEntries {
    // TODO: Test for these:
    // - opcode::OpenAt2::CODE
    // - opcode::Close::CODE
    // - opcode::Socket::CODE // to ensure fixed table support

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

fn create_linked_read_close_sqes(
    path: &CString,
    buffer: &mut Option<anyhow::Result<Vec<u8>>>,
    fixed_fd: &types::Fixed,
    index_of_op: usize,
) -> VecEntries {
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
    let read_op = opcode::Read::new(*fixed_fd, buf.as_mut_ptr(), filesize_bytes as u32)
        .build()
        .user_data(index_of_op | (opcode::Read::CODE as u64))
        .flags(squeue::Flags::IO_LINK);

    unsafe {
        buf.set_len(filesize_bytes as _);
    }
    let _ = *buffer.insert(Ok(buf));

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(*fixed_fd)
        .build()
        .user_data(index_of_op | (opcode::Close::CODE as u64));

    vec![read_op, close_op]
}
