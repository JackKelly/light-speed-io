use crate::aligned_buffer::AlignedBuffer;
/// `Operation`s are used to communicate the user's instructions
/// to the backend. The intention is that there will be
/// one `Operation` variant per `ObjectStore` method.
/// This is necessary so we can have a queue of (potentially millions of) operations.
/// `Operation` is independent of the IO backend.
use std::{ffi::CString, ops::Range};

#[derive(Debug)]
pub enum Operation {
    Get {
        // Creating a new CString allocates memory. And io_uring openat requires a CString.
        // We need to ensure the CString is valid until the completion queue entry arrives.
        // So we keep the CString here, in the `Operation`.
        path: CString,
    },
    GetRange {
        path: CString,
        range: Range<isize>,
    },
    #[allow(dead_code)] // TODO: Remove this `allow` when we implement GetRange!
    GetRanges {
        path: CString,
        ranges: Vec<Range<isize>>,
    },
}

#[derive(Debug)]
pub enum OperationOutput {
    Get(AlignedBuffer),
    GetRange(AlignedBuffer),
    #[allow(dead_code)] // TODO: Remove this `allow` when we implement GetRange!
    GetRanges(Vec<AlignedBuffer>),
}
