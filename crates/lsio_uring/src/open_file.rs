use std::{
    ffi::CString,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering},
};

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

/// Used to build an [`OpenFile`].
#[derive(Debug)]
pub(crate) struct OpenFileBuilder {
    location: CString,
    file_descriptor: Option<io_uring::types::Fd>,
    statx: MaybeUninit<libc::statx>,
    statx_is_initialised: AtomicBool,
}

impl OpenFileBuilder {
    pub(crate) fn new(location: CString) -> Self {
        Self {
            location,
            file_descriptor: None,
            statx: MaybeUninit::<libc::statx>::uninit(),
            statx_is_initialised: AtomicBool::new(false),
        }
    }

    pub(crate) fn set_file_descriptor(&mut self, file_descriptor: io_uring::types::Fd) {
        self.file_descriptor = Some(file_descriptor);
    }

    pub(crate) fn get_statx_ptr(&self) -> *mut libc::statx {
        self.statx.as_mut_ptr()
    }

    pub(crate) unsafe fn set_statx_as_initialised(&mut self) {
        self.statx_is_initialised.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.file_descriptor.is_some() && self.statx_is_initialised.load(Ordering::Relaxed)
    }

    /// Safety: [`Self::is_ready`] must return `true` before calling `build`!
    /// Panics: If `build` is called while [`Self::is_ready`] is still false.
    pub(crate) fn build(self) -> OpenFile {
        assert!(self.is_ready());
        let statx = unsafe { self.statx.assume_init() };
        OpenFile {
            location: self.location,
            file_descriptor: self.file_descriptor.unwrap(),
            size: statx.stx_size,
            alignment: statx.stx_dio_mem_align,
            // TODO: Maybe also use statx.stx_dio_offset_align.
        }
    }
}
