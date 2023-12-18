# Draft design for `light-speed-io` (LSIO)

`light-speed-io` (or "LSIO", for short) will be a Rust library crate for loading and processing many chunks of files, as fast as the storage system will allow.

## Planned features

- [ ] Provide a simple API (using Rust's iterators) for reading many chunks of files (and/or many files) with single API call. Users will be able to ask LSIO: "_Please get me these million file chunks, and apply this function to each chunk, and then move the resulting data to these array locations._".
- [ ] The API will be the same, no matter which operating system you're on, and no matter whether the data is on local disk, or a cloud storage bucket, or available over HTTP. (Inspired by [fsspec](https://filesystem-spec.readthedocs.io/en/latest/) :smiley:!)
- [ ] Expose a Rust API and a Python API.
- [ ] Cache compressed chunks in RAM (configurable).
- [ ] Laser-focus on _speed_:
  - Achieve many [input/output operations per second](https://en.wikipedia.org/wiki/IOPS) (IOPS), high bandwidth, and low latency by exploiting "modern" operating system storage APIs, and designing for inherently parallel storage systems like NVMe SSDs and cloud storage buckets.
  - Before submitting any IO operations, tune the sequence of IO operations according to the performance characteristics of each storage system. For example, on a hard drive (with spinning platters), the performance of random reads is dominated by the time taken to move the read head. So LSIO will merge nearby reads, even if those reads aren't strictly consecutive: For example, if we want to read every third block of a file, it may be faster to read the entire file, even if we immediately throw away two thirds of the data. Or, when reading large files from a cloud storage bucket, it may be faster to split each file into consecutive chunks, and request those chunks in parallel.
  - "Auto-tune" to each storage system. Or, if users do not want to auto-tune, then provide sane defaults for a range of common storage systems.
  - Exploit CPU caches and hence minimize the number of time-consuming reads from RAM. Once a chunk is loaded into CPU cache, perform all transformations on that chunk in quick succession (to maximize the chance that the data stays in cache), and pin the computation for a given chunk to a single CPU core (because level-1 CPU cache is specific to a CPU core).
  - Use multiple CPU cores in parallel (each working on a different chunk).
  - When scheduling work across multiple CPU cores: Avoid locks, or any synchronization primitives that would block a CPU core, wherever possible.
  - Look for opportunities to completely cut the CPU out of the data path. For example, if we're loading uncompressed [Zarr](https://zarr.dev/) chunks that are destined to be merged into a final numpy array, then we may be able to use [direct memory access](https://en.wikipedia.org/wiki/Direct_memory_access) (DMA) to directly copy chunks into the final numpy array from IO, without the CPU ever touching the data. This may be possible even in cases where the creation of the final array is more complicated than simply concatenating the chunks in RAM.
  - Where appropriate, align chunks in RAM (and pad the ends of chunks) so the CPU & compiler can easily use SIMD instructions, and minimize the number of cache lines that must be read. (Using SIMD may provide a large speedup "just" for memory copies, even if the transform function doesn't use SIMD).
- [ ] For each chunk, the user could request, for example, that the chunk be decompressed, followed by some numerical transformation, followed by moving the transformed data to a large array which is the concatenation of all the chunks. As much of this as possible should happen whilst the chunk is in the CPU cache (without time-consuming round-trips to RAM).
- [ ] LSIO will implement multiple IO backends. Each backend will exploit the performance features of a particular operating system and storage system. The ambition is to support:
    - These operating system APIs:
        - [ ] Linux [io_uring](https://en.wikipedia.org/wiki/Io_uring) (for local storage and network storage).
        - [ ] Windows [I/O Ring](https://windows-internals.com/i-o-rings-when-one-i-o-operation-is-not-enough/).
        - [ ] MacOS X [kqueue](https://en.wikipedia.org/wiki/Kqueue).
    - These storage systems:
        - [ ] Local disks. (With different optimizations for SSDs and HDDs).
        - [ ] Cloud storage buckets.
        - [ ] HTTP.

## Use cases

Allow for very fast access to arbitrary selections of:
* Multi-dimensional [Zarr](https://zarr.dev/) arrays. Jack is mostly focused on [_sharded_ Zarr arrays](https://zarr.dev/zeps/accepted/ZEP0002.html). But LSIO could also be helpful for non-sharded Zarr arrays.
    * Jack is particularly focused on speeding up the data pipeline for training machine learning models on multi-dimensional datasets, where we want to select hundreds of random crops of data per second. This is described below in the [Priorities](#priorities) section. The ambition is to enable us to read on the order of 1 million Zarr chunks per second (from a fast, local SSD).
* Other file formats used for multi-dimensional arrays, such as NetCDF, GRIB, and EUMETSAT's native file format. (LSIO could help to speed up [kerchunk](https://fsspec.github.io/kerchunk/))

## Priorities

Jack's main hypothesis is that it _should_ be possible to train large machine learning (ML) models _directly_ from multi-dimensional data stored on disk as Zarr arrays, instead of having to prepare ML training batches ahead of time. These ML models require random crops to be selected from multi-dimensional datasets, at several gigabytes per second. (See [Jack's blog post](https://jack-kelly.com/blog/2023-07-28-speeding-up-zarr) for more details. An example multi-dimensional dataset is satellite imagery over time.)

(And, even more ambitiously, LSIO may allow us to train directly from the _original data_ stored in, for example, GRIB files). 

The ultimate test is: Can LSIO enable us to train ML models directly from Zarr? (whilst ensuring that the GPU is constantly at near 100% utilization). So, Jack's _first_ priority will be to implement just enough of LSIO to enable us to test this hypothesis empirically: and that means implementing just one IO backend, to start with. That backend will be io_uring for local files.

If this provides a significant speed-up, then Jack will focus on implementing reading from Google Cloud Storage buckets, maybe using io_uring for async network IO.

On the other hand, if LSIO does _not_ provide a speed-up, then - to be frank - LSIO will probably be abandoned!

## Timeline

Ha! :smiley:. This project is in the earliest planning stages! It'll be _months_ before it does anything vaguely useful! And, for now at least, this project is just Jack hacking away his spare time, whilst learning Rust!

## Design

### Public Rust API

#### Initialize a `Reader` struct

Using a persistent object will allow us to cache (in memory) values such as file sizes. And provides an opportunity to pre-allocated memory buffers (where possible).

##### User code

```rust
let reader = IoUringLocal::new();
```

##### Under the hood (in LSIO)

```rust
pub trait Reader {
    pub fn new() -> Self { Self }
}

/// Linux io_uring for locally-attached disks.
pub struct IoUringLocal {
    /// Map from the full file name to the file size in bytes.
    /// We need to know the length of each file if we want to read the file
    /// in its entirety, or if we want to seek to a position relative to the
    /// end of the file.
    cache_of_file_sizes_in_bytes: Map<PathBuf, u64>,
}

impl Reader for IoUringLocal {
    // Implement io_uring-specific stuff...
}
```

#### Specify which chunks to read

##### User code

In this example, we read the entirety of `/foo/bar`. And we read three chunks from `/foo/baz`:

```rust
let mut buf0 = vec![0; 1000];
let mut buf1 = vec![0;  300];
let mut buf2 = vec![0;  100];

let chunks = vec![

    // Read entirety of /foo/bar, and ask LSIO to allocate the memory buffer:
    FileChunks{
        path: "/foo/bar", 
        chunks: vec![
            Chunk{
                byte_range: ...,
                raw_buffers: None,
                processed_buffers: None,
            },
        ],
    },

    // Read 3 chunks from /foo/baz:
    FileChunks{
        path: "/foo/baz", 
        chunks: vec![
            Chunk{
                byte_range: ..1000,     // Read the first 1,000 bytes
                raw_buffers: Some(vec![&mut buf0]),
                processed_buffers: None,
            },
            Chunk{
                byte_range: -500..-200, // Read 300 bytes, until the 200th byte from the end
                raw_buffers: Some(vec![&mut buf1]),
                processed_buffers: None,
            },
            Chunk{
                byte_range: -100..,                 // Read the last 100 bytes. For example, shared Zarrs
                raw_buffers: Some(vec![&mut buf2]), // place the shard index at the end of each file.
                processed_buffers: None,
            },
        ],
    },

];
```

It is highly recommended that the user only submits _one_ `FileChunks` object per `path`. 
This is because LSIO optimises each `FileChunks` object independently of other `FileChunks`.

##### Under the hood (in LSIO)

```rust
pub struct FileChunks {
    pub path: Path,
    pub chunks: Vec<Chunk>,
}

pub struct Chunk{
    pub byte_range: Range<i64>,

    // Memory buffers for storing the raw data, straight after the data arrives from IO.
    // If raw_buffers is None, then LSIO will take responsibility for allocating the buffers.
    // If the user wants to supply buffers, then use `Some(Vec<&mut [u8]>)`.
    // For example, this would allow us to bypass the CPU when copying multiple
    // uncompressed chunks from a sharded Zarr directly into the final array.
    // The buffers could point to different slices of the final array.
    // This mechanism could be used when creating the final array is more
    // complicated than simply appending chunks: you could, for example, read each
    // row of a chunk into a different `&mut [u8]`. Under the hood, LSIO would
    // notice the consecutive reads, and would use `readv` where available.
    //
    // LSIO borrows a mutable reference to each buffer, so that the user can supply a *slice* to a subset
    // of a larger array. This does mean that caching within LSIO will be a little slower
    // when the user supplies raw_buffers, and memory usage will be larger, because LSIO's
    // cache will have to _copy_ the contents of these memory buffers. LSIO can't use user-supplied
    // buffers as its cache, because LSIO can't guarantee that the user-supplied buffers will be immutable.
    pub raw_buffers: Option<Vec<&mut [u8]>>,

    // Memory buffers for storing the data after it has been processed.
    pub processed_buffers: Option<Vec<&mut [u8]>>,
}
```

#### Optimising the IO plan

LSIO optimizes the sequence of `byte_ranges` read from IO.

We explicitly have 2-steps: first, we optimise the IO plan. Then we read from disk.

We make the optimisation modular by using iterators.

First, establish a cache for raw chunk data.

```rust
let mut cache = CacheOfRawChunks::new();
```

(Note that caching won't be implemented for a while - if at all. For now, I'm just checking that the design could,
in principal, support caching. For more discussion and design ideas about caching, 
see [this GitHub issue](https://github.com/JackKelly/light-speed-io/issues/9). In short, I think I need to simplify caching!)

Next, users create a set list of abstracted read operations: 

```rust
let plan = chunks
    .iter()  // I'm not sure we can use Rayon to parallelise this, if each chunk requires a mutable borrow of `cache`.
             // That said, each FileChunk _should_ only access a single path. Which might provide a mechanism to slice up
             // the cache. That said, I also think I need to simplify the caching, and restrict the caching to a layer
             // that sits between the user and the planner.
    .map( |filechunks| 
        OptimisedFileChunks::from(filechunks)
            .check_cache(&cache)
            .deduplicate()
            .merge_nearby_reads(merge_threshold_in_megabytes)
            .detect_contiguous_reads()

            // In the ideal case, we'll read directly from IO into the cache's memory buffer. And then we'll
            // share immutable slice(s) of that memory buffer with the user.
            .plan_cache(&mut cache)
    )
```

Each `Item` will be a single `FileChunks` struct. After this line, no processing will have started yet. You'd have to call `collect()` to collect, if you wanted to... but we want to submit the first few operations before we've finished computing the operations. So, usually, you'd leave `plan` as an uncollected iterator.

Data chunks returned to the user will always immutable. That will make the design easier and faster: If the data is always immutable, then we can use slices instead of copies when apportioning merged reads. And it allows LSIO's caching mechanism to be faster than the operating system's page cache because LSIO doesn't have to memcopy anything. In contrast, the OS has to memcopy from page cache into the process' address space.

For now, just optimise each `FileChunks` struct, independently of other `FileChunks`. Let's assume - for now - that the user is only submits one `FileChunks` per file!

Optimisations include:

- Check the cache to see if it already has some of the data we require. It's important to do this first, so that all subsequent operations only operate on just the byte ranges that we actually need to read from disk. Although, this isn't _essential_. If this gets tricky, we could not concern the planning stage with the cache, and only use the cache in the backend. But I think there are a few cases where this would lead to a sub-optimial plan.
- Deduplicate overlapping read requests. For example, if the user requests two chunks `[..1000, ..100]` then only actually read `..1000`, and return - as the second chunk - an immutable slice of the last 100 bytes. <font size="1">(Maybe don't implement this in the MVP. But check that the design can support this. Although don't worry too much - I'm not even sure if this issue would arise in the real world.)</font>
- Merge nearby reads, even if those reads are not strictly consecutive. Use `readv` to scatter the single read into the requested vectors <font size="2">(and - optionally - cache all the data read from disk, or just cache the chunks the user requested, or cache nothing)</font>
- Split large reads into multiple smaller reads. This is useful for reading from cloud storage buckets, or from HTTPS. <font size="2">(Maybe don't worry about this for now, given that this isn't relevant for reading local SSDs using io_uring. This may still be possible in a single vectored read operation, which reads into slices of the same underlying array. Or, if that's not possible, maybe spin up a separate io_uring context just for the individual reads that make up the single requested read, so it's clear when all the reads have finished.)</font>
- Detect contiguous chunks destined for different buffers, and use `readv` to read these. <font size="2">(Although we should benchmark `readv` vs `read`)</font>.

##### Implementation details (within LSIO)

The plan needs to express:
- "_this single optimized read started life as multiple, nearby reads. After performing this single read, the memory buffer will need to be sliced, and those slices processed in parallel. And we may want to throw away some data. The IO backend should be encouraged to use `readv` if available, to directly read into multiple buffers. (POSIX can use `readv` to read sockets as well as files.)_"
- "_this single optimized read started life as n multiple, overlapping reads. The user is expecting n slices (views) of this memory buffer_"
- "_these multiple optimized reads started life as a single read request. Create one large memory buffer. And each sub-chunk should be read directly into a different slice of the memory buffer._"

Maybe the answer is that the optimization step should be responsible for allocating the memory buffers, and it just submits a sequence of abstracted `readv` operations to the IO backend? If the backend can't natively perform `readv` then it's trivial for the backend to split one `readv` call into multiple `read`s. But! We should only allocate memory buffers when we're actually ready to read! Say we want to read 1,000,000 chunks. Using io_uring, we won't actually submit all 1,000,000 read requests at once: instead we'll keep the submission ring topped up with, say, 64 tasks. If the user wants all 1,000,000 chunks returned then we have no option but to allocate (and keep) all 1,000,000 chunks. But if, instead, the user wants each chunk to be processed and then moved into a common final array, then we only have to keep 64 buffers around at any given time.

```rust
/// Used as the key to the cache of raw chunks.
struct PathAndRange {
    path: PathBuf,
    range: Range,
}

struct CacheOfRawChunks {
    cache_of_raw_chunks: Map<Path, Map<Range, [u8]>>,
}

struct OptimisedChunk {
    byte_range: Range, 

    buffers: Optional<Vec<&mut [u8]>>,

    // Index of the original chunks for which this optimised chunk is a superset.
    // For example, if the user originally requested two chunks: ..100, 200..300,
    // and we merged these two chunks into a single read of ..300,
    // then idx_of_original_chunks would be [0, 1].
    idx_of_original_chunks: Vec<usize>,

    // Keys into the cache, for any cache hits which at least partially satisfy this read.
    // This is useful so we don't have to search through the cache a second time.
    cache_keys: Vec<PathAndRange>,
}

struct OptimisedFileChunks {
    original: FileChunks,
    optimised_chunks: Vec<OptimisedChunk>,
}
```

#### Reading chunks

After optimising the plan, we submit those operations and process them:


##### User code

(Maybe, I could make a separate crate which wraps compression algorithms as iterator adaptors, for decompressing chunks like this. See the [streaming-compressor crate](https://github.com/jorgecarleitao/streaming-decompressor/tree/main), but note that it doesn't actually implement any codecs)


if we want to apply a function to each chunk, we could do something like this. This example
is based on the Zarr use-case. For each chunk, we want to decompress, and apply a simple numerical
transformation.

```rust
// Load chunks
// We need one `Result` per chunk, because reading each chunk could fail.
// Note that we take ownership of the returned vectors of bytes.
let mut data: Vec<Result<Vec<u8>, lsio::Error>> = Vec::with_capacity(chunks.len());
reader.submit(&plan)  // Returns an Iterator.
    .par_iter()
    .decompress_zstd()
    .map(|chunk| chunk * 2)
    .collect_into_vec(&mut data);
```

Or we could move data to its final resting place:

```rust
let results: Vec<Result<(), lsio::Error>> = reader
    .submit(&plan)
    .decompress_zstd()
    .map(|chunk| chunk * 2)
    .mem_move_to_final_buffers();
```

`mem_move_to_final_buffers()` moves the data to its final location. The final location is specified by the user in `Chunk.final_buffers`. The completion queue entry's user_data contains a raw pointer, created by `Box::into_raw()`. The raw pointer would point to an `OptimisedFileChunks` struct (which contains all the information the Rayon worker thread needs).

##### Implementation details (within LSIO)

Merging and splitting read operations means that there's no longer a one-to-one mapping between chunks that the _user_ requested, and chunks that LSIO will request from the storage subsystem. This raises some important design questions:
- How do we ensure that each of the user's chunks are processes in their own threads. (The transform function supplied by the user probably expects the chunks that the user requested) Use Rayon! We can use `ring.completion().par_iter()`??? Which I _think_ wouldn't use a blocking thread synchronization primitive (instead it would use work steeling). I will test this (10 lines of Rust?!). 
- How to keep the submission queue topped up? Maybe a separate thread (not part of the worker thread pool, because we don't want to take CPU cores away from decompression). But, how to ask io_uring to apply backpressure? Set `IORING_SETUP_CQ_NODROP`. Then check for `-EBUSY` returned from `io_uring_submit()`, and wait before submitting, and warn the user that the CQ needs to be larger. When using `IORING_SETUP_SQPOLL`, also need to check `io_uring_has_overflow()` before submitting (and warn the user if overflow). See [my SO Q&A](https://stackoverflow.com/questions/77580828/how-to-guarantee-that-the-io-uring-completion-queue-never-overflows/77580829#77580829). But, not that - at the time of writing - Rust's io_uring crate doesn't support _setting_ `IORING_SETUP_CQ_NODROP` ([here](https://docs.rs/io-uring/latest/io_uring/struct.Builder.html) are the settings supported by Rust's io_uring crate). But I could probably implement that fairly easily.


Within LSIO, the pipeline for the IO ops will be something like this:

- User submits a Vector of `FileChunks`.
- In the main thread:
    - We need to get the file size for:
        - any `EntireFiles` (where `buffer` is `None`). If `EntireFiles` exist, then we need to get the file size ahead-of-time, so we can pre-allocate a memory buffer.
        - any `MultiRange`s which include offsets from the end of the file, iff the backend doesn't natively support offsets from the end of the file (or maybe this should be the backend's problem? Although I'd guess it'd be faster to get all file sizes in one go, ahead of time?)
        - in the MVP, let's get the file sizes in the main thread, using the easiest (blocking) method. In later versions, we can get the file sizes async. (Getting filesizes async might be useful when, for example, we need to read huge numbers of un-sharded Zarr chunks).
- Spin up a "submission thread". Its job is to keep the submission queue full. And, probably, to allocate buffers for io_uring to read into (if the user hasn't supplied buffers).
- How to move ownership of buffers that LSIO allocates _through_ io_uring? I _think_ the approach should be:
    - Allocate filehandles for each read op in flight. (So io_uring can chain `open(fh)`, `read(fh)`, `close(fh)`).
    - Submit the first, say, 64 read ops to each thread's submission queue. (Each "read op" would actually be a chain of open, read, close).
    - In the "submission thread": `let optimised_file_chunks = Box::new(optimised_file_chunks)`.
    - Set the SQE's user_data to `Box::into_raw(optimised_file_chunks)` (see [docs for into_raw](https://doc.rust-lang.org/std/boxed/struct.Box.html#method.into_raw)). `into_raw()` _consumes_ the object (but doesn't de-allocated it), which is exactly what we want. We mustn't touch `buffer` until it re-emerges from the kernel. And we _do_ want Rayon's worker thread (that processes the CQE) to decide whether to drop the buffer (after moving data elsewhere) or keep the buffer (if we're passing the buffer back to the user)
    - Pass the SQE to io_uring
    - Use `ring.collect().par_iter().for_each()` to process each CQE. Turn the user_data back into an _owned_ Box using `unsafe {optimised_file_chunks = Box::from_raw(cqe.user_data)}`. (see [docs for from_raw](https://doc.rust-lang.org/std/boxed/struct.Box.html#method.from_raw)).
