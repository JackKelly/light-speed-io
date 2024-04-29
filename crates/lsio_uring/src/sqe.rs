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

pub(crate) fn build_openat_sqe(index_of_op: usize, path: &CString) -> squeue::Entry {
    let user_data = UringUserData::new(index_of_op.try_into().unwrap(), opcode::OpenAt::CODE);

    // Prepare the "open" opcode:
    let path_ptr = path.as_ptr();
    opcode::OpenAt::new(
        // `dirfd` is ignored if the pathname is absolute.
        // See the "openat()" section in https://man7.org/linux/man-pages/man2/openat.2.html
        types::Fd(-1),
        path_ptr,
    )
    .flags(libc::O_RDONLY | libc::O_DIRECT)
    .build()
    .user_data(user_data.into())
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
    let read_op = opcode::Read::new(
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
