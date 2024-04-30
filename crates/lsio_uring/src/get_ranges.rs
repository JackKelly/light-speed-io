use std::{ffi::CString, ops::Range};

#[derive(Debug)]
pub(crate) struct GetRanges {
    // Creating a new CString allocates memory. And io_uring openat requires a CString.
    // We need to ensure the CString is valid until the completion queue entry arrives.
    // So we keep the CString here, in the `Operation`. This `location` hasn't yet been
    // opened, which is why it's not yet an [`OpenFile`].
    location: CString,
    ranges: Vec<Range<isize>>,
    user_data: Vec<u64>,
}
