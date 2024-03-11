use io_uring::cqueue;
use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use nix::sys::stat::stat;
use nix::NixPath;
use std::ffi::CString;
use std::ops::Range;
use tokio::sync::oneshot;

use crate::{aligned_buffer::AlignedBuffer, operation};

const ALIGN: usize = 512;

pub(super) trait Operation {
    fn process_cqe(&mut self, cqe: cqueue::Entry);
    /// If called while `self.inner.last_cqe` is `None`, then returns the first `squeue::Entry`(s).
    /// If `self.inner.last_cqe` is `Some(cqe)`, then submit further SQEs and/or send result.
    fn next_step(&mut self, index_of_op: usize) -> NextStep;
}

pub(super) enum NextStep {
    SubmitEntries {
        entries: Vec<squeue::Entry>,
        // If true, then these squeue entries will register one file.
        register_file: bool,
    },
    Pending,
    // We're done! Remove this operation from the list of ops in flight.
    Done {
        // If true, the the CQE reports that it's unregistered one file.
        unregister_file: bool,
    },
}

#[derive(Debug)]
pub(super) struct InnerState {
    pub(super) output: Option<operation::OperationOutput>,
    // `output_channel` is an `Option` because `send` consumes itself,
    // so we need to `output_channel.take().unwrap().send(Some(buffer))`.
    pub(super) output_channel: Option<oneshot::Sender<anyhow::Result<operation::OperationOutput>>>,
    pub(super) error_has_occurred: bool,
    pub(super) last_cqe: Option<cqueue::Entry>,
    pub(super) last_opcode: Option<u8>,
    pub(super) n_steps_completed: usize,
}

impl InnerState {
    pub(super) fn new(
        output_channel: oneshot::Sender<anyhow::Result<operation::OperationOutput>>,
    ) -> Self {
        Self {
            output: None,
            output_channel: Some(output_channel),
            error_has_occurred: false,
            last_cqe: None,
            last_opcode: None,
            n_steps_completed: 0,
        }
    }

    pub(super) fn send_output(&mut self) {
        self.output_channel
            .take()
            .unwrap()
            .send(Ok(self.output.take().unwrap()))
            .unwrap();
    }

    pub(super) fn process_cqe(&mut self, cqe: cqueue::Entry) {
        // user_data holds the io_uring opcode in the lower 32 bits,
        // and holds the index_of_op in the upper 32 bits.
        self.last_opcode = Some((cqe.user_data() & 0xFFFFFFFF) as u8);
        self.last_cqe = Some(cqe);

        if self.last_cqe.as_ref().unwrap().result() < 0 {
            let err = self.cqe_error_to_anyhow_error();
            self.send_error(err);
            self.error_has_occurred = true;
        }
    }

    pub(super) fn send_error(&mut self, error: anyhow::Error) {
        if self.error_has_occurred {
            eprintln!("The output_channel has already been consumed (probably by sending a previous error)! But a new error has been reported: {error}");
            return;
        }

        let error = error.context(format!("IoUringUserOp = {self:?}"));

        self.output_channel
            .take()
            .unwrap()
            .send(Err(error))
            .unwrap();
    }

    pub(super) fn cqe_error_to_anyhow_error(&self) -> anyhow::Error {
        let cqe = self.last_cqe.as_ref().unwrap();
        let nix_err = nix::Error::from_i32(-cqe.result());
        anyhow::Error::new(nix_err).context(format!(
            "{nix_err} (reported by io_uring completion queue entry (CQE) for opcode = {}, opname = {})",
            self.last_opcode.unwrap(), opcode_to_opname(self.last_opcode.unwrap())
        ))
    }

    pub(super) fn check_n_steps_completed_is_1(&self) {
        assert_eq!(
            self.n_steps_completed, 1,
            "`next_step` has been called {} times, yet `last_cqe` is None. Have you forgotten to call `process_cqe`?",
            self.n_steps_completed
        );
    }
}

fn get_filesize_bytes<P>(path: &P) -> i64
where
    P: ?Sized + NixPath,
{
    stat(path).expect("Failed to get filesize!").st_size
}

pub(super) fn build_openat_sqe(path: &CString, index_of_op: usize) -> Vec<squeue::Entry> {
    // TODO: Test for these:
    // - opcode::OpenAt2::CODE
    // - opcode::Close::CODE
    // - opcode::Socket::CODE // to ensure fixed table support

    // Convert the index_of_op into a u64, and bit-shift it left.
    // We do this so the u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
    // and represents the io_uring opcode CODE in the right-most 32 bits.
    let index_of_op: u64 = (index_of_op as u64) << 32;

    // Prepare the "open" opcode:
    let path_ptr = path.as_ptr();
    let file_index = types::DestinationSlot::auto_target();
    let open_op = opcode::OpenAt::new(
        types::Fd(-1), // dirfd is ignored if the pathname is absolute. See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        path_ptr,
    )
    .file_index(Some(file_index))
    .flags(libc::O_RDONLY) // | libc::O_DIRECT)
    .build()
    .user_data(index_of_op | (opcode::OpenAt::CODE as u64));

    vec![open_op]
}

pub(super) fn create_linked_read_close_sqes(
    path: &CString,
    fixed_fd: &types::Fixed,
    index_of_op: usize,
) -> (Vec<squeue::Entry>, operation::OperationOutput) {
    // Convert the index_of_op into a u64, and bit-shift it left.
    // We do this so the u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
    // and represents the io_uring opcode CODE in the right-most 32 bits.
    let index_of_op: u64 = (index_of_op as u64) << 32;

    // Get filesize: TODO: Use io_uring to get filesize; see issue #41.
    let filesize_bytes = get_filesize_bytes(path.as_c_str());

    // Allocate vector:
    let mut buffer = AlignedBuffer::new(filesize_bytes as _, ALIGN);

    // Prepare the "read" opcode:
    let read_op = opcode::Read::new(*fixed_fd, buffer.as_mut(), buffer.aligned_len() as u32)
        .build()
        .user_data(index_of_op | (opcode::Read::CODE as u64))
        .flags(squeue::Flags::IO_LINK);

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(*fixed_fd)
        .build()
        .user_data(index_of_op | (opcode::Close::CODE as u64));

    (
        vec![read_op, close_op],
        operation::OperationOutput::Get(buffer),
    )
}

pub(super) fn create_linked_read_range_close_sqes(
    range: &Range<isize>,
    fixed_fd: &types::Fixed,
    index_of_op: usize,
) -> (Vec<squeue::Entry>, operation::OperationOutput) {
    // Convert the index_of_op into a u64, and bit-shift it left.
    // We do this so the u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
    // and represents the io_uring opcode CODE in the right-most 32 bits.
    let index_of_op: u64 = (index_of_op as u64) << 32;

    // Allocate vector:
    let mut buffer = AlignedBuffer::new(range.len(), ALIGN);

    // Prepare the "read" opcode:
    let read_op = opcode::Read::new(*fixed_fd, buffer.as_mut(), buffer.aligned_len() as u32)
        .offset(range.start as _)
        .build()
        .user_data(index_of_op | (opcode::Read::CODE as u64))
        .flags(squeue::Flags::IO_LINK);

    // Prepare the "close" opcode:
    let close_op = opcode::Close::new(*fixed_fd)
        .build()
        .user_data(index_of_op | (opcode::Close::CODE as u64));

    (
        vec![read_op, close_op],
        operation::OperationOutput::GetRange(buffer),
    )
}

fn opcode_to_opname(opcode: u8) -> &'static str {
    match opcode {
        opcode::OpenAt::CODE => "openat",
        opcode::Read::CODE => "read",
        opcode::Close::CODE => "close",
        _ => "Un-recognised opcode",
    }
}
