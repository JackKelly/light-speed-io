A memory buffer allocated on the heap.

The start position and end position of the backing buffer are both aligned in memory. The user
specifies the memory alignment at runtime. This is useful for working with `O_DIRECT` file IO,
where the filesystem will often expect the buffer to be aligned to the logical block size of
the filesystem[^o_direct] (typically 512 bytes).

The API is loosely inspired by the [`bytes`](https://docs.rs/bytes/latest/bytes/index.html) crate.
To give a very quick overview of the `bytes` crate: The `bytes` crate has an (immutable)
[`Bytes`](https://docs.rs/bytes/latest/bytes/struct.Bytes.html) struct and a (mutable)
[`BytesMut`](https://docs.rs/bytes/latest/bytes/struct.BytesMut.html) struct. `BytesMut` can be
`split` into multiple non-overlapping owned views of the same backing buffer. The backing
buffer is dropped when all the views referencing that buffer are dropped. `BytesMut` can be
[frozen](https://docs.rs/bytes/latest/bytes/struct.BytesMut.html#method.freeze) to produce an
(immutable) `Bytes` struct which, in turn, can be sliced to produce (potentially overlapping)
owned views of the same backing buffer.

`aligned_bytes` follows a similar pattern:

[`AlignedBytesMut`] can be [`AlignedBytesMut::split_to`] to produce multiple non-overlapping mutable
views of the same backing buffer without copying the memory (each `AlignedBytesMut` has its own
`range` (which represents the byte range that this `AlignedBytesMut` has exclusive access to)
and an `Arc<InnerBuffer>`). The splitting process guarantees that views cannot overlap, so we
do not have to use locks, whilst allowing multiple threads to write to (non-overlapping regions
of) the same buffer.

When you have finished writing into the buffer, drop all but one of the `AlignedBytesMut`
objects, and call [`AlignedBytesMut::freeze`] on the last `AlignedByteMut`. This will
consume the `AlignedBytesMut` and return an (immutable) [`AlignedBytes`]. Then you
can `clone` and [`AlignedBytes::set_slice`] to get (potentially overlapping) owned views of the
same backing buffer.

The backing buffer will be dropped when all views into the backing buffer are dropped.

Unlike `bytes`, `aligned_bytes` does not use a `vtable`, nor does it allow users to grow the
backing buffers. `aligned_bytes` implements the minimal set of features required for the rest
of the LSIO project! In fact, `aligned_bytes` is _so_ minimal that the only way to write data
into an `AlignedBytesMut` is via [`AlignedBytesMut::as_mut_ptr`] (because that's what the
operating system expects!)

# Examples and use-cases

**Use case 1: The user requests multiple contiguous byte ranges from LSIO.**

Let's say the user requests two byte ranges from a single file: `0..4096`, and `4096..8192`.

Under the hood, LSIO will:

- Notice that these two byte ranges are consecutive, and merge these two byte ranges into a
  single read operation.
- Allocate a single 8,192 byte `AlignedBytesMut`, aligned to 512-bytes.
- Submit a `read` operation to `io_uring` for all 8,192 bytes.
- When the single read op completes, we `freeze` the buffer, which consumes the
  `AlignedBytesMut` and returns an `AlignedBytes`, which we then `reset_slice()` to view the
  entire 8,192 backing buffer.
- Split the `AlignedBytes` into two owned `AlignedBytes`, and return these to the user.
- The underlying buffer will be dropped when the user drops the two `AlignedBytes`.

Here's a code sketch to show how this works:

```rust
use lsio_aligned_bytes::AlignedBytesMut;

// Allocate a single 8,192 byte `AlignedBytesMut`:
const LEN: usize = 8_192;
const ALIGN: usize = 512;
let mut bytes = AlignedBytesMut::new(LEN, ALIGN);

// Write into the buffer. (In this toy example, we'll write directly into the buffer.
// But in "real" code, we'd pass the pointer to the operating system, which in turn
// would write data into the buffer for us.)
let ptr = bytes.as_mut_ptr();
for i in 0..LEN {
    unsafe { *ptr.offset(i as isize) = i as u8; }
}

// Freeze (to get a read-only `AlignedBytes`). We `unwrap` because `freeze`
// will fail if there's more than one `AlignedBytesMut` referencing our backing buffer.
let mut bytes = bytes.freeze().unwrap();
bytes.reset_slice();
let expected_byte_string: Vec<u8> = (0..LEN).map(|i| i as u8).collect();
assert_eq!(bytes.as_slice(), expected_byte_string);

// Slice the buffer into two new buffers:
let mut buffer_0 = bytes.clone();
buffer_0.set_slice(0..4_096);
let mut buffer_1 = bytes.clone();
buffer_1.set_slice(4_096..8_192);
assert_eq!(buffer_0.len(), 4_096);
assert_eq!(buffer_1.len(), 4_096);
assert_eq!(buffer_0.as_slice(), &expected_byte_string[0..4_096]);
assert_eq!(buffer_1.as_slice(), &expected_byte_string[4_096..8_192]);

// Check that the original `bytes` buffer is still valid:
assert_eq!(bytes.as_slice(), &expected_byte_string);

// Remove the original `bytes` and check that the two views of the same buffer
// are still valid:
drop(bytes);
assert_eq!(buffer_0.as_slice(), &expected_byte_string[0..4_096]);
assert_eq!(buffer_1.as_slice(), &expected_byte_string[4_096..8_192]);
```

**Use-case 2: The user requests a single 8 GiB file.**

Linux can't read more than 2 GiB at once[^linux_read]. So we need to read the 8 GiB files in
multiple chunks.

LSIO will:
- Allocate a single 8 GiB `AlignedBytesMut`.
- Split this into a new 2 GiB `AlignedBytesMut` and the old `AlignedBytesMut` is reduced to 6 GiB.
  Both of these buffers must have their starts and ends aligned. Then repeat the process to
  get a total of 4 x 2 GiB `AlignedBytesMut`s.
- Issue four `read` operations to the OS (one operation per `AlignedBytesMut`).
- When the first, second, and third `read` ops complete, drop their `AlignedBytesMut`
  (but that won't drop the underlying storage, it just removes its reference).
- When the last `read` op completes, `freeze` the last `AlignedBytesMut` to get an immutable `AlignedBytes`.
  `reset_slice` to get the 8 GB slice requested by the user. Pass this 8 GiB `AlignedBytes` to the user.

```rust
use lsio_aligned_bytes::AlignedBytesMut;

// Allocate a single array (for this toy example, we'll just allocate 8 MiB, instead of 8 GiB!)
const MiB: usize = 2_usize.pow(20);
const LEN: usize = 8 * MiB;
const ALIGN: usize = 512;
let mut bytes_3 = AlignedBytesMut::new(LEN, ALIGN);
// `bytes_3` will be the final of four bytes_<n> arrays!

// Split into a 2 MiB buffer, and a 6 MiB buffer:
let mut bytes_0 = bytes_3.split_to(2 * MiB).unwrap();
assert_eq!(bytes_0.len(), 2 * MiB);
assert_eq!(bytes_3.len(), 6 * MiB);

// Continue splitting:
let mut bytes_1 = bytes_3.split_to(4 * MiB).unwrap();
let mut bytes_2 = bytes_3.split_to(6 * MiB).unwrap();
assert_eq!(bytes_0.len(), 2 * MiB);
assert_eq!(bytes_1.len(), 2 * MiB);
assert_eq!(bytes_2.len(), 2 * MiB);
assert_eq!(bytes_3.len(), 2 * MiB);

// Write into the arrays:
// Fill the first 2 MiB with zeros, fill the second 2 MiB with ones, etc.
for i in 0..(2 * MiB) {
    unsafe {
        *bytes_0.as_mut_ptr().offset(i as isize) = 0;
        *bytes_1.as_mut_ptr().offset(i as isize) = 1;
        *bytes_2.as_mut_ptr().offset(i as isize) = 2;
        *bytes_3.as_mut_ptr().offset(i as isize) = 3;
    }
}

// Drop three of the four AlignedBytesMuts, in preparation for freezing:
drop(bytes_0);
drop(bytes_1);
drop(bytes_2);

// Needs to be `mut` so we can `reset_slice()`. Doesn't actually mutate the buffer!
let mut bytes = bytes_3.freeze().unwrap();
bytes.reset_slice();

let expected: Vec<u8> = (0..LEN).map(|i| (i / (2 * MiB)) as u8).collect();
// We use `Iterator::eq` instead of `assert_eq!` to avoid `assert_eq!` printing out
// 16 million numbers if the arrays aren't exactly equal!
assert!(bytes.as_slice().iter().eq(expected.iter()));

```

[^o_direct]: For more information on `O_DIRECT`, including the memory alignment requirements,
  see all the mentions of `O_DIRECT` in the [`open(2)`](https://man7.org/linux/man-pages/man2/open.2.html) man page.
[^linux_read]: Actually, the limit isn't exactly 2 GiB. On Linux, `read` will transfer at most
  2,147,479,552 bytes. See the [`read`](https://man7.org/linux/man-pages/man2/read.2.html) man
  page!

