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
        range: Range<i32>,
    },
    #[allow(dead_code)] // TODO: Remove this `allow` when we implement GetRange!
    GetRanges {
        path: CString,
        ranges: Vec<Range<i32>>,
    },
}

#[derive(Debug)]
pub enum OperationOutput {
    Get(Vec<u8>),
    GetRange(Vec<u8>),
    #[allow(dead_code)] // TODO: Remove this `allow` when we implement GetRange!
    GetRanges(Vec<Vec<u8>>),
}
