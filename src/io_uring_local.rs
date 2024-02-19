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

use crate::operation::{Operation, OperationWithCallback};

struct OpTracker<const N: usize> {
    ops_in_flight: [Option<Box<OperationWithCallback>>; N],
    next_index: VecDeque<usize>,
}

impl<const N: usize> OpTracker<N> {
    fn new() -> Self {
        const ARRAY_REPEAT_VALUE: Option<Box<OperationWithCallback>> = None;
        Self {
            ops_in_flight: [ARRAY_REPEAT_VALUE; N],
            next_index: (0..N).collect(),
        }
    }

    fn get_next_index(&mut self) -> usize {
        self.next_index
            .pop_front()
            .expect("next_index should not be empty!")
    }

    /// Store an OperationWithCallback and return the index into which that
    /// OperationWithCallback has been stored.
    fn put(&mut self, index: usize, op: Box<OperationWithCallback>) {
        let index = self.ops_in_flight[index].replace(op);
    }

    fn get_mut(&mut self, index: usize) -> &mut Box<OperationWithCallback> {
        self.ops_in_flight[index]
            .as_mut()
            .expect("No Operation found at index {index}!")
    }

    fn remove(&mut self, index: usize) -> Box<OperationWithCallback> {
        self.next_index.push_back(index);
        self.ops_in_flight[index]
            .take()
            .expect("No Operation found at index {index}!")
    }
}

pub(crate) fn worker_thread_func(rx: Receiver<Box<OperationWithCallback>>) {
    const SQ_RING_SIZE: usize = 48; // TODO: Allow the user to configure SQ_RING_SIZE.
    const MAX_ENTRIES_PER_CHAIN: usize = 3; // Maximum number of io_uring entries per io_uring chain.
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

    // Counters
    let mut n_tasks_in_flight_in_io_uring: usize = 0;
    let mut n_ops_received_from_user: u32 = 0;
    let mut n_user_ops_completed: u32 = 0;
    let mut rx_might_have_more_data_waiting: bool;
    let mut fixed_fd: u32 = 0;

    // Track ops in flight
    let mut op_tracker = OpTracker::<SQ_RING_SIZE>::new();

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
                MAX_ENTRIES_BEFORE_BREAKING_LOOP.. => {
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

            let index_of_op = op_tracker.get_next_index();

            // Convert `Operation` to one or more `squeue::Entry`, and submit to io_uring.
            let entries = create_sq_entries_for_op(&mut op_with_callback, index_of_op, fixed_fd);
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

            // user_data holds the io_uring opcode in the lower 32 bits,
            // and holds the index_of_op in the upper 32 bits.
            let user_data = cqe.user_data();
            let uring_opcode = (user_data & 0xFFFFFFFF) as u8;
            let index_of_op = (user_data >> 32) as usize;

            // Handle errors reported by io_uring:
            if cqe.result() < 0 {
                let err = nix::Error::from_i32(-cqe.result());
                println!(
                    "Error from CQE: {err:?}. opcode={uring_opcode}; index_of_op={index_of_op}"
                );
                // TODO: This error needs to be sent to the user and, ideally, associated with a filename.
                // Something like: `Err(err.into())`. See issue #45.
            }

            if uring_opcode == opcode::OpenAt::CODE || uring_opcode == opcode::Close::CODE {
                // This is an `open` or `close` operation. For now, we ignore these.
                // TODO: Keep track of `open` and `close` operations. See issue #54.
                continue;
            }

            n_user_ops_completed += 1;

            // Get the associated `OperationWithCallback` and call `execute_callback()`!
            let mut op_with_callback = op_tracker.remove(index_of_op);
            op_with_callback.execute_callback();

            if rx_might_have_more_data_waiting && i > (SQ_RING_SIZE / 2) as _ {
                // Break, to keep the SQ topped up.
                break;
            }
        }
    }
    assert_eq!(n_ops_received_from_user, n_user_ops_completed);
}

fn create_sq_entries_for_op(
    op_with_callback: &mut OperationWithCallback,
    index_of_op: usize,
    fixed_fd: u32,
) -> [squeue::Entry; 3] {
    let op = op_with_callback.get_mut_operation().as_ref().unwrap();
    match op {
        Operation::Get { .. } => {
            create_sq_entry_for_get_op(op_with_callback, index_of_op, fixed_fd)
        }
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
    fixed_fd: u32,
) -> [squeue::Entry; 3] {
    // TODO: Test for these:
    // - opcode::OpenAt2::CODE
    // - opcode::Close::CODE
    // - opcode::Socket::CODE // to ensure fixed table support

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
    let _ = *buffer.insert(Ok(vec![0; filesize_bytes as _]));

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
        buffer.as_mut().unwrap().as_mut().unwrap().as_mut_ptr(),
        filesize_bytes as _,
    )
    .build()
    .flags(squeue::Flags::IO_LINK)
    .user_data(index_of_op | (opcode::Read::CODE as u64));

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(types::Fixed(fixed_fd))
        .build()
        .user_data(index_of_op | (opcode::Close::CODE as u64));

    [open_op, read_op, close_op]
}
