use core::slice;
use std::alloc;

/// A memory buffer allocated on the heap, where the start position and end position are both
/// aligned to `align` bytes. This is useful for working with O_DIRECT file IO, where the
/// filesystem will often expect the buffer to be aligned to the logical block size (typically 512
/// bytes).
#[derive(Debug)]
pub struct AlignedBuffer {
    buf: *mut u8,
    len: usize,          // The number of bytes requested by the user.
    start_offset: usize, // The number of bytes unused at the start of the buffer.
    layout: alloc::Layout, // `layout.size()` gives the number of bytes _actually_ allocated,
                         // which will be a multiple of `align`.
}

unsafe impl Send for AlignedBuffer {}

impl AlignedBuffer {
    /// Aligns the start and end of the buffer with `align`.
    /// 'align' must not be zero, and must be a power of two.
    pub(crate) fn new(len: usize, align: usize, start_offset: usize) -> Self {
        assert_ne!(len, 0);
        // Let's say the user requests a buffer of len 3 and offset 2; and align is 4:
        //            index:     0 1 2 3 4 5 6 7
        //   aligned blocks:     |------|------|
        //        requested:         |---|
        // In this case, we need to allocate 8 bytes becuase we need to move the start
        // backwards to the first byte, and move the end forwards to the eighth byte.
        let layout = alloc::Layout::from_size_align(len + (start_offset % align), align)
            .expect("failed to create Layout!")
            .pad_to_align();
        let buf = unsafe { alloc::alloc(layout) };
        if buf.is_null() {
            alloc::handle_alloc_error(layout);
        }
        Self {
            buf,
            len,
            start_offset,
            layout,
        }
    }

    pub(crate) fn aligned_start_offset(&self) -> usize {
        (self.start_offset / self.layout.align()) * self.layout.align()
    }

    pub(crate) const fn aligned_len(&self) -> usize {
        self.layout.size()
    }

    pub(crate) fn as_ptr(&mut self) -> *mut u8 {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            slice::from_raw_parts(
                self.buf.offset(self.start_offset.try_into().unwrap()),
                self.len,
            )
        }
    }
}

impl Drop for AlignedBuffer {
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
        let mut aligned_buf1 = AlignedBuffer::new(LEN, 8, 0);
        let mut aligned_buf2 = AlignedBuffer::new(LEN, 8, 0);

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
