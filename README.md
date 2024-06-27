# Light Speed IO (LSIO)
The ultimate ambition is to enable folks to efficiently load and process large, multi-dimensional datasets as fast as modern CPUs & I/O subsystems will allow.

For now, this repo is just a place for me to tinker with ideas. This code won't do anything vaguely useful for months!

Under the hood, `light-speed-io` uses [`io_uring`](https://kernel.dk/io_uring.pdf) on Linux for local files, and will use [`object_store`](https://lib.rs/crates/object_store) for all other data I/O.

My first use-case for light-speed-io is to help to speed up reading [Zarr](https://zarr.dev/). After that, I'm interested in helping to create fast readers for "native" geospatial file formats like GRIB2 and EUMETSAT native files. And, even further than that, I'm interested in efficient & fast _computation_ on [out-of-core](https://en.wikipedia.org/w/index.php?title=Out-of-core), chunked, labelled, multi-dimensional data.

See [`planned_design.md`](planned_design.md) for more info on the planned design. And please see [this blogpost](https://jack-kelly.com/blog/2023-07-28-speeding-up-zarr) for my motivations for wanting to help speed up Zarr.

# Roadmap

(This will almost certainly change!)

The list below is in (rough) chronological order. This roadmap is also represented in the [GitHub milestones for this project, when sorted alphabetically](https://github.com/JackKelly/light-speed-io/milestones?direction=asc&sort=title&state=open).

### Throw-away prototype
- [x] Initial prototype where a single crate does the IO and compute
- [x] `io_uring` prototype
- [x] `io_uring` prototype using `Rayon` to loop through io_uring completion queue
- [x] `io_uring` async/await implementation with `object_store`-like API
- [x] Try mixing `Tokio` with `Rayon`
- [x] Don't initialise buffers
- [x] Use aligned buffers and `O_DIRECT`
- [x] Benchmark against `object_store` using `criterion`
- [x] Chain open, read, close ops in `io_uring`
- [x] Build new workstation (with PCIe5 SSD)
- [x] Try using trait objects vs enum  vs `Box::into_raw` for tracking in-flight operations
- [x] Try using fixed (registered) file descriptors
- [x] Try using `Rayon` for the IO threadpool
- [x] Investigate Rust's `Stream`.

### Fresh start. Laying the foundations. New crates:
- [x] `lsio_aligned_bytes`: Shareable buffer which can be aligned to arbitrary boundaries at runtime
- [x] `lsio_threadpool`: Work-stealing threadpool (based on `crossbeam-deque`)
- [x] `lsio_io`: Traits for all LSIO IO backends

### MVP IO layer
- [x] Implement minimal `lsio_uring` IO backend (for loading data from a local SSD) with user-defined number of worker threads
- [ ] [Benchmark `lsio_uring` backend](https://github.com/JackKelly/light-speed-io/milestone/3)
- [ ] [Implement minimal `lsio_object_store_bridge` IO backend](https://github.com/JackKelly/light-speed-io/milestone/4)
- [ ] [Compare benchmarks for `lsio_uring` vs `lsio_object_store_bridge`](https://github.com/JackKelly/light-speed-io/milestone/7)
- [ ] [Improve usability and robustness](https://github.com/JackKelly/light-speed-io/milestone/8)
- [ ] [Group operations](https://github.com/JackKelly/light-speed-io/milestone/9)

### MVP Compute layer
- [ ] Build a general-purpose work-steeling framework for applying arbitrary functions to chunks of data in parallel. And respect groups.
- [ ] Wrap a few decompression algorithms

### MVP File format layer: Read from Zarr
- [ ] MVP Zarr library (just for _reading_ data)
- [ ] Python API for `lsio_zarr`
- [ ] Benchmark `lsio_zarr` vs `zarr-python v3` (from Python)

### Improve the IO layer:
- [ ] Optimise (merge and split) IO operations

### Improve the compute layer
- [ ] Investigate how xarray can "push down" chunkwise computation to LSIO

### MVP End-user applications!
- [ ] Compute simple stats of a large dataset (to see if we hit our target of processing 1 TB per 5 mins on a laptop!)
- [ ] Load Zarr into a PyTorch training pipeline
- [ ] Implement merging multiple datasets on-the-fly (e.g. NWP and satellite).

### First release!
- [ ] Docs; GitHub actions for Python releases; more rigorous automated testing; etc.
- [ ] Release!
- [ ] Enable Zarr-Python to use LSIO as a storage and codec pipeline?

### Implement writing
- [ ] Implement writing using `lsio_uring`
- [ ] Implement writing using `lsio_object_store_bridge`
- [ ] Implement writing in `lsio_zarr`

### Improve IO:
- [ ] [Speed up reading from cloud storage buckets](https://github.com/JackKelly/light-speed-io/issues/10) (using object_store)
- [ ] Maybe experiment with [using io_uring for reading from cloud storage buckets](https://github.com/JackKelly/light-speed-io/issues/10#issuecomment-2178689758)
- [ ] Re-use IO buffers
- [ ] Register buffers with `io_uring`
- [ ] Python API for LSIO's IO layer (and LSIO's compute layer?)

### Improve the file formats layer: Add GRIB support???
(Although maybe this won't be necessary because [dynamical.org](https://dynamical.org) are converting datasets to Zarr)
- [ ] Implement simple GRIB reader?
- [ ] Convert GRIB to Zarr?
- [ ] Load GRIB into a PyTorch training pipeline?

### Grow the team? (Only if the preceding work has shown promise)
- [ ] Try to raise grant funding?
- [ ] Hire???

### Future work (in no particular order, and no promise any of these will be done!)
- [ ] [Multi-dataset abstraction layer](https://github.com/JackKelly/light-speed-io/issues/142) (under the hood, the same data would be chunked differently for different use-cases. But that complexity would be hidden from users. Users would just interact with a single "logical dataset".)
- [ ] Allow xarray to "push down" all its operations to LSIO
- [ ] xarray-like data structures implemented in Rust? ([notes](https://docs.google.com/document/d/1_T0ay9wXozgqq334E2w1SROdlAM7y6JSgL1rmXJnIO0/edit#heading=h.7ctns22vpab5))
- [ ] Fast indexing operations for xarray ([notes](https://docs.google.com/document/d/1_T0ay9wXozgqq334E2w1SROdlAM7y6JSgL1rmXJnIO0/edit#heading=h.kjphntldyaaw))
- [ ] Support for kerchunk / [VirtualiZarr](https://discourse.pangeo.io/t/pangeo-showcase-virtualizarr-create-virtual-zarr-stores-using-xarray-syntax/4127) / [Zarr Manifest Storage Transformer](https://github.com/zarr-developers/zarr-specs/issues/287)
- [ ] Compute using SIMD / NPUs / GPUs, perhaps using [Bend](https://github.com/JackKelly/light-speed-io/issues/132) / [Mojo](https://github.com/JackKelly/light-speed-io/discussions/12)
- [ ] Support many compression algorithms
- [ ] Automatically tune performance
- [ ] "Smart" scheduling of compute and IO (see [notes](https://docs.google.com/document/d/1_T0ay9wXozgqq334E2w1SROdlAM7y6JSgL1rmXJnIO0/edit#heading=h.bqhd2mq9o42t))
- [ ] Tile-based algorithms for numpy
- [ ] EUMETSAT Native file format
- [ ] NetCDF
- [ ] Warping / spatial reprojection
- [ ] Rechunking Zarr
- [ ] Converting between formats (e.g. convert EUMETSAT `.nat` files to 10-bit per channel bit-packed Zarr). If there's no computation to be done on the data during conversion then do all the copying with `io_uring`: open source file -> read chunks from source -> write to destination -> etc.
- [ ] Write a wiki (or a book) on high-performance multi-dimensional data IO and compute
- [ ] Integrate with Dask to run tasks across many machines
- [ ] Use LSIO as the storage and compute backend for other software packages

# Project structure

Light Speed IO is organised as a [Cargo workspace](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html) with multiple ([small](https://rust-unofficial.github.io/patterns/patterns/structural/small-crates.html)) crates. The crates are organised in a [flat crate structure](https://matklad.github.io/2021/08/22/large-rust-workspaces.html). The flat crate structure is used by projects such as [Ruff](https://github.com/astral-sh/ruff), [Polars](https://github.com/pola-rs/polars), and [rust-analyser](https://github.com/rust-lang/rust-analyzer).

LSIO crate names use snake_case, following in the footsteps of the [Rust Book](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html) and [Ruff](https://github.com/astral-sh/ruff/tree/main/crates). (The choice of snake_case versus hyphens is, as far as I can tell, entirely arbitrary: [Polars](https://github.com/pola-rs/polars/tree/main/crates) and [rust-analyser](https://github.com/rust-lang/rust-analyzer/tree/master/crates) both use hyphens. I just prefer the look of underscores!)
