use core::{ops::RangeBounds, slice};
use std::alloc;

/// A memory buffer allocated on the heap, where the start position and end position of the backing
/// buffer are both aligned. This is useful for working with O_DIRECT file IO, where the filesystem
/// will often expect the buffer to be aligned to the logical block size (typically 512 bytes).
#[derive(Debug)]
pub struct AlignedBuffer<R>
where
    R: RangeBounds<usize> + ExactSizeIterator,
{
    buf: *mut u8,
    /// The slice requested by the user.
    valid_slice: R,
    /// `layout.size()` gives the number of bytes _actually_ allocated, which will be
    /// a multiple of `align`.
    layout: alloc::Layout,
}

unsafe impl<R> Send for AlignedBuffer<R> where R: RangeBounds<usize> + ExactSizeIterator {}

impl<R> AlignedBuffer<R>
where
    R: RangeBounds<usize> + ExactSizeIterator,
{
    /// Aligns the start and end of the buffer with `align`.
    /// 'align' must not be zero, and must be a power of two.
    pub(crate) fn new(slice: R, align: usize) -> Self {
        assert_ne!(slice.len(), 0);
        // Let's say the user requests a buffer of len 3 and offset 2; and align is 4:
        //            index:     0 1 2 3 4 5 6 7
        //   aligned blocks:     |------|------|
        //        requested:         |---|
        // In this case, we need to allocate 8 bytes becuase we need to move the start
        // backwards to the first byte, and move the end forwards to the eighth byte.
        let layout =
            alloc::Layout::from_size_align(slice.len() + (slice.start_bound() % align), align)
                .expect("failed to create Layout!")
                .pad_to_align();
        let buf = unsafe { alloc::alloc(layout) };
        if buf.is_null() {
            alloc::handle_alloc_error(layout);
        }
        Self {
            buf,
            valid_slice: slice,
            layout,
        }
    }

    pub(crate) fn aligned_start_offset(&self) -> usize {
        (self.valid_slice.start_bound() / self.layout.align()) * self.layout.align()
    }

    /// The length of the `valid_slice` requested by the user. The `valid_slice` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub(crate) fn len(&self) -> usize {
        self.valid_slice.len()
    }

    /// The length of the underlying buffer.
    pub(crate) const fn capacity(&self) -> usize {
        self.layout.size()
    }

    pub(crate) fn as_ptr_to_underlying_buf(&mut self) -> *mut u8 {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            slice::from_raw_parts(
                self.buf
                    .offset(self.valid_slice.start_bound().try_into().unwrap()),
                self.len(),
            )
        }
    }
}

impl<R> Drop for AlignedBuffer<R>
where
    R: RangeBounds<usize> + ExactSizeIterator,
{
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
        let mut aligned_buf1 = AlignedBuffer::new(..LEN, 8);
        let mut aligned_buf2 = AlignedBuffer::new(..LEN, 8);

        // Set the values of the buffer:
        {
            let ptr1 = aligned_buf1.as_ptr();
            let ptr2 = aligned_buf2.as_ptr();
            unsafe {
                for i in 0..LEN {
                    *ptr1.offset(i as _) = i as u8;
                    *ptr2.offset(i as _) = i as u8;
                }
            }
        }
        // Read the values back out:
        {
            let slice1 = aligned_buf1.as_slice();
            let slice2 = aligned_buf2.as_slice();
            for i in 0..LEN {
                assert_eq!(slice1[i], i as u8);
                assert_eq!(slice2[i], i as u8);
            }
            assert_eq!(
                slice1,
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
            );
        }
    }
}
