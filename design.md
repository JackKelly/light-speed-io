# Draft design for `light-speed-io`

## Planned features

`light-speed-io` (or "LSIO", for short) will be a Rust library crate for loading and processing many chunks of files, as fast as the storage system will allow.


- [ ] Provide a simple, async API for reading many chunks of files (and/or many files) with single API call. Users will be able to ask LSIO: "_Please get me these million file chunks, and apply this function to each chunk. Tell me when you're done._".
- [ ] The API will be the same, no matter which operating system you're on, and no matter whether the data is on local disk, or a cloud storage bucket, or available over HTTP. (Hat tip to [fsspec](https://filesystem-spec.readthedocs.io/en/latest/) :smiley:!)
- [ ] Laser-focus on _speed_:
  - Achieve many [input/output operations per second](https://en.wikipedia.org/wiki/IOPS) (IOPS), high bandwidth, and low latency by exploiting "modern" operating system storage APIs, and designing for inherently parallel storage systems like NVMe SSDs and cloud storage buckets.
  - Before submitting any IO operations, tune the sequence of IO operations according to the performance characteristics of each storage system. For example, on a storage system that uses hard drives (with spinning platters), the performance of random reads is dominated by the time taken to move the read head. So merge nearby reads, even if those reads aren't strictly consecutive. Or, when reading large files from a cloud storage bucket, it may be faster to split each file into multiple chunks, and request those chunks in parallel.
  - Exploit CPU caches: Minimise the number of round-trips to RAM. Once a chunk is loaded into CPU cache, perform all transformations on that chunk in quick succession, and pin the computation to a single CPU core per chunk.
  - Use multiple CPU cores in parallel.
  - When scheduling work across multiple CPU cores: Avoid locks, or any synchronization primitives that would block a CPU core, wherever possible.
  - Look for opportunities to completely cut the CPU out of the data path. For example, if we're loading uncompressed [Zarr](https://zarr.dev/) chunks that are destined to be merged into a final numpy array, then we may be able to use DMA to directly copy chunks into the final numpy array, without the CPU ever touching the data. This may be possible even in cases where the creation of the final array is more complicated than simply concatenating the chunks in RAM.
  - Where appropriate, align chunks in RAM (and pad the ends of chunks) so the CPU & compiler can easily use SIMD instructions. (SIMD registers may be useful "just" for memory copies).
- [ ] The user-supplied function that's applied to each chunk could include, for example, decompression, followed by some numerical transformation, followed by copying the transformed data to a large array which is the concatenation of all the chunks. As much of this as possible should happen in the CPU cache (without time-consuming round-trips to RAM)
- [ ] Implement multiple IO backends. Each backend will exploit the performance features of a particular operating system and storage system. The ambition is to support:
    - These operating systems:
        - [ ] Linux [io_uring](https://en.wikipedia.org/wiki/Io_uring) (for local storage and network storage).
        - [ ] Windows [I/O Ring](https://windows-internals.com/i-o-rings-when-one-i-o-operation-is-not-enough/).
        - [ ] MacOS X [kqueue](https://en.wikipedia.org/wiki/Kqueue).
    - These storage systems:
        - [ ] Local disks.
        - [ ] Cloud storage buckets.
        - [ ] HTTP.
- [ ] Provide an async Rust API
- [ ] Provide an async Python API.