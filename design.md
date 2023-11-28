# Draft design for `light-speed-io` (LSIO)

`light-speed-io` (or "LSIO", for short) will be a Rust library crate for loading and processing many chunks of files, as fast as the storage system will allow.

## Planned features

- [ ] Provide a simple, async API for reading many chunks of files (and/or many files) with single API call. Users will be able to ask LSIO: "_Please get me these million file chunks, and apply this function to each chunk. Tell me when you're done._".
- [ ] The API will be the same, no matter which operating system you're on, and no matter whether the data is on local disk, or a cloud storage bucket, or available over HTTP. (Inspired by [fsspec](https://filesystem-spec.readthedocs.io/en/latest/) :smiley:!)
- [ ] Laser-focus on _speed_:
  - Achieve many [input/output operations per second](https://en.wikipedia.org/wiki/IOPS) (IOPS), high bandwidth, and low latency by exploiting "modern" operating system storage APIs, and designing for inherently parallel storage systems like NVMe SSDs and cloud storage buckets.
  - Before submitting any IO operations, tune the sequence of IO operations according to the performance characteristics of each storage system. For example, on a hard drive (with spinning platters), the performance of random reads is dominated by the time taken to move the read head. So LSIO will merge nearby reads, even if those reads aren't strictly consecutive: For example, if we want to read every third block of a file, it may be faster to read the entire file, even if we immediately throw away two thirds of the data. Or, when reading large files from a cloud storage bucket, it may be faster to split each file into consecutive chunks, and request those chunks in parallel.
  - "Auto-tune" to each storage system. Or, if users does not want to auto-tune, then provide sane defaults for a range of common storage systems.
  - Exploit CPU caches and hence minimize the number of time-consuming reads from RAM. Once a chunk is loaded into CPU cache, perform all transformations on that chunk in quick succession (to maximize the chance that the data stays in cache), and pin the computation for a given chunk to a single CPU core (because level-1 CPU cache is specific to a CPU core).
  - Use multiple CPU cores in parallel (each working on a different chunk).
  - When scheduling work across multiple CPU cores: Avoid locks, or any synchronization primitives that would block a CPU core, wherever possible.
  - Look for opportunities to completely cut the CPU out of the data path. For example, if we're loading uncompressed [Zarr](https://zarr.dev/) chunks that are destined to be merged into a final numpy array, then we may be able to use [direct memory access](https://en.wikipedia.org/wiki/Direct_memory_access) (DMA) to directly copy chunks into the final numpy array from IO, without the CPU ever touching the data. This may be possible even in cases where the creation of the final array is more complicated than simply concatenating the chunks in RAM.
  - Where appropriate, align chunks in RAM (and pad the ends of chunks) so the CPU & compiler can easily use SIMD instructions, and minimize the number of cache lines that must be read. (Using SIMD may provide a large speedup "just" for memory copies, even if the transform function doesn't use SIMD).
- [ ] The user-supplied function that's applied to each chunk could include, for example, decompression, followed by some numerical transformation, followed by copying the transformed data to a large array which is the concatenation of all the chunks. As much of this as possible should happen whilst the chunk is in the CPU cache (without time-consuming round-trips to RAM).
- [ ] LSIO will implement multiple IO backends. Each backend will exploit the performance features of a particular operating system and storage system. The ambition is to support:
    - These operating system APIs:
        - [ ] Linux [io_uring](https://en.wikipedia.org/wiki/Io_uring) (for local storage and network storage).
        - [ ] Windows [I/O Ring](https://windows-internals.com/i-o-rings-when-one-i-o-operation-is-not-enough/).
        - [ ] MacOS X [kqueue](https://en.wikipedia.org/wiki/Kqueue).
    - These storage systems:
        - [ ] Local disks. (With different optimizations for SSDs and HDDs).
        - [ ] Cloud storage buckets.
        - [ ] HTTP.
- [ ] Async Rust API.
- [ ] Async Python API.

## Use cases

Allow for very fast access to arbitrary selections of:
* Multi-dimensional [Zarr](https://zarr.dev/) arrays. Jack is mostly focused on [_sharded_ Zarr arrays](https://zarr.dev/zeps/accepted/ZEP0002.html). But LSIO could also be helpful for non-sharded Zarr arrays.
    * Jack is particularly focused on speeding up the data pipeline for training machine learning models on multi-dimensional datasets, where we want to select hundreds of random crops of data per second. This is described below in the [Priorities](#priorities) section. The ambition is to enable us to read on the order of 1 million Zarr chunks per second (from a fast, local SSD).
* Other file formats used for multi-dimensional arrays, such as NetCDF, GRIB, and EUMETSAT's native file format. (LSIO could help to speed up [kerchunk](https://fsspec.github.io/kerchunk/))

## Priorities

Jack's main hypothesis is that it _should_ be possible to train large machine learning (ML) models _directly_ from multi-dimensional data stored on disk as Zarr arrays, instead of having to prepare ML training batches ahead of time. These ML models require random crops to be selected from multi-dimensional datasets, at several gigabytes per second. (See [Jack's blog post](https://jack-kelly.com/blog/2023-07-28-speeding-up-zarr) for more details. An example multi-dimensional dataset is satellite imagery over time.)

(And, even more ambitiously, LSIO may allow us to train directly from the _original data_ stored in, for example, GRIB files). 

The ultimate test is: Can LSIO enable us to train ML models directly from Zarr? (whilst ensuring that the GPU is constantly at near 100% utilization). So, Jack's priority will be to implement just enough of LSIO to enable us to test this hypothesis empirically: and that means implementing just one IO backend (io_uring for local files), to start with.

If this provides a significant speed-up, then Jack will focus on implementing reading from Google Cloud Storage buckets, maybe using io_uring for async network IO.

On the other hand, if LSIO does _not_ provide a speed-up, then - to be frank - LSIO will probably be abandoned!

## Timeline

Ha! :smiley:. This project is in the earliest planning stages! It'll be _months_ before it does anything vaguely useful! And, for now at least, this project is just Jack hacking away his spare time, whilst learning Rust!

## Design

TODO! (But, for now, see the file [`src/draft_API_design.rs` in this pull request](https://github.com/JackKelly/light-speed-io/blob/draft-API-design/src/draft_API_design.rs))

### Public Rust API

#### Describe the performance characteristics of the storage subsystem

First, the user must describe the performance characteristics of their storage subsystem. This can be done using pre-defined defaults, or auto calibration, or manually specifying options, or loading from disk (using [`serde`](https://serde.rs/)). This information will be used by LSIO to optimize the sequence of chunks for the user's storage system, prior to submitting IO operations to the hardware. The user's code would look like this:

##### User code

```rust
let config = SSD_NVME_PCIE_GEN4;

// Or do this :)
let config = IoConfig::auto_calibrate();
```

##### Under the hood (in LSIO)

```rust
/// Describe the performance characteristics of the storage subsystem
pub struct IoConfig {
    pub latency_millisecs: f64,
    pub bandwidth_megabytes_per_sec: f64,

    /// Files larger than this will be broken into consecutive chunks,
    /// and the chunks will be requested concurrently.
    /// Breaking up files may speed up reading from cloud storage buckets.
    /// Each chunk will be no larger than this size.
    /// Set this to `None` if you never want to break files apart.
    pub max_megabytes_of_single_read: Option<f64>,
}

impl IoConfig {
    pub fn auto_calibrate() -> Self {
        // TODO
    }
    // Use Serde to save and load IoConfig.
}

/// Default config options for NVMe SSDs using PCIe generation 4.
pub const SSD_NVME_PCIE_GEN4: IoConfig = IoConfig{
    latency_millisecs: 0.001,
    bandwidth_megabytes_per_sec: 8000,
    max_megabytes_of_single_read: None,
};
```

#### Initialize a `Reader` struct

Using a persistent object will allow us to cache (in memory) values such as file sizes. And provides an opportunity to pre-allocated memory buffers (where possible).

##### User code

```rust
let reader = IoUringLocal::new(config);
```

##### Under the hood (in LSIO)

```rust
pub trait Reader {
    pub fn new(config: IoConfig) -> Self { Self {config} }
}

/// Linux io_uring for locally-attached disks.
pub struct IoUringLocal {
    config: IoConfig,

    /// Map from the full file name to the file size in bytes.
    /// We need to know the length of each file if we want to read the file
    /// in its entirety, or if we want to seek to a position relative to the
    /// end of the file.
    cached_file_sizes_in_bytes: map<PathBuf, u64>,
}

impl Reader for IoUringLocal {
    // Implement io_uring-specific stuff...
}
```

#### Specify which chunks to read

##### User code

In this example, we read the entirety of `/foo/bar`, and we also read three chunks from `/foo/baz`:

```rust
let mut buf0 = vec![0; 1000];
let mut buf1 = vec![0;  300];
let mut buf2 = vec![0;  100];

let chunks = vec![

    // Read the entirity of /foo/bar
    FileChunks{
        path: "/foo/bar",
        byte_range: ByteRange::EntireFile, // Read all of file
        buffer: None, // LSIO takes responsibility for allocating a memory buffer
    },

    // Read 3 chunks from /foo/baz
    FileChunks{
        path: "/foo/baz", 
        byte_range: ByteRange::MultiRange(
            vec![
                ..1000,     // Read the first 1,000 bytes
                -500..-200, // Read 300 bytes, until the 200th byte from the end
                -100..,     // Read the last 100 bytes. For example, shared Zarrs store
                            // the shard index at the end of each file.
                ],
        ),

        // If the user wants to supply buffers, then use `Some(Vec<&mut [u8]>)`
        // with one buffer per element in the `byte_range` vector.
        // For example, this would allow us to bypass the CPU when copying multiple
        // uncompressed chunks from a sharded Zarr directly into the final array.
        // The buffers could point to different slices of the final array.
        // This mechanism could even be used when creating the final array is more
        // complicated than simply appending chunks: you could, for example, read each
        // row of each chunk into a different `&mut [u8]`. Under the hood, LSIO would
        // notice the consecutive reads, and would use `readv` where available.
        buffer: Some(
            vec![
                &mut buf0,
                &mut buf1,
                &mut buf2,
            ]
        )
    },
];
```

##### Under the hood (in LSIO)

```rust
pub struct FileChunks {
    pub path: Path,
    pub byte_range: ByteRange,

    // If buffer is None, then LSIO will take responsibility for allocating
    // the memory buffers. This should be the preferred approach.
    pub buffer: Option<Vec<&mut [u8]>>,
}

pub enum ByteRange {
    EntireFile,
    MultiRange(Vec<Range>),
}
```

#### Async reading of chunks

##### User code

```rust
// Start async loading of data from disk:
let future = reader.read_chunks(&chunks);

// Wait for data to all be ready.
// We need one `Result` per chunk, because reading each chunk could fail.
// Note that we take ownership of the returned vectors of bytes.
let data: Vec<Result<Vec<u8>>> = future.wait();
```

Or, if we want to apply a function to each chunk, we could do something like this. This example
is based on the Zarr use-case. For each chunk, want to decompress, and apply a simple numerical
transformation, and then move the transformed data into a final array:

```rust
let mut final_array = Array();
let chunk_idx_to_array_loc = Vec::new();
// TODO: Fill out `chunk_idx_to_array_loc`

// processing_fn could fail, so we return a Result.
// processing_fn may not return any data (because the data has been moved to another location)
// so we return an Option wrapped in a Result.
let processing_fn = |chunk_idx: u64, chunk: &[u8]| -> Result<Option<&[u8]>> {
    // ******** DECOMPRESS ************
    // If we don't know the size of the uncompressed chunk, then 
    // deliberately over-allocate, and shrink later...
    const OVER_ALLOCATION_RATIO: usize = 4;
    let mut decompressed_chunk = Vec::with_capacity(OVER_ALLOCATION_RATIO * chunk.size());
    decompress(&chunk, &mut decompressed_chunk)?;
    decompressed_chunk.shrink_to_fit();

    // ******** PROCESS ***********
    decompressed_chunk = decompressed_chunk / 2;  // to give a very simple example!

    // ******** COPY TO FINAL ARRAY **************
    final_array[chunk_idx_to_array_loc[chunk_idx]] = decompressed_chunk;
    Ok(None)  // We're deliberately not passing back the decompressed array.
};
let future = read.read_chunks_and_apply(&chunks, processing_fn);
let results = future.wait();
// TODO: check `results` for any failures
pass_to_python(&final_array);
```

### Internal design of LSIO

TODO. Things to consider:

Within LSIO, the pipeline for the IO ops is something like this:

- User submits a Vector of `FileChunks`.
- In the main thread:
    - We need to get the file size for:
        - any `EntireFiles`. If these exist, then we need to get the file size ahead-of-time, so we can pre-allocate a memory buffer.
        - any `MultiRange`s which include offsets from the end of the file, iff the backend doesn't natively support offsets from the end of the file (or maybe this should be the backend's problem? Although I'd guess it'd be faster to get all file sizes in one go, ahead of time?)
    - For any `MultiRange`s, LSIO optimizes the sequence of ranges. This is dependent on `IoConfig`, but shouldn't be dependent on the IO backend. Maybe this could be implemented as a set of methods on `ByteRange`?
        - Merge any overlapping read requests (e.g. if the user requests `[..1000, ..100]` then only actually read `..1000`, but return - as the second chunk - a memory copy of the last 100 bytes). Maybe don't implement this in the MVP. But check that the design can support this.
        - Merge nearby reads into smaller reads, depending on `IoConfig`.
        - Split large reads into multiple smaller reads, depending on `IoConfig.max_megabytes_of_single_read`.
        - Detect contiguous chunks, and use `readv`. (Although we should benchmark `readv` vs `read`).
        - Perhaps we need a new type for the _optimized_ byte ranges? We need to express:
        - "_this single optimized read started life as multiple, nearby reads. After performing this single read, the memory buffer will need to be split into sub-chunks, and those sub-chunks processed in parallel. And we may want to throw away some data. The IO backend should be encouraged to use `readv` if available, to directly read into multiple buffers. (POSIX can use `readv` to read sockets as well as files.)_"
        - "_this single optimized read started life as n multiple, overlapping reads. The user is expecting n slices (views) of this memory buffer_"
        - "_these multiple optimized reads started life as a single read request. Create one large memory buffer. And each sub-chunk should be read directly into a different slice of the memory buffer._"
        - Maybe the answer is that the optimization step should be responsible for allocating the memory buffers, and it just submits a sequence of abstracted `readv` operations to the IO backend? If the backend can't natively perform `readv` then it's trivial for the backend to split one `readv` call into multiple `read`s. But! We should only allocate memory buffers when we're actually ready to read! Say we want to read 1,000,000 chunks. Using io_uring, we won't actually submit all 1,000,000 read requests at once: instead we'll keep the submission ring topped up with, say, 64 tasks. If the user wants all 1,000,000 chunks returned then we have no option but to allocate all 1,000,000 chunks. But if, instead, the user wants each chunk to be processed and then moved into a common final array, then we only have to allocate 64 buffers per thread.
    - Pass this optimized sequence to the IO backend (e.g. `IoUringLocal`).
- For `IoUringLocal`, the main thread spins up _n_ io_uring rings, and _n_ worker threads (where _n_ defaults to the number of logical CPU cores, or the number of requested read ops, which ever is smaller - there's no point spinning up 32 threads if we only have 2 read operations!). Each worker thread gets its own completion ring. The main thread is responsible for submitting operations to all _n_ submission rings. The worker threads all write to a single, shared channel, to say when they've completed a task, which tells the main thread to submit another task to that thread's submission queue. This design should be faster than the main thread creating single queue of tasks, which each worker thread reads from. Queues block. Blocking is bad!
    - The main thread:
        - Starts by splitting all the operations into _n_ lists. For example, if we start with 1,000,000 read operations, and have 10 CPU cores, then we end up with 100,000 read ops per CPU core.
        - But we don't want to simply submit all 100,000 ops to each submission queue, in one go. That doesn't give us the opportunity to balance work across the worker threads. (Some read ops might take longer than others.) And, we can't use that many filehandles per process!
        - Allocate filehandles for each read op in flight. (So io_uring can chain open(fh), read(fh), close(fh)).
        - Submit the first, say, 64 read ops to each thread's submission queue. (Each "read op" would actually be a chain of open, read, close).
        - Block on `channel.recv()`. When a message arrives, submit another task to that thread's submission ring.
    - Each worker thread:
        - Blocks waiting for data from its io_uring completion queue.
        - When data arrives, it checks for errors, and performs the requested processing.
        - The worker thread ends its ID to the channel, to signal that it has completed a task.
    - BUT! In cases where the user has not requested any processing, then the worker threads are redundant??? Maybe we simply don't spin up any worker threads, in that case? Although, actually, we still need to check each completion queue entry for errors, I think? Maybe threads would be useful for that??? And, for the MVP, maybe we should always spin up threads, so we don't have to worry about a separate code path for the "no processing" case?
