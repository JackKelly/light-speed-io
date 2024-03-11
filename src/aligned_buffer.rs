use core::{ptr::NonNull, slice};
use std::alloc;

struct AlignedBuffer {
    buf: NonNull<u8>,
    len: usize,
    layout: alloc::Layout,
}

impl AlignedBuffer {
    /// align must not be zero, and must be a power of two.
    fn new(len: usize, align: usize) -> Self {
        assert_ne!(len, 0);
        let layout = alloc::Layout::from_size_align(len, align).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        if ptr.is_null() {
            alloc::handle_alloc_error(layout);
        }
        Self {
            buf: NonNull::new(ptr).expect("ptr is null!"),
            len,
            layout,
        }
    }

    fn as_mut(&mut self) -> *mut u8 {
        unsafe { self.buf.as_mut() }
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.buf.as_ref(), self.len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        unsafe { alloc::dealloc(self.buf.as_ptr(), self.layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_and_read() {
        // Create a new buffer:
        const LEN: usize = 16;
        let mut aligned_buf = AlignedBuffer::new(LEN, 8);

        // Set the values of the buffer:
        {
            let ptr = aligned_buf.as_mut();
            unsafe {
                for i in 0..LEN {
                    *ptr.offset(i as _) = i as u8;
                }
            }
        }
        // Read the values back out:
        {
            let slice = aligned_buf.as_slice();
            for i in 0..LEN {
                assert_eq!(slice[i], i as u8);
            }
        }
    }
}
