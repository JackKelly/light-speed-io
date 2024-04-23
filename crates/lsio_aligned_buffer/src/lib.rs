use core::{ops::Range, slice};
use std::{
    alloc,
    collections::HashSet,
    sync::{Arc, Mutex},
};

/// A memory buffer allocated on the heap, where the start position and end position of the backing
/// buffer are both aligned. This is useful for working with O_DIRECT file IO, where the filesystem
/// will often expect the buffer to be aligned to the logical block size (typically 512 bytes).
#[derive(Debug)]
pub struct AlignedBufferMut {
    buf: Arc<InnerAlignedBuffer>,
    /// The slice requested by the user.
    valid_slice: Range<usize>,
}

unsafe impl Send for AlignedBufferMut {}
unsafe impl Sync for AlignedBufferMut {}

impl AlignedBufferMut {
    /// Aligns the start and end of the buffer with `align`.
    /// 'align' must not be zero, and must be a power of two.
    pub(crate) fn new(len: usize, align: usize) -> Self {
        let inner_buf = InnerAlignedBuffer::new(len, align);
        Self {
            buf: Arc::new(inner_buf),
            valid_slice: 0..len,
        }
    }

    /// The length of the `valid_slice` requested by the user. The `valid_slice` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub(crate) fn len(&self) -> usize {
        self.valid_slice.len()
    }

    /// Get a mutable pointer to this `AlignedBufferMut`'s `valid_slice`.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut u8 {
        let ptr = self.buf.as_mut_ptr();
        unsafe { ptr.offset(self.valid_slice.start as isize) }
    }

    /// Split this view of the underlying buffer into two views at the given index.
    ///
    /// `idx` must not be zero. `idx` must be exactly divisible by the alignment of the underlying
    /// buffer. `idx` must be contained in `self.valid_slice`.
    ///
    /// Afterwards, `self` contains `[idx, valid_slice.end)` and the returned `AlignedBufferMut`
    /// contains elements `[valid_slice.start, idx)`.
    ///
    /// To show this graphically:
    ///
    /// Before calling `split_to`:
    ///
    /// Underlying buffer:  0 1 2 3 4 5 6 7 8 9
    /// self.valid_slice :     [2,          8)
    ///
    /// After calling `split_to(6)`:
    ///
    /// self.valid_slice :             [6,  8)
    /// other.valid_slice:     [2,      6)
    pub(crate) fn split_to(&mut self, idx: usize) -> anyhow::Result<Self> {
        if !self.valid_slice.contains(&idx) {
            Err(anyhow::format_err!(
                "idx {idx} is not contained in this buffer's valid_slice {:?}",
                self.valid_slice,
            ))
        } else if idx == 0 {
            Err(anyhow::format_err!("idx must not be zero!"))
        } else if idx % self.buf.alignment() != 0 {
            Err(anyhow::format_err!(
                "idx {idx} must be exactly divisible by the alignment {}",
                self.buf.alignment()
            ))
        } else {
            let new_valid_slice = self.valid_slice.start..idx;
            self.valid_slice.start = idx;
            Ok(AlignedBufferMut {
                buf: self.buf.clone(),
                valid_slice: new_valid_slice,
            })
        }
    }

    /// If this is the only `AlignedBufferMut` with access to the underlying buffer
    /// then `freeze_and_grow` returns a read-only `AlignedBuffer` (wrapped in `Ok`), which contains a
    /// reference to the underlying buffer, and has its `valid_slice` set to the entire byte range
    /// of the underlying buffer. If, on the other hand, other `AlignedBufferMut`s have access to
    /// the underlying then `freeze_and_grow` will return `Err(self)`.
    pub(crate) fn freeze_and_grow(self) -> Result<AlignedBuffer, Self> {
        if Arc::strong_count(&self.buf) == 1 {
            Ok(AlignedBuffer {
                buf: self.buf.clone(),
                valid_slice: 0..self.buf.size(),
            })
        } else {
            Err(self)
        }
    }
}

/// Immutable.
#[derive(Debug)]
struct AlignedBuffer {
    buf: Arc<InnerAlignedBuffer>,
    /// The slice requested by the user.
    valid_slice: Range<usize>,
}

unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

impl AlignedBuffer {
    /// The length of the `valid_slice` requested by the user. The `valid_slice` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub(crate) fn len(&self) -> usize {
        self.valid_slice.len()
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            slice::from_raw_parts(
                self.buf
                    .as_ptr()
                    .offset(self.valid_slice.start.try_into().unwrap()),
                self.len(),
            )
        }
    }
}

#[derive(Debug)]
struct InnerAlignedBuffer {
    buf: *mut u8,
    /// `layout.size()` gives the number of bytes _actually_ allocated, which will be
    /// a multiple of `align`.
    layout: alloc::Layout,
}

impl InnerAlignedBuffer {
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
    const fn size(&self) -> usize {
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

impl Drop for InnerAlignedBuffer {
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
        let mut aligned_buf1 = AlignedBufferMut::new(LEN, 8);
        let mut aligned_buf2 = AlignedBufferMut::new(LEN, 8);

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
