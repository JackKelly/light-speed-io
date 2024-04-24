#![warn(missing_docs)]

//! A memory buffer allocated on the heap.
//!
//! The start position and end position of the backing buffer are both aligned in memory. The user
//! specifies the memory alignment at runtime. This is useful for working with `O_DIRECT` file IO,
//! where the filesystem will often expect the buffer to be aligned to the logical block size of
//! the filesystem[^o_direct] (typically 512 bytes).
//!
//!
//! The API is loosely inspired by the [`bytes`](https://docs.rs/bytes/latest/bytes/index.html) crate.
//! To give a very quick overview of the `bytes` crate: The `bytes` crate has an (immutable)
//! [`Bytes`](https://docs.rs/bytes/latest/bytes/struct.Bytes.html) struct and a (mutable)
//! [`BytesMut`](https://docs.rs/bytes/latest/bytes/struct.BytesMut.html) struct. `BytesMut` can be
//! `split` into multiple non-overlapping owned views into the same backing buffer. The backing
//! buffer is dropped when all the views referencing that buffer are dropped. `BytesMut` can be
//! [frozen](https://docs.rs/bytes/latest/bytes/struct.BytesMut.html#method.freeze) to produce an
//! (immutable) `Bytes` struct which, in turn, can be sliced to produce (potentially overlapping)
//! owned views of the same backing buffer.
//!
//! `aligned_bytes` follows a similar pattern:
//!
//! [`AlignedBytesMut`] can be [`AlignedBytesMut::split_to`] to produce multiple non-overlapping mutable
//! views of the same backing buffer without copying the memory (each `AlignedBytesMut` has its own
//! `range` (which represents the byte range that this `AlignedBytesMut` has exclusive access to)
//! and an `Arc<InnerBuffer>`).
//!
//! When you have finished writing into the buffer, drop all but one of the `AlignedBytesMut`
//! objects, and call [`AlignedBytesMut::freeze_and_grow`] on the last `AlignedByteMut`. This will
//! consume the `AlignedBytesMut` and return an (immutable) [`AlignedBytes`] whose `range` is set
//! to the full extent of the backing buffer. Then you can [`AlignedBytes::slice`] to get
//! (potentially overlapping) owned views of the same backing buffer.
//!
//! The backing buffer will be dropped when all views into the backing buffer are dropped.
//!
//! Unlike `bytes`, `aligned_bytes` does not use a `vtable`, nor does it allow users to grow the
//! backing buffers. `aligned_bytes` implements the minimal set of features required for the rest
//! of the LSIO project! In fact, `aligned_bytes` is _so_ minimal that the only way to write data
//! into an `AlignedBytesMut` is via [`AlignedBytesMut::as_mut_ptr`] (because that's what the
//! operating system expects!)
//!
//! # Examples and use-cases
//!
//! **Use case 1: The user requests multiple contiguous byte ranges from LSIO.**
//!
//! Let's say the user requests two byte ranges from a single file: `0..4096`, and `4096..8192`.
//!
//! Under the hood, LSIO will:
//!
//! - Notice that these two byte ranges are consecutive, and merge these two byte ranges into a
//!   single read operation.
//! - Allocate a single 8,192 byte `AlignedBytesMut`, aligned to 512-bytes.
//! - Submit a `read` operation to `io_uring` for all 8,192 bytes.
//! - When the single read op completes, we `freeze_and_grow` the buffer, which consumes the
//!   `AlignedBytesMut` and returns a 8,192 byte `AlignedBytes`.
//! - Split the `AlignedBytes` into two owned `AlignedBytes`, and return these to the user.
//! - The underlying buffer will be dropped when the user drops the two `AlignedBytes`.
//!
//! Here's a code sketch to show how this works:
//!
//! ```
//! use lsio_aligned_bytes::AlignedBytesMut;
//!
//! // Allocate a single 8,192 byte `AlignedBytesMut`:
//! const LEN: usize = 8_192;
//! const ALIGN: usize = 512;
//! let mut bytes = AlignedBytesMut::new(LEN, ALIGN);
//!
//! // Write into the buffer. (In this toy example, we'll write directly into the buffer.
//! // But in "real" code, we'd pass the pointer to the operating system, which in turn would write
//! // data into the buffer for us.)
//! let ptr = bytes.as_mut_ptr();
//! for i in 0..LEN {
//!     unsafe { *ptr.offset(i as isize) = i as u8; }
//! }
//!
//! // Freeze (to get a read-only `AlignedBytes`). We `unwrap` because `freeze_and_grow` will fail
//! // if there's more than one `AlignedBytesMut` referencing our backing buffer.
//! let bytes = bytes.freeze_and_grow().unwrap();
//! let expected_byte_string: Vec<u8> = (0..LEN).map(|i| i as u8).collect();
//! assert_eq!(bytes.as_slice(), expected_byte_string);
//!
//! // Split
//! let buffer_0 = bytes.slice(0..4_096);
//! let buffer_1 = bytes.slice(4_096..8_192);
//! assert_eq!(buffer_0.len(), 4_096);
//! assert_eq!(buffer_1.len(), 4_096);
//! assert_eq!(buffer_0.as_slice(), &expected_byte_string[0..4_096]);
//! assert_eq!(buffer_1.as_slice(), &expected_byte_string[4_096..8_192]);
//!
//! // Check that the original `bytes` buffer is still valid:
//! assert_eq!(bytes.as_slice(), &expected_byte_string);
//!
//! // Remove the original and check the new buffers again:
//! drop(bytes);
//! assert_eq!(buffer_0.as_slice(), &expected_byte_string[0..4_096]);
//! assert_eq!(buffer_1.as_slice(), &expected_byte_string[4_096..8_192]);
//! ```
//!
//! **Use-case 2: The user requests a single 8 GB file.**
//!
//! Linux can't read more than 2 GB at once.
//!
//! LSIO will:
//! - Allocate a single 8 GB `AlignedBytesMut`.
//! - Split this into a new 6 GB `AlignedBytesMut` and the old `AlignedBytesMut` is reduced to 2 GB.
//!   Both of these buffers must have their starts and ends aligned. Then repeat the process to get 4 x 2 GB `AlignedBytesMut`s.
//! - Issue four `read` operations to the OS (one operation per `AlignedBytesMut`)
//! - When the first, second, and third `read` ops complete, drop their `AlignedBytesMut`
//!   (but that won't drop the underlying storage, it just removes its reference).
//! - When the last `read` op completes, `freeze_and_grow` the last `AlignedBytesMut` to get an immutable `AlignedBytes`
//!   of the 8 GB slice requested by the user. Pass this 8 GB `AlignedBytes` to the user.
//!
//! TODO: Write code sketch for use-case 2!
//!
//! [^o_direct]: For more information on `O_DIRECT`, including the memory alignment requirements,
//! see all the mentions of `O_DIRECT` in the [`open(2)`](https://man7.org/linux/man-pages/man2/open.2.html) man page.
//!
use anyhow;
use std::{alloc, ops::Range, slice, sync::Arc};

/// A mutable aligned buffer.
#[derive(Debug)]
pub struct AlignedBytesMut {
    buf: Arc<InnerBuffer>,
    /// The slice requested by the user.
    range: Range<usize>,
}

unsafe impl Send for AlignedBytesMut {}
unsafe impl Sync for AlignedBytesMut {}

impl AlignedBytesMut {
    /// Aligns the start and end of the buffer with `align`.
    /// 'align' must not be zero, and must be a power of two.
    pub fn new(len: usize, align: usize) -> Self {
        let inner_buf = InnerBuffer::new(len, align);
        Self {
            buf: Arc::new(inner_buf),
            range: 0..len,
        }
    }

    /// The length of the `range` requested by the user. The `range` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub fn len(&self) -> usize {
        self.range.len()
    }

    /// Get a mutable pointer to this `AlignedBytesMut`'s `range`.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        let ptr = self.buf.as_mut_ptr();
        unsafe { ptr.offset(self.range.start as isize) }
    }

    /// Split this view of the underlying buffer into two views at the given index.
    ///
    /// `idx` must not be zero. `idx` must be exactly divisible by the alignment of the underlying
    /// buffer. `idx` must be contained in `self.range`.
    ///
    /// Afterwards, `self` contains `[idx, range.end)` and the returned `AlignedBytesMut`
    /// contains elements `[range.start, idx)`.
    ///
    /// To show this graphically:
    ///
    /// Before calling `split_to`:
    ///
    /// ```text
    /// Underlying buffer:  0 1 2 3 4 5 6 7 8 9
    /// self.range       :     [2,          8)
    /// ```
    ///
    /// After calling `split_to(6)`:
    ///
    /// ```text
    /// Underlying buffer:  0 1 2 3 4 5 6 7 8 9
    /// self.range       :             [6,  8)
    /// other.range      :     [2,      6)
    /// ```
    pub fn split_to(&mut self, idx: usize) -> anyhow::Result<Self> {
        if !self.range.contains(&idx) {
            Err(anyhow::format_err!(
                "idx {idx} is not contained in this buffer's range {:?}",
                self.range,
            ))
        } else if idx == 0 {
            Err(anyhow::format_err!("idx must not be zero!"))
        } else if idx % self.buf.alignment() != 0 {
            Err(anyhow::format_err!(
                "idx {idx} must be exactly divisible by the alignment {}",
                self.buf.alignment()
            ))
        } else {
            let new_range = self.range.start..idx;
            self.range.start = idx;
            Ok(AlignedBytesMut {
                buf: self.buf.clone(),
                range: new_range,
            })
        }
    }

    /// If this is the only `AlignedBytesMut` with access to the underlying buffer
    /// then `freeze_and_grow` returns a read-only `AlignedBytes` (wrapped in `Ok`), which contains a
    /// reference to the underlying buffer, and has its `range` set to the entire byte range
    /// of the underlying buffer. If, on the other hand, other `AlignedBytesMut`s have access to
    /// the underlying then `freeze_and_grow` will return `Err(self)`.
    pub fn freeze_and_grow(self) -> Result<AlignedBytes, Self> {
        if Arc::strong_count(&self.buf) == 1 {
            Ok(AlignedBytes {
                buf: self.buf.clone(),
                range: 0..self.buf.len(),
            })
        } else {
            Err(self)
        }
    }
}

/// Immutable.
#[derive(Debug, Clone)]
pub struct AlignedBytes {
    buf: Arc<InnerBuffer>,
    /// The slice requested by the user.
    range: Range<usize>,
}

unsafe impl Send for AlignedBytes {}
unsafe impl Sync for AlignedBytes {}

impl AlignedBytes {
    /// Returns a slice of self for the provided range.
    ///
    /// This will increment the reference count for the underlying memory and return a new `AlignedBytes`
    /// handle set to the slice.
    ///
    /// The requested `range` indexes into the entire underlying buffer.
    ///
    /// ## Panics
    /// Panics if `range.is_empty()` or if `range.end` > the size of the underlying buffer.
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(!range.is_empty());
        assert!(range.end <= self.buf.len());
        Self {
            buf: self.buf.clone(),
            range,
        }
    }

    /// The length of the `range` requested by the user. The `range` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub fn len(&self) -> usize {
        self.range.len()
    }

    pub fn as_ptr(&self) -> *const u8 {
        let ptr = self.buf.as_ptr();
        unsafe { ptr.offset(self.range.start as isize) }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.as_ptr(), self.len()) }
    }
}

#[derive(Debug)]
struct InnerBuffer {
    buf: *mut u8, // TODO: Use `NotNull`.
    /// `layout.size()` gives the number of bytes _actually_ allocated, which will be
    /// a multiple of `align`.
    layout: alloc::Layout,
}

impl InnerBuffer {
    fn new(len: usize, align: usize) -> Self {
        assert_ne!(len, 0);
        let layout = alloc::Layout::from_size_align(len, align)
            .expect("failed to create Layout!")
            .pad_to_align();
        let buf = unsafe { alloc::alloc(layout) };
        if buf.is_null() {
            alloc::handle_alloc_error(layout);
        }
        Self { buf, layout }
    }

    /// The total size of the underlying buffer.
    const fn len(&self) -> usize {
        self.layout.size()
    }

    /// Get the alignment, in bytes.
    const fn alignment(&self) -> usize {
        self.layout.align()
    }

    fn as_mut_ptr(&self) -> *mut u8 {
        self.buf
    }

    fn as_ptr(&self) -> *const u8 {
        self.buf
    }
}

impl Drop for InnerBuffer {
    fn drop(&mut self) {
        unsafe { alloc::dealloc(self.buf, self.layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_and_read() {
        // Create a new buffer:
        const LEN: usize = 16;
        let mut aligned_buf1 = AlignedBytesMut::new(LEN, 8);
        let mut aligned_buf2 = AlignedBytesMut::new(LEN, 8);

        // Set the values of the buffer:
        {
            let ptr1 = aligned_buf1.as_mut_ptr();
            let ptr2 = aligned_buf2.as_mut_ptr();
            unsafe {
                for i in 0..LEN {
                    *ptr1.offset(i as _) = i as u8;
                    *ptr2.offset(i as _) = i as u8;
                }
            }
        }
        // Read the values back out:
        {
            let slice1 = aligned_buf1.freeze_and_grow().unwrap();
            let slice2 = aligned_buf2.freeze_and_grow().unwrap();
            for i in 0..LEN {
                assert_eq!(slice1.as_slice()[i], i as u8);
                assert_eq!(slice2.as_slice()[i], i as u8);
            }
            assert_eq!(
                slice1.as_slice(),
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
            );
        }
    }
}
