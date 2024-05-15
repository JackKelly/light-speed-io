#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

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
    /// Creates a new `AlignedBytesMut`.
    ///
    /// Aligns the start and end of the buffer with `align`.
    /// 'align' must not be zero, and must be a power of two.
    /// `len` is the length of the underlying buffer, in bytes.
    pub fn new(len: usize, align: usize) -> Self {
        let inner_buf = InnerBuffer::new(len, align);
        Self {
            buf: Arc::new(inner_buf),
            range: 0..len,
        }
    }

    /// Returns the length of the `range` requested by the user. The `range` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub fn len(&self) -> usize {
        self.range.len()
    }

    /// Returns a mutable pointer to the underlying buffer offset by `self.range.start`.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        let ptr = self.buf.as_mut_ptr();
        unsafe { ptr.offset(self.range.start as isize) }
    }

    /// Split this view of the underlying buffer into two views at the given index.
    ///
    /// This does not allocate a new buffer. Instead, both `AlignedBytesMut` objects reference
    /// the same underlying backing buffer.
    ///
    /// `idx` indexes into the backing buffer.
    ///
    /// `idx` must not be zero. `idx` must be exactly divisible by the alignment of the underlying
    /// buffer. `idx` must be contained in `self.range`.
    ///
    /// Afterwards, `self` contains `[idx, range.end)`. The returned `AlignedBytesMut`
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
    /// then `freeze` consumes `self` and returns a read-only `AlignedBytes`
    /// (wrapped in `Ok`), which contains a reference to the underlying buffer,
    /// and has its `range` set to byte range of the `AlignedBytesMut`.
    /// If, on the other hand, other `AlignedBytesMut`s have access to
    /// the underlying buffer then `freeze` will return `Err(self)`.
    pub fn freeze(self) -> Result<AlignedBytes, Self> {
        if Arc::strong_count(&self.buf) == 1 {
            Ok(AlignedBytes {
                buf: self.buf,
                range: self.range,
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

/// An immutable view of a memory buffer.
///
/// The only way to make is an `AlignedBytes` is using [`AlignedBytesMut::freeze`].
impl AlignedBytes {
    /// Sets the slice for `self`.
    ///
    /// The requested `range` indexes into the entire underlying buffer.
    ///
    /// ## Panics
    /// Panics if `range.is_empty()` or if `range.end` > the size of the underlying buffer.
    pub fn set_slice(&mut self, range: Range<usize>) -> &Self {
        assert!(!range.is_empty());
        assert!(range.end <= self.buf.len());
        self.range = range;
        self
    }

    /// Resets this `AlignedBytes` range to be equal to the total extent of the underlying buffer.
    pub fn reset_slice(&mut self) -> &Self {
        self.range = 0..self.buf.len();
        self
    }

    /// Returns the length of the `range` requested by the user. The `range` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub fn len(&self) -> usize {
        self.range.len()
    }

    /// Returns a constant pointer to `self.range.start` of the underlying buffer.
    pub fn as_ptr(&self) -> *const u8 {
        let ptr = self.buf.as_ptr();
        unsafe { ptr.offset(self.range.start as isize) }
    }

    /// Returns an immutable slice of the `range` view of the underlying buffer.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.as_ptr(), self.len()) }
    }
}

#[derive(Debug)]
struct InnerBuffer {
    buf: *mut u8, // TODO: Replace `*mut u8` with `NotNull<u8>`.

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

    /// Returns the total size of the underlying buffer.
    const fn len(&self) -> usize {
        self.layout.size()
    }

    /// Returns the alignment, in bytes.
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
            let slice1 = aligned_buf1.freeze().unwrap();
            let slice2 = aligned_buf2.freeze().unwrap();
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
