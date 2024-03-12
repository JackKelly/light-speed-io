use core::slice;
use std::alloc;

#[derive(Debug)]
pub(crate) struct AlignedBuffer {
    buf: *mut u8,
    len: usize,
    layout: alloc::Layout,
}

unsafe impl Send for AlignedBuffer {}

impl AlignedBuffer {
    /// Aligns the start and end of the buffer with `align`.
    /// 'align' must not be zero, and must be a power of two.
    pub(crate) fn new(len: usize, align: usize) -> Self {
        assert_ne!(len, 0);
        let layout = alloc::Layout::from_size_align(len, align)
            .expect("failed to create Layout!")
            .pad_to_align();
        let ptr = unsafe { alloc::alloc(layout) };
        if ptr.is_null() {
            eprint!("ptr is null! handle_alloc_error...");
            alloc::handle_alloc_error(layout);
        }
        Self {
            buf: ptr,
            len,
            layout,
        }
    }

    pub(crate) const fn aligned_len(&self) -> usize {
        self.layout.size()
    }

    pub(crate) fn as_ptr(&mut self) -> *mut u8 {
        self.buf
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.buf, self.len) }
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
        let mut aligned_buf1 = AlignedBuffer::new(LEN, 8);
        let mut aligned_buf2 = AlignedBuffer::new(LEN, 8);

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
