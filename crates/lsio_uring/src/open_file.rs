use std::ffi::CString;

#[derive(Debug)]
pub(crate) struct OpenFile {
    location: CString,
    file_descriptor: io_uring::types::Fd,
    /// The file size in bytes.
    /// Note that we always have to `statx` the file to get the `alignment`, so we'll always get
    /// the file size, too.
    size: u64,
    alignment: u32,
}

impl OpenFile {
    pub(crate) fn file_descriptor(&self) -> &io_uring::types::Fd {
        &self.file_descriptor
    }

    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn alignment(&self) -> u32 {
        self.alignment
    }
}

/// Used to build an [`OpenFile`].
#[derive(Debug)]
pub(crate) struct OpenFileBuilder {
    location: CString,
    file_descriptor: Option<io_uring::types::Fd>,
    statx: libc::statx,
    assume_statx_is_initialised: bool,
}

impl OpenFileBuilder {
    pub(crate) fn new(location: CString) -> Self {
        Self {
            location,
            file_descriptor: None,
            statx: unsafe { std::mem::zeroed() },
            assume_statx_is_initialised: false,
        }
    }

    pub(crate) const fn location(&self) -> &CString {
        &self.location
    }

    pub(crate) fn set_file_descriptor(&mut self, file_descriptor: io_uring::types::Fd) {
        self.file_descriptor = Some(file_descriptor);
    }

    pub(crate) fn get_statx_ptr(&self) -> *mut libc::statx {
        &mut self.statx as *mut libc::statx
    }

    pub(crate) unsafe fn assume_statx_is_initialised(&mut self) {
        self.assume_statx_is_initialised = true;
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.file_descriptor.is_some() && self.assume_statx_is_initialised
    }

    /// Safety: [`Self::is_ready`] must return `true` before calling `build`!
    /// Panics: If `build` is called while [`Self::is_ready`] is still false.
    pub(crate) fn build(self) -> OpenFile {
        assert!(self.is_ready());
        OpenFile {
            location: self.location,
            file_descriptor: self.file_descriptor.unwrap(),
            size: self.statx.stx_size,
            alignment: self.statx.stx_dio_mem_align,
            // TODO: Maybe also use `statx.stx_dio_offset_align`.
        }
    }
}
