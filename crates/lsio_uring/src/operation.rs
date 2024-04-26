use std::{ffi::CString, ops::Range};

#[derive(Debug)]
pub enum Operation {
    GetRanges {
        // Creating a new CString allocates memory. And io_uring openat requires a CString.
        // We need to ensure the CString is valid until the completion queue entry arrives.
        // So we keep the CString here, in the `Operation`.
        path: CString,
        ranges: Vec<Range<isize>>,
    },
}
