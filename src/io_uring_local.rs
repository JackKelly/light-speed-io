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

use crate::{operation, operation::Operation, tracker::Tracker};

type VecEntries = Vec<squeue::Entry>;

const MAX_FILES_TO_REGISTER: usize = 16;
const MAX_ENTRIES_PER_CHAIN: usize = 2; // Maximum number of io_uring entries per io_uring chain.
const SQ_RING_SIZE: usize = MAX_FILES_TO_REGISTER * MAX_ENTRIES_PER_CHAIN; // TODO: Allow the user to configure SQ_RING_SIZE.

pub struct IoUringLocal {
    ring: IoUring,
    n_files_registered: usize,

    // These are the `squeue::Entry`s generated within this thread.
    // Each inner `Vec<Entry>` will be submitted in one go. Each chain of linked entries
    // must be in its own inner `Vec<Entry>`.
    internal_op_queue: VecDeque<VecEntries>,

    // These are the tasks that the user submits via `rx`.
    user_tasks_in_flight: Tracker<Box<dyn Operation + Send>>,
}

impl IoUringLocal {
    pub fn new() -> Self {
        assert!(MAX_ENTRIES_PER_CHAIN < SQ_RING_SIZE);

        let ring: IoUring<squeue::Entry, cqueue::Entry> = io_uring::IoUring::builder()
            .setup_sqpoll(1000) // The kernel sqpoll thread will sleep after this many milliseconds.
            // TODO: Allow the user to decide whether sqpoll is used.
            .build(SQ_RING_SIZE as _)
            .expect("Failed to initialise io_uring.");

        // Check that io_uring is set up to apply back-pressure to the submission queue, to stop the
        // completion queue overflowing. See issue #66.
        assert!(ring.params().is_feature_nodrop());
        assert_eq!(ring.params().cq_entries(), ring.params().sq_entries() * 2);

        // Register "fixed" file descriptors, for use in chaining SQ entries:
        ring.submitter()
            .register_files_sparse(MAX_FILES_TO_REGISTER as _)
            .expect("Failed to register files!");

        // io_uring supports a max of 16 registered ring descriptors. See:
        // https://manpages.debian.org/unstable/liburing-dev/io_uring_register.2.en.html#IORING_REGISTER_RING_FDS

        Self {
            ring,
            n_files_registered: 0,
            internal_op_queue: VecDeque::with_capacity(SQ_RING_SIZE),
            user_tasks_in_flight: Tracker::new(SQ_RING_SIZE),
        }
    }

    pub(crate) fn worker_thread_func(&mut self, mut rx: Receiver<Box<dyn Operation + Send>>) {
        // This is the main loop for the thread.
        loop {
            // The internal queue always takes precedence over the injector queue.
            self.move_entries_from_internal_queue_to_uring_sq();

            // If there's space in io_uring's SQ, then add SQEs from the injector queue:
            if self
                .move_entries_from_injector_queue_to_uring_sq(&mut rx)
                .is_err()
            {
                break;
            }

            self.submit_and_maybe_wait();
            self.process_uring_cq();
        }
        assert!(self.user_tasks_in_flight.is_empty());
    }

    /// Keep io_uring's submission queue topped up from this thread's internal queue.
    /// The internal queue always takes precedence over tasks from the user.
    fn move_entries_from_internal_queue_to_uring_sq(&mut self) {
        while !self.uring_is_full() {
            match self.internal_op_queue.pop_front() {
                None => break,
                Some(entries) => {
                    unsafe {
                        self.ring
                            .submission()
                            .push_multiple(entries.as_slice())
                            .unwrap()
                    };
                }
            }
        }
    }

    /// Keep io_uring's submission queue topped up with tasks from the user.
    fn move_entries_from_injector_queue_to_uring_sq(
        &mut self,
        rx: &mut Receiver<Box<dyn Operation + Send>>,
    ) -> Result<(), RecvError> {
        // TODO: The `n_files_registered < MAX_FILES_TO_REGISTER` check is only appropriate while
        // Operations are only ever `get` Operations.
        while !self.uring_is_full() && self.n_files_registered < MAX_FILES_TO_REGISTER {
            let op = if self.user_tasks_in_flight.is_empty() {
                // There are no tasks in flight in io_uring, so all that's
                // left to do is to block and wait for more `Operations` from the user.
                match rx.recv() {
                    Ok(s) => s,
                    Err(RecvError) => return Err(RecvError), // The caller hung up.
                }
            } else {
                match rx.try_recv() {
                    Ok(s) => s,
                    Err(TryRecvError::Empty) => return Ok(()),
                    Err(TryRecvError::Disconnected) => return Err(RecvError), // The caller hung up.
                }
            };

            // Convert `Operation` to one or more `squeue::Entry`, and submit to io_uring.
            let index_of_op = self.user_tasks_in_flight.get_next_index().unwrap();
            let entries = match op.build_submission_queue_entries();
            self.user_tasks_in_flight.put(index_of_op, op);
            unsafe {
                self.ring
                    .submission()
                    .push_multiple(entries.as_slice())
                    .unwrap()
            };
            self.n_files_registered += 1; // TODO: When we support more `Operations` than just `get`,
                                          // we'll need a way to only increment this when appropriate.
        }
        Ok(())
    }

    fn submit_and_maybe_wait(&mut self) {
        if self.ring.completion().is_empty() {
            self.ring.submit_and_wait(1).unwrap();
        } else {
            // We need to call `ring.submit()` the first time we submit. And, if sqpoll is enabled, then
            // we also need to call `ring.submit()` to waken the kernel polling thread.
            // `ring.submit()` is basically a no-op if the kernel's sqpoll thread is still awake.
            self.ring.submit().unwrap();
        }
    }

    fn process_uring_cq(&mut self) {
        for cqe in self.ring.completion() {
            // user_data holds the io_uring opcode in the lower 32 bits,
            // and holds the index_of_op in the upper 32 bits.
            let uring_opcode = (cqe.user_data() & 0xFFFFFFFF) as u8;
            let index_of_op = (cqe.user_data() >> 32) as usize;
            let op = self.user_tasks_in_flight.as_mut(index_of_op).unwrap();

            // Handle errors reported by io_uring:
            if cqe.result() < 0 {
                let nix_err = nix::Error::from_i32(-cqe.result());
                let err = anyhow::Error::new(nix_err).context(format!(
                    "{nix_err} (reported by io_uring completion queue entry (CQE) for opcode = {uring_opcode}, opname = {})",
                    opcode_to_opname(uring_opcode),
                ));
                op.send_error(err);
            }

            let error_has_occurred = op.error_has_occurred();

            match op.operation_mut() {
                Operation::Get { path, fixed_fd, .. } => 'get: {
                    match uring_opcode {
                        opcode::OpenAt::CODE => {
                            if error_has_occurred {
                                break 'get;
                            };
                            *fixed_fd = Some(types::Fixed(cqe.result() as u32));
                            let (entries, buffer) = create_linked_read_close_sqes(
                                path,
                                fixed_fd.as_ref().unwrap(),
                                index_of_op,
                            );
                            op.set_output(buffer);
                            self.internal_op_queue.push_back(entries);
                        }
                        opcode::Read::CODE => {
                            if error_has_occurred {
                                break 'get;
                            };
                            op.send_output();
                        }
                        opcode::Close::CODE => {
                            self.user_tasks_in_flight.remove(index_of_op).unwrap();
                            self.n_files_registered -= 1;
                        }
                        _ => panic!("Unrecognised opcode!"),
                    };
                }
            }
        }
    }

    fn sq_len_plus_cq_len(&self) -> usize {
        unsafe { self.ring.submission_shared().len() + self.ring.completion_shared().len() }
    }

    fn uring_is_full(&self) -> bool {
        self.sq_len_plus_cq_len() >= SQ_RING_SIZE - MAX_ENTRIES_PER_CHAIN
    }
}
fn get_filesize_bytes<P>(path: &P) -> i64
where
    P: ?Sized + NixPath,
{
    stat(path).expect("Failed to get filesize!").st_size
}

fn build_openat_sqe(path: &CString, index_of_op: usize) -> VecEntries {
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
    fixed_fd: &types::Fixed,
    index_of_op: usize,
) -> (VecEntries, Output) {
    // Convert the index_of_op into a u64, and bit-shift it left.
    // We do this so the u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
    // and represents the io_uring opcode CODE in the right-most 32 bits.
    let index_of_op: u64 = (index_of_op as u64) << 32;

    // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
    let filesize_bytes = get_filesize_bytes(path.as_c_str());

    // Allocate vector:
    let mut buffer = Vec::with_capacity(filesize_bytes as _);

    // Prepare the "read" opcode:
    let read_op = opcode::Read::new(*fixed_fd, buffer.as_mut_ptr(), filesize_bytes as u32)
        .build()
        .user_data(index_of_op | (opcode::Read::CODE as u64))
        .flags(squeue::Flags::IO_LINK);

    unsafe {
        buffer.set_len(filesize_bytes as _);
    }

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(*fixed_fd)
        .build()
        .user_data(index_of_op | (opcode::Close::CODE as u64));

    (vec![read_op, close_op], Output::Get { buffer })
}

fn opcode_to_opname(opcode: u8) -> &'static str {
    match opcode {
        opcode::OpenAt::CODE => "openat",
        opcode::Read::CODE => "read",
        opcode::Close::CODE => "close",
        _ => "Un-recognised opcode",
    }
}

trait IoUringOperation {
    fn process_completion_queue_entry(&mut self, cqe: cqueue::Entry) {
        self.core().cqe = Some(cqe);
        self.check_for_and_handle_cqe_error();
    }

    /// If called while `self.cqe` is None, then returns the first `Vec<squeue::Entry>`.
    /// If `self.cqe` is `Some(cqe)`, then submit further `Vec<squeue::Entry>` and/or send result.
    /// If the returned `Vec<squeue::Entry>` is empty then this operation can be removed.
    fn build_submission_queue_entries(&mut self) -> Vec<squeue::Entry>;

    fn check_for_and_handle_cqe_error(&mut self) {
        if self.core().cqe.unwrap().as_ref().result() < 0 {
            self.core().error_has_occurred = True;
            let err = cqe_error_to_anyhow_error(self.cqe.unwrap().as_ref());
            self.core().send_error(err);    
        }
    }
}

impl IoUringOperation for operation::Get<cqueue::Entry> {
    fn build_submission_queue_entries(&mut self) -> Vec<squeue::Entry> {
        match self.core.cqe {
            None => build_openat_sqe(&self.core.path, index_of_op)
        }
    }

}