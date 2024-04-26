use io_uring::cqueue;
use io_uring::opcode;
use io_uring::squeue;
use io_uring::types;
use nix::sys::stat::stat;
use nix::NixPath;
use std::ffi::CString;
use std::ops::Range;

use crate::uring_user_data::UringUserData;

fn cqe_error_to_anyhow_error(cqe_result: i64) -> anyhow::Error {
    let nix_err = nix::Error::from_raw(-cqe_result);
    anyhow::Error::new(nix_err).context(format!(
        "{nix_err} (reported by io_uring completion queue entry (CQE))",
    ))
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

    let user_data = UringUserData::new(index_of_op.try_into().unwrap(), opcode::OpenAt::CODE);

    // Prepare the "open" opcode:
    let path_ptr = path.as_ptr();
    let file_index = types::DestinationSlot::auto_target();
    let open_op = opcode::OpenAt::new(
        types::Fd(-1), // dirfd is ignored if the pathname is absolute. See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        path_ptr,
    )
    .file_index(Some(file_index))
    .flags(libc::O_RDONLY | libc::O_DIRECT)
    .build()
    .user_data(user_data.into());

    vec![open_op]
}

pub(super) fn create_linked_read_range_close_sqes(
    path: &CString,
    range: &Range<isize>,
    fixed_fd: &types::Fixed,
    index_of_op: usize,
) -> (Vec<squeue::Entry>, operation::OperationOutput) {
    // Get the start_offset and len of the range:
    let filesize = if range.start < 0 || range.end < 0 {
        // Call `get_filesize_bytes` at most once per file!
        Some(get_filesize_bytes(path.as_c_str()) as isize)
    } else {
        None
    };
    let start_offset = if range.start >= 0 {
        range.start
    } else {
        // range.start is negative, so we must interpret it as an offset from the end of the file.
        filesize.unwrap() + range.start
    };
    let end_offset = if range.end >= 0 {
        range.end
    } else {
        // range.end is negative, so we must interpret it as an offset from the end of the file.
        filesize.unwrap() + range.end + 1
    };
    let len = end_offset - start_offset;
    assert!(len > 0);

    // Allocate vector:
    let mut buffer = AlignedBuffer::new(len as usize, ALIGN, start_offset.try_into().unwrap());

    // Prepare the "read" opcode:
    let read_op = opcode::Read::new(*fixed_fd, buffer.as_ptr(), buffer.aligned_len() as u32)
        .offset(buffer.aligned_start_offset() as _)
        .build()
        .user_data(UringUserData::new(index_of_op.try_into().unwrap(), opcode::Read::CODE).into())
        .flags(squeue::Flags::IO_HARDLINK); // We need a _hard_ link because read will fail if we read
                                            // beyond the end of the file, which is very likely to happen when we're using O_DIRECT.
                                            // When using O_DIRECT, the read length has to be a multiple of ALIGN.
                                            // Prepare the "close" opcode:
    let close_op = opcode::Close::new(*fixed_fd)
        .build()
        .user_data(UringUserData::new(index_of_op.try_into().unwrap(), opcode::Close::CODE).into());

    (
        vec![read_op, close_op],
        operation::OperationOutput::GetRange(buffer),
    )
}
