# Draft design for `light-speed-io` (LSIO)

`light-speed-io` (or "LSIO", for short) will be a Rust library crate for loading and processing many chunks of files, as fast as the storage system will allow. **The aim is to to allow users to load and process on the order of 1 million 4 kB chunks per second from a single local SSD**.

Why aim for 1 million chunks per second? See [this spreadsheet](https://docs.google.com/spreadsheets/d/1DSNeU--dDlNSFyOrHhejXvTl9tEWvUAJYl-YavUdkmo/edit#gid=0) an ML training use-case that comfortably requires hundreds of thousands of chunks per second.

But, wait, isn't it inefficient to load tiny chunks? [Dask recommends chunk sizes between 100 MB and 1 GB](https://blog.dask.org/2021/11/02/choosing-dask-chunk-sizes)! Modern SSDs are turning the tables: modern SSDs can sustain over 1 million input/output operations per second. And cloud storage looks like it is speeding up (for example, see the recent announcement of [AWS Express Zone One](https://aws.amazon.com/blogs/aws/new-amazon-s3-express-one-zone-high-performance-storage-class/); and there may be [ways to get high performance from existing cloud storage buckets](https://github.com/JackKelly/light-speed-io/issues/10), too). One reason that Dask recommends large chunk sizes is that Dask's scheduler takes on the order of 1 ms to plan each task. LSIO's data processing should be faster (see below).

(See [this Google Doc](https://docs.google.com/document/d/1_T0ay9wXozgqq334E2w1SROdlAM7y6JSgL1rmXJnIO0/edit) for a longer discussion of LSIO.)

## Planned features

- [ ] Provide a simple API for reading and writing many chunks of files (and/or many files) with single API call. Users will be able to ask LSIO: "_Please get me these million file chunks, and apply this function to each chunk, and then move the resulting data to these array locations._".
- [ ] The API will be the same, no matter which operating system you're on, and no matter whether the data is on local disk, or a cloud storage bucket, or available over HTTP. (Inspired by [fsspec](https://filesystem-spec.readthedocs.io/en/latest/) :smiley:!)
- [ ] Expose a Rust API and a Python API.
- [ ] Cache compressed chunks in RAM (configurable).
- [ ] Laser-focus on _speed_:
  - Achieve many [input/output operations per second](https://en.wikipedia.org/wiki/IOPS) (IOPS), high bandwidth, and low latency by:
    - exploiting modern, asynchronous operating system storage APIs ([`io_uring`](https://man.archlinux.org/man/io_uring.7.en) on Linux, [`I/O Ring`](https://windows-internals.com/i-o-rings-when-one-i-o-operation-is-not-enough/) on Windows, [`kqueue`](https://en.wikipedia.org/wiki/Kqueue) on MacOS X), and 
    - designing for inherently parallel storage systems like NVMe SSDs and cloud storage buckets.
  - Before submitting any IO operations, tune the sequence of IO operations according to the performance characteristics of each storage system. For example, on a hard drive (with spinning platters), the performance of random reads is dominated by the time taken to move the read head. So LSIO will merge nearby reads, even if those reads aren't strictly consecutive: For example, if we want to read every third block of a file, it may be faster to read the entire file, even if we immediately throw away two thirds of the data. Or, when reading large files from a cloud storage bucket, it may be faster to split each file into consecutive chunks, and request those chunks in parallel.
  - "Auto-tune" to each storage system. Or, if users do not want to auto-tune, then provide sane defaults for a range of common storage systems.
  - Exploit CPU caches and hence minimize the number of time-consuming reads from RAM. Once a chunk is loaded into CPU cache, perform all transformations on that chunk in quick succession (to maximize the chance that the data stays in cache), and pin the computation for a given chunk to a single CPU core (because level-1 CPU cache is specific to a CPU core).
  - Use multiple CPU cores in parallel (each working on a different chunk).
  - When scheduling work across multiple CPU cores: Avoid locks, or any synchronization primitives that would block a CPU core, wherever possible.
  - Look for opportunities to completely cut the CPU out of the data path. For example, if we're loading uncompressed [Zarr](https://zarr.dev/) chunks that are destined to be merged into a final numpy array, then we may be able to use [vectored read operations](https://en.wikipedia.org/wiki/Vectored_I/O) which will use [direct memory access](https://en.wikipedia.org/wiki/Direct_memory_access) (DMA) to directly copy chunks into the final numpy array from IO, without the CPU ever touching the data. This may be possible even in cases where the creation of the final array is more complicated than simply concatenating the chunks in RAM.
  - Where appropriate, align chunks in RAM (and pad the ends of chunks) so the CPU & compiler can easily use [SIMD instructions](https://en.wikipedia.org/wiki/Single_instruction,_multiple_data), and minimize the number of cache lines that must be read. (Using SIMD may provide a large speedup "just" for memory copies, even if the `transform` function doesn't use SIMD).
- [ ] The user will be able to supply a `transform` function that LSIO will apply to each chunk (in parallel). (Like the the "map" step in [MapReduce](https://en.wikipedia.org/wiki/MapReduce), except that LSIO allows "side effects" like copying the data to a final array). For each chunk, the user could request, for example, that the chunk be decompressed, followed by some numerical transformation, followed by moving the transformed data to a large array which is the concatenation of all the chunks. As much of this as possible should happen whilst the chunk is in the CPU cache (without time-consuming round-trips to RAM).
- [ ] LSIO will implement multiple IO backends. Each backend will exploit the performance features of a particular operating system and storage system. The ambition is to support:
    - These operating system APIs:
        - [ ] Linux [io_uring](https://en.wikipedia.org/wiki/Io_uring) (for local storage and network storage).
        - [ ] Windows [I/O Ring](https://windows-internals.com/i-o-rings-when-one-i-o-operation-is-not-enough/).
        - [ ] MacOS X [kqueue](https://en.wikipedia.org/wiki/Kqueue). (Although Jack doesn't currently own any Mac hardware!)
    - These storage systems:
        - [ ] Local disks. (With different optimizations for SSDs and HDDs).
        - [ ] Cloud storage buckets.
        - [ ] HTTP.

## Use cases

Allow for very fast access to arbitrary selections of:
* Multi-dimensional [Zarr](https://zarr.dev/) arrays. Jack is mostly focused on [_sharded_ Zarr arrays](https://zarr.dev/zeps/accepted/ZEP0002.html). But LSIO could also be helpful for non-sharded Zarr arrays.
    * Jack is particularly focused on speeding up the data pipeline for training machine learning models on multi-dimensional datasets, where we want to select hundreds of random crops of data per second. This is described below in the [Priorities](#priorities) section. The ambition is to enable us to read on the order of 1 million Zarr chunks per second (from a fast, local SSD).
* Other file formats used for multi-dimensional arrays, such as NetCDF, GRIB, and EUMETSAT's native file format. (LSIO could help to speed up [kerchunk](https://fsspec.github.io/kerchunk/))
* Vector database which indexes into crops of n-dimensional data. For example: implement "retrieval assisted generation" (RAG) for solar forecasting: give each chunk of satellite data a vector, and then give the ML model the 4 most similar examples from the entire history. This will have to be very fast to work at training time. 
* Interactive visualization of neuroscientific datasets 


## Priorities

Jack's main hypothesis is that it _should_ be possible to train large machine learning (ML) models _directly_ from multi-dimensional data stored on disk as Zarr arrays, instead of having to prepare ML training batches ahead of time. These ML models require random crops to be selected from multi-dimensional datasets, at several gigabytes per second. (See [Jack's blog post](https://jack-kelly.com/blog/2023-07-28-speeding-up-zarr) for more details. An example multi-dimensional dataset is satellite imagery over time.)

(And, even more ambitiously, LSIO may allow us to train directly from the _original data_ stored in, for example, GRIB files). 

The ultimate test is: Can LSIO enable us to train ML models directly from Zarr? (whilst ensuring that the GPU is constantly at near 100% utilization). So, Jack's _first_ priority will be to implement just enough of LSIO to enable us to test this hypothesis empirically: and that means implementing just one IO backend, to start with. That backend will be `io_uring` for local disks.

If this provides a significant speed-up, then Jack will focus on implementing reading from at least one cloud storage buckets, maybe using `io_uring` for async network IO.

On the other hand, if LSIO does _not_ provide a speed-up, then - to be frank - LSIO will probably be abandoned!

## Concrete examples of what LSIO should be capable of

### Machine learning

The aim will be to keep the GPU constantly fed with data, so the GPU is constantly at (or near) 100% utilisation. This often requires data to be read from disk at several GBytes per second.

* Train an ML model on a fast GPU, sampling directly from a single sharded Zarr dataset with tiny (~ 4 kB) chunks sizes. Load 1 million chunks per second from a single SSD.
* Stretch goals:
    * Also normalise the data in LSIO.
    * Train from _multiple_ Zarr datasets at the same time. For example, when training ML models to forecast solar power generation, we might want at least three datasets: satellite imagery, numerical weather predictions, and solar PV power. These datasets might use different geospatial projections, and different temporal resolutions. It'd be great to be able to randomly sample ML training examples, such that we take rectangular crops from the satellite data and NWP data, centered on the same geospatial location.

### Data processing

* Compute the mean of a 1 TB dataset in under 3 minutes on a single laptop. (A fast PCIe 4 SSD should sustain 7 GB/sec. Reading 1 TB at 7 GB/s takes about two and a half minutes).
* Convert a 1 TB GRIB dataset to Zarr in under 10 minutes, on a single machine.
* Rechunk a 1 TB Zarr dataset in under 10 minutes, on a single machine.

## Timeline

Ha! :smiley:. This project is in the earliest planning stages! It'll be _months_ before it does anything vaguely useful! And, for now at least, this project is just Jack hacking away his spare time whilst learning Rust! **So please don't depend on LSIO yet!**

## Design

### Public Rust API

#### Initialize a `Reader` struct

Using a persistent object will allow us to cache (in memory) values such as file sizes. And provides an opportunity to pre-allocate (and maybe re-use) memory buffers.

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

let io_operations = vec![

    // Read entirety of /foo/bar, and ask LSIO to allocate the memory buffer:
    IO_Operations_For_File{
        io_type: Read,
        path: "/foo/bar", 
        chunks: vec![
            Chunk{
                byte_range: ...,
                buffers: Auto,
            },
        ],
    },

    // Read 3 chunks from /foo/baz:
    IO_Operations_For_File{
        io_type: Read,
        path: "/foo/baz", 
        chunks: vec![
            Chunk{
                byte_range: ..1000,     // Read the first 1,000 bytes
                buffers: Manual(vec![&mut buf0]),
            },
            Chunk{
                byte_range: -500..-200, // Read 300 bytes, until the 200th byte from the end
                buffers: Manual(vec![&mut buf1]),
            },
            Chunk{
                byte_range: -100..,             // Read the last 100 bytes. For example, shared Zarrs
                buffers: Manual(vec![&mut buf2]), // place the shard index at the end of each file.
            },
        ],
    },
];
```

It is highly recommended that the user only submits _one_ `IO_Operations_For_File` object per `path`. 
This is because LSIO optimises each `IO_Operations_For_File` object independently of other `IO_Operations_For_File`s.

##### Under the hood (in LSIO)

```rust
pub struct IO_Operations_For_File {
    pub io_type: IO_Type,
    pub path: PathBuf,
    pub chunks: Vec<Chunk>,
};

pub enum IO_Type {
    Read,
    Write,
};

pub struct Chunk{
    pub byte_range: Range<i64>,

    // Memory buffers for storing the raw data, straight after the data arrives from IO.
    //
    // If buffers is Auto, then LSIO will take responsibility for allocating the buffers.
    //
    // If the user wants to supply buffers, then use `Manual(Vec<&mut [u8]>)`.
    // For example, this would allow us to bypass the CPU when copying multiple
    // uncompressed chunks from a sharded Zarr directly into the final array.
    // The buffers could point to different slices of the final array.
    // This mechanism could be used when creating the final array is more
    // complicated than simply appending chunks: you could, for example, read each
    // row of a chunk into a different `&mut [u8]`.
    //
    // LSIO borrows a mutable reference to each buffer, so that the user can supply a *slice* to a subset
    // of a larger array. This does mean that caching within LSIO will be a little slower
    // when the user supplies buffers, and memory usage will be larger, because LSIO's
    // cache will have to _copy_ the contents of these memory buffers. LSIO can't use user-supplied
    // buffers as its cache, because LSIO can't guarantee that the user-supplied buffers will be immutable
    // and live long enough.
    pub buffers: AutoOrManual<Vec<&mut [u8]>>,
};
```

#### Optimising the IO plan

LSIO optimizes the sequence of `byte_ranges` before sending those operations to the IO subsystem.

(Caching may be implemented in LSIO, but not for a while. For more discussion and design ideas about caching, 
see [this GitHub issue](https://github.com/JackKelly/light-speed-io/issues/9))

Users create a set list of abstracted read operations: 

```rust
let plan: Vec<Optimised_IO_Operation> = io_operations
    .par_iter()
    .map( |io_operations_for_file| 
        io_operations_for_file.merge_nearby_byte_ranges(merge_threshold_in_megabytes)
    )
```

After this line, no processing will have started yet. You'd have to call `collect()` to collect, if you wanted to... but we want to submit the first few operations before we've finished computing the operations. So, usually, you'd leave `plan` as an uncollected iterator, for now.

Data chunks returned to the user will always immutable. That will make the design easier and faster: If the data is always immutable, then we can use slices instead of copies when apportioning merged reads. And it allows LSIO's caching mechanism to be faster than the operating system's page cache because LSIO doesn't have to memcopy anything. In contrast, the OS has to memcopy from page cache into the process' address space.

For now, just optimise each `IO_Operations_For_File` struct, independently of other `IO_Operations_For_File`. Let's assume - for now - that the user only submits one `IO_Operations_For_File` per file!

Optimisations that LSIO will definitely implement include:

- Merge nearby reads into a single read, even if those reads are not strictly consecutive. Use `readv` to scatter the single read into the requested vectors. Optionally, for cloud storage, don't merge reads after the merged read hits a certain size <font size="2">(and - optionally - cache all the data read from disk, or just cache the chunks the user requested, or cache nothing)</font>
- Split large reads into multiple smaller reads. This is useful for reading from cloud storage buckets, or from HTTPS. <font size="2">(Maybe don't worry about this for now, given that this isn't relevant for reading local SSDs using io_uring. This may still be possible in a single vectored read operation, which reads into slices of the same underlying array. Or, if that's not possible, maybe spin up a separate io_uring context just for the individual reads that make up the single requested read, so it's clear when all the reads have finished.)</font>

Optimisations that LSIO _may_ implement include:

- (If/when we have caching: Check the cache to see if it already has some of the data we require. It's important to do this first, so that all subsequent operations only operate on just the byte ranges that we actually need to read from disk. Although, this isn't _essential_. If this gets tricky, we could not concern the planning stage with the cache, and only use the cache in the backend. But I think there are a few cases where this would lead to a sub-optimial plan.)
- Deduplicate overlapping read requests. For example, if the user requests two chunks `[..1000, ..100]` then only actually read `..1000`, and return - as the second chunk - an immutable slice of the last 100 bytes. <font size="1">(Maybe don't implement this in the MVP. But check that the design can support this. Although don't worry too much - I'm not even sure if this issue would arise in the real world.)</font>
- Detect contiguous chunks destined for different buffers, and use `readv` to read these. <font size="2">(Although we should benchmark `readv` vs `read`)</font>.

##### Implementation details (within LSIO)

The plan needs to express:
- "_this single optimized read started life as multiple, nearby reads. After performing this single read, throw away the buffers allocated just for filling the "gaps". All the user's requested buffers are now ready. So spawn processing tasks, one per user-requested buffer. The IO backend should be encouraged to use `readv` if available, to directly read into multiple buffers. (POSIX can use `readv` to read sockets as well as files.)_"
- "_this single optimized read started life as n multiple, overlapping reads. The user is expecting n slices (views) of this memory buffer_"
- "_these multiple optimized reads started life as a single read request. Create one large memory buffer. And each sub-chunk should be read directly into a different slice of the memory buffer._"

```rust
struct Optimised_IO_Operation {
    io_type: IO_Type,
    path: PathBuf,
    optimised_chunk: Chunk,

    // These are the buffers requested by the user.
    //
    // MERGING EXAMPLE:
    // For example, if the user originally requested two byte ranges: ..100, 200..300,
    // and we merged these into a single read by also reading 100..200 to a "dummy" buffer
    // then `original_buffers` would just contain the buffers originally requested by the user.
    // When this IO operation completes, LSIO will spawn one processing task per user requested buffer.
    //
    // SPLITTING EXAMPLE:
    // If this Optimised_IO_Operation is one of multiple Optimised_IO_Operations formed by splitting
    // a large IO_Operation, then `original_buffers` will contain just a single entry: the
    // (large) original buffer.
    original_buffers: Vec<&[u8]>,

    // When first splitting into multiple Optimised_IO_Operations,
    // create an AtomicUInt with the number of splits.
    // Then, when one of these Optimised_IO_Operations completes,
    // reduce this number by one. If the number is zero then
    // the `original_buffers` are all ready. This is only really relevant when splitting.
    number_of_outstanding_operations: Arc<AtomicUInt>,
};
```
##### TODO: Decide when to allocate buffers?

See [issue #22](https://github.com/JackKelly/light-speed-io/issues/22).
