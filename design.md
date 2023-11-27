# Draft design for `light-speed-io` (LSIO)

`light-speed-io` (or "LSIO", for short) will be a Rust library crate for loading and processing many chunks of files, as fast as the storage system will allow.

## Planned features

- [ ] Provide a simple, async API for reading many chunks of files (and/or many files) with single API call. Users will be able to ask LSIO: "_Please get me these million file chunks, and apply this function to each chunk. Tell me when you're done._".
- [ ] The API will be the same, no matter which operating system you're on, and no matter whether the data is on local disk, or a cloud storage bucket, or available over HTTP. (Hat tip to [fsspec](https://filesystem-spec.readthedocs.io/en/latest/) :smiley:!)
- [ ] Laser-focus on _speed_:
  - Achieve many [input/output operations per second](https://en.wikipedia.org/wiki/IOPS) (IOPS), high bandwidth, and low latency by exploiting "modern" operating system storage APIs, and designing for inherently parallel storage systems like NVMe SSDs and cloud storage buckets.
  - Before submitting any IO operations, tune the sequence of IO operations according to the performance characteristics of each storage system. For example, on a hard drive (with spinning platters), the performance of random reads is dominated by the time taken to move the read head. So LSIO will merge nearby reads, even if those reads aren't strictly consecutive: For example, if we want to read every third block of a file, it may be faster to read the entire file, even if we immediately throw away two thirds of the data. Or, when reading large files from a cloud storage bucket, it may be faster to split each file into consecutive chunks, and request those chunks in parallel.
  - "Auto-tune" to each storage system. Or, if users does not want to auto-tune, then provide sane defaults for a range of common storage systems.
  - Exploit CPU caches and hence minimize the number of time-consuming reads from RAM. Once a chunk is loaded into CPU cache, perform all transformations on that chunk in quick succession, and pin the computation for a given chunk to a single CPU core.
  - Use multiple CPU cores in parallel (each working on a different chunk).
  - When scheduling work across multiple CPU cores: Avoid locks, or any synchronization primitives that would block a CPU core, wherever possible.
  - Look for opportunities to completely cut the CPU out of the data path. For example, if we're loading uncompressed [Zarr](https://zarr.dev/) chunks that are destined to be merged into a final numpy array, then we may be able to use DMA to directly copy chunks into the final numpy array, without the CPU ever touching the data. This may be possible even in cases where the creation of the final array is more complicated than simply concatenating the chunks in RAM.
  - Where appropriate, align chunks in RAM (and pad the ends of chunks) so the CPU & compiler can easily use SIMD instructions, and minimize the number of cache lines that must be read (using SIMD may provide a large speedup "just" for memory copies, even if the transform function doesn't use SIMD).
- [ ] The user-supplied function that's applied to each chunk could include, for example, decompression, followed by some numerical transformation, followed by copying the transformed data to a large array which is the concatenation of all the chunks. As much of this as possible should happen in the CPU cache (without time-consuming round-trips to RAM).
- [ ] LSIO will implement multiple IO backends. Each backend will exploit the performance features of a particular operating system and storage system. The ambition is to support:
    - These operating system APIs:
        - [ ] Linux [io_uring](https://en.wikipedia.org/wiki/Io_uring) (for local storage and network storage).
        - [ ] Windows [I/O Ring](https://windows-internals.com/i-o-rings-when-one-i-o-operation-is-not-enough/).
        - [ ] MacOS X [kqueue](https://en.wikipedia.org/wiki/Kqueue).
    - These storage systems:
        - [ ] Local disks.
        - [ ] Cloud storage buckets.
        - [ ] HTTP.
- [ ] Provide an async Rust API
- [ ] Provide an async Python API.

## Use cases

Allow for very fast access to:
* [Zarr](https://zarr.dev/) arrays. Jack is mostly focused on [sharded Zarr arrays](https://zarr.dev/zeps/accepted/ZEP0002.html). But LSIO could also be helpful for non-sharded Zarr arrays.
* Other file formats used for multi-dimensional arrays, such as NetCDF, GRIB, and EUMETSAT's native file format. (LSIO could help to speed up [kerchunk](https://fsspec.github.io/kerchunk/))

## Timeline

Ha! :smiley:. This project is in the earliest planning stages! It'll be _months_ before it does anything vaguely useful! And, for now at least, this project is just Jack hacking away his spare time, whilst learning Rust!

## Design

TODO! (But, for now, see the file [`src/draft_API_design.rs` in this pull request](https://github.com/JackKelly/light-speed-io/blob/draft-API-design/src/draft_API_design.rs))