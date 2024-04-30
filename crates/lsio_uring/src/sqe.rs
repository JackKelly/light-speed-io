use io_uring::squeue;
use io_uring::types;
use nix::sys::stat::stat;
use nix::NixPath;
use std::ffi::CString;
use std::ops::Range;

use crate::{opcode::OpCode, user_data::UringUserData};

fn get_filesize_bytes<P>(path: &P) -> i64
where
    P: ?Sized + NixPath,
{
    stat(path).expect("Failed to get filesize!").st_size
}

pub(crate) fn build_openat_sqe(index_of_op: usize, location: &CString) -> squeue::Entry {
    let idx_and_opcode = UringUserData::new(
        index_of_op.try_into().unwrap(),
        OpCode::new(io_uring::opcode::OpenAt::CODE),
    );

    // Prepare the "openat" submission queue entry (SQE):
    let path_ptr = location.as_ptr();
    io_uring::opcode::OpenAt::new(
        // `dirfd` is ignored if the pathname is absolute.
        // See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        types::Fd(-1),
        path_ptr,
    )
    .flags(libc::O_RDONLY | libc::O_DIRECT)
    .build()
    .user_data(idx_and_opcode.into())
}

pub(crate) fn build_statx_sqe(
    index_of_op: usize,
    location: &CString,
    statx_ptr: *mut libc::statx,
) -> squeue::Entry {
    let idx_and_opcode = UringUserData::new(
        index_of_op.try_into().unwrap(),
        OpCode::new(io_uring::opcode::Statx::CODE),
    );

    // Prepare the "statx" submission queue entry (SQE):
    let path_ptr = location.as_ptr();
    io_uring::opcode::Statx::new(
        // TODO: Check the statement below is still true for statx!
        // `dirfd` is ignored if the pathname is absolute.
        // See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        types::Fd(-1),
        path_ptr,
        statx_ptr,
    )
    .flags(libc::O_RDONLY | libc::O_DIRECT)
    .build()
    .user_data(idx_and_opcode.into())
}

pub(crate) fn build_read_range_sqe(
    index_of_op: usize,
    file: &OpenFile,
    range: &Range<isize>,
) -> (squeue::Entry, AlignedBytes) {
    let start_offset = if range.start >= 0 {
        range.start
    } else {
        // `range.start` is negative. We interpret a negative `range.start`
        // as an offset from the end of the file.
        file.size.unwrap() + range.start
    };
    assert!(start_offset >= 0);

    let end_offset = if range.end >= 0 {
        range.end
    } else {
        // `range.end` is negative. We interpret a negative `range.end`
        // as an offset from the end of the file, where `range.end = -1` means the last byte.
        file.size.unwrap() + range.end + 1
    };
    assert!(end_offset >= 0);

    let aligned_start_offset = (start_offset / ALIGN) * ALIGN;

    let buf_len = end_offset - aligned_start_offset;
    assert!(buf_len > 0);

    // Allocate vector. If `buf_len` is not exactly divisible by ALIGN, then
    // `AlignedBytesMut::new` will extend the length until it is aligned.
    let mut buffer = AlignedBytesMut::new(buf_len as usize, ALIGN);
    drop(buf_len); // From now on, use `buffer.len()` as the correct length!

    // Prepare the "read" opcode:
    let read_op = io_uring::opcode::Read::new(
        *file.fd,
        buffer.as_mut_ptr(),
        buffer.len().try_into::<u32>().unwrap(),
    )
    .offset(aligned_start_offset as _)
    .build()
    .user_data(UringUserData::new(index_of_op.try_into().unwrap(), opcode::Read::CODE).into());

    // If the `start_offset` is not aligned, then the start of the buffer will contain data th
    if aligned_start_offset != start_offset {
        _ = buffer
            .split_to(start_offset - aligned_start_offset)
            .unwrap();
    }

    // `freeze` the buffer, and set the slice to the slice requested by the user:
    let start_slice = start_offset - aligned_start_offset;
    let end_slice = end_offset - aligned_start_offset;
    let mut buffer = buffer.freeze().unwrap();
    buffer.set_slice(start_slice..end_slice);

    (read_op, buffer)
}
