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
    //
    // TODO: Actually, I think `new` should take a `len` argument, _not_ a `slice` argument.
    // For a new `AlignedBufferMut`, `valid_slice` should be `0..len`.
    // And then, slices should always be guaranteed to be fully aligned.
    // Then we can get rid of the confusing `as_mut_ptr_to_start_of_underlying_buf`, and rename
    // `as_mut_ptr_to_valid_slice` to `as_mut_ptr`.
    pub(crate) fn new(slice: Range<usize>, align: usize) -> Self {
        assert_ne!(slice.len(), 0);
        // Let's say the user requests a buffer of len 3 and offset 2; and align is 4:
        //            index:     0 1 2 3 4 5 6 7
        //   aligned blocks:     |------|------|
        //        requested:         |---|
        // In this case, we need to allocate 8 bytes becuase we need to move the start
        // backwards to the first byte, and move the end forwards to the eighth byte.
        let inner_buf = InnerAlignedBuffer::new(slice.len() + (slice.start % align), align);
        inner_buf.request_new_exclusive_slice(&slice).unwrap();
        Self {
            buf: Arc::new(inner_buf),
            valid_slice: slice,
        }
    }

    /// The length of the `valid_slice` requested by the user. The `valid_slice` is a view into the
    /// underlying buffer. The underlying buffer may be larger than `len`.
    pub(crate) fn len(&self) -> usize {
        self.valid_slice.len()
    }

    /// The length of the underlying buffer, in bytes.
    pub(crate) fn capacity(&self) -> usize {
        self.buf.size()
    }

    pub(crate) fn as_mut_ptr_to_start_of_underlying_buf(&mut self) -> *mut u8 {
        self.buf.as_mut_ptr()
    }

    pub(crate) fn as_mut_ptr_to_valid_slice(&mut self) -> *mut u8 {
        unsafe {
            self.buf
                .as_mut_ptr()
                .offset(self.valid_slice.start as isize)
        }
    }

    /// Split this view of the underlying buffer into two at the given index.
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
            // Remove our old `valid_slice`:
            self.buf.remove_exclusive_slice(&self.valid_slice).unwrap();

            // Create and request a new `valid_slice` for the new `AlignedBufferMut`:
            let new_valid_slice = self.valid_slice.start..idx;
            self.buf
                .request_new_exclusive_slice(&new_valid_slice)
                .unwrap();

            // Request a new `valid_slice` for `self`:
            self.valid_slice.start = idx;
            self.buf
                .request_new_exclusive_slice(&self.valid_slice)
                .unwrap();

            Ok(AlignedBufferMut {
                buf: self.buf.clone(),
                valid_slice: new_valid_slice,
            })
        }
    }

    /// If this is the only `AlignedBufferMut` with access to the underlying buffer
    /// then `freeze` returns a read-only `AlignedBuffer` (wrapped in `Ok`), which contains a
    /// reference to the underlying buffer, and has its `valid_slice` set to the entire byte range
    /// of the underlying buffer. If, on the other hand, other `AlignedBufferMut`s have access to
    /// the underlying then `freeze` will return `Err(self)`.
    pub(crate) fn freeze(self) -> Result<AlignedBuffer, Self> {
        if Arc::strong_count(&self.buf) == 1 {
            Ok(AlignedBuffer {
                buf: self.buf.clone(),
                valid_slice: 0..self.capacity(),
            })
        } else {
            Err(self)
        }
    }
}

impl Drop for AlignedBufferMut {
    fn drop(&mut self) {
        self.buf.remove_exclusive_slice(&self.valid_slice).unwrap();
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
    /// A list of the slices which are currently mutably borrowed.
    //
    // TODO: Actually, I think we can completely remove `exclusive_slices` and all the methods
    // associated with it! The only way of getting a new `AlignedBufferMut` is by splitting it,
    // which (should!) guarantee that we only ever get non-overlapping mutable views.
    //
    // TODO: To improve performance of finding a `Range` within this set, we might want to use
    // `BTreeSet`. But `BTreeSet` requires `Ord`, and `Ord` isn't implemented for `Range`, so we'd
    // have to define a new range type which holds a `Range` and implements `Ord`, or perhaps have
    // two `BTreeSet<usize>`s, one for the start of each range, and one for the end of each range.
    exclusive_slices: Mutex<HashSet<Range<usize>>>,
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
        Self {
            buf,
            layout,
            exclusive_slices: Mutex::new(HashSet::new()),
        }
    }

    /// Attempt to claim a new, exclusive slice. This will return an Error if the requested slice
    /// overlaps with an existing slice.
    fn request_new_exclusive_slice(&self, slice: &Range<usize>) -> anyhow::Result<()> {
        let mut exclusive_slices = self.exclusive_slices.lock().unwrap();
        for exclusive_slice in exclusive_slices.iter() {
            if overlaps(exclusive_slice, slice) {
                return Err(anyhow::format_err!("The requested slice {slice:?} overlaps with an existing slice: {exclusive_slice:?}"));
            }
        }
        exclusive_slices.insert(slice.clone());
        Ok(())
    }

    fn remove_exclusive_slice(&self, slice: &Range<usize>) -> anyhow::Result<()> {
        let slice_was_in_set = self.exclusive_slices.lock().unwrap().remove(slice);
        if slice_was_in_set {
            return Ok(());
        } else {
            return Err(anyhow::format_err!("Slice {slice:?} was not in the set!"));
        }
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

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a.contains(&b.start) || a.contains(&(b.end - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlaps() {
        // Ranges which don't overlap
        for (a, b) in vec![((0..5), (5..10)), ((0..1), (100..200)), ((1..1), (0..5))] {
            assert!(!overlaps(&a, &b));
            assert!(!overlaps(&b, &a));
        }

        // Ranges which do overlap
        for (a, b) in vec![((0..5), (4..5)), ((0..5), (0..5))] {
            assert!(overlaps(&a, &b), "overlaps({a:?}, {b:?})");
            assert!(overlaps(&b, &a), "overlaps({b:?}, {a:?})");
        }
    }

    #[test]
    fn test_write_and_read() {
        // Create a new buffer:
        const LEN: usize = 16;
        let mut aligned_buf1 = AlignedBufferMut::new(0..LEN, 8);
        let mut aligned_buf2 = AlignedBufferMut::new(0..LEN, 8);

        // Set the values of the buffer:
        {
            let ptr1 = aligned_buf1.as_mut_ptr_to_valid_slice();
            let ptr2 = aligned_buf2.as_mut_ptr_to_valid_slice();
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
