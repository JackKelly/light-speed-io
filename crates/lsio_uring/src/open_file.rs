use std::ffi::CString;

#[derive(Debug)]
pub(crate) struct OpenFile {
    location: CString,
    file_descriptor: io_uring::types::Fd,
    /// The file size in bytes.
    /// Note that we always have to `statx` the file to get the `alignment`, so we'll always get
    /// the file size, too.
    size: usize,
    alignment: u32,
}
