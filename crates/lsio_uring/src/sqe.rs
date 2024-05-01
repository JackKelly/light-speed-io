use io_uring::squeue;
use io_uring::types;
use lsio_aligned_bytes::AlignedBytes;
use lsio_aligned_bytes::AlignedBytesMut;
use std::ffi::CString;
use std::ops::Range;

use crate::open_file::OpenFile;
use crate::{opcode::OpCode, user_data::UringUserData};

const ALIGN: isize = 512; // TODO: Get ALIGN at runtime from statx.

/// # Documentation about the openat operation in io_uring:
/// - https://man7.org/linux/man-pages/man2/openat.2.html
/// - https://man7.org/linux/man-pages/man3/io_uring_prep_openat.3.html
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

/// Build a `statx` submission queue entry (SQE).
///
/// # Safety
/// Assumes the struct that `statx_ptr` points to exists and has been zeroed.
///
/// # Documentation about the statx operation in io_uring:
/// - https://man7.org/linux/man-pages/man2/statx.2.html
/// - https://man7.org/linux/man-pages/man3/io_uring_prep_statx.3.html
/// - https://docs.rs/io-uring/latest/io_uring/opcode/struct.Statx.html
/// - https://docs.rs/libc/latest/libc/struct.statx.html
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
        // `dirfd` is ignored if the pathname is absolute. See:
        // https://man7.org/linux/man-pages/man2/statx.2.html
        types::Fd(-1),
        path_ptr,
        statx_ptr as *mut _,
    )
    // See here for a description of the flags for statx:
    // https://man7.org/linux/man-pages/man2/statx.2.html
    .mask(libc::STATX_SIZE | libc::STATX_DIOALIGN)
    .build()
    .user_data(idx_and_opcode.into())
}

pub(crate) fn build_read_range_sqe(
    index_of_op: usize,
    file: &OpenFile,
    range: &Range<isize>,
) -> (squeue::Entry, AlignedBytes) {
    let filesize: isize = file.size().try_into().unwrap();
    let start_offset = if range.start >= 0 {
        range.start
    } else {
        // `range.start` is negative. We interpret a negative `range.start`
        // as an offset from the end of the file.
        filesize + range.start
    };
    assert!(start_offset >= 0);

    let end_offset = if range.end >= 0 {
        range.end
    } else {
        // `range.end` is negative. We interpret a negative `range.end`
        // as an offset from the end of the file, where `range.end = -1` means the last byte.
        filesize + range.end + 1
    };
    assert!(end_offset >= 0);

    let aligned_start_offset = (start_offset / ALIGN) * ALIGN;

    let buf_len = end_offset - aligned_start_offset;
    assert!(buf_len > 0);

    // Allocate vector. If `buf_len` is not exactly divisible by ALIGN, then
    // `AlignedBytesMut::new` will extend the length until it is aligned.
    let mut buffer = AlignedBytesMut::new(buf_len as usize, ALIGN.try_into().unwrap());
    drop(buf_len); // From now on, use `buffer.len()` as the correct length!

    // Prepare the "read" opcode:
    let read_op = io_uring::opcode::Read::new(
        *file.file_descriptor(),
        buffer.as_mut_ptr(),
        buffer.len().try_into().unwrap(),
    )
    .offset(aligned_start_offset as _)
    .build()
    .user_data(
        UringUserData::new(
            index_of_op.try_into().unwrap(),
            OpCode::new(io_uring::opcode::Read::CODE),
        )
        .into(),
    );

    // If the `start_offset` is not aligned, then the start of the buffer will contain data that
    // the user did not request.
    if aligned_start_offset != start_offset {
        _ = buffer
            .split_to((start_offset - aligned_start_offset).try_into().unwrap())
            .unwrap();
    }

    // `freeze` the buffer, and set the slice to the slice requested by the user:
    let start_slice: usize = (start_offset - aligned_start_offset).try_into().unwrap();
    let end_slice: usize = (end_offset - aligned_start_offset).try_into().unwrap();
    let mut buffer = buffer.freeze().unwrap();
    buffer.set_slice(start_slice..end_slice);

    (read_op, buffer)
}
