# Light Speed IO (LSIO)
The ultimate ambition is to enable folks to efficiently load and process large, multi-dimensional datasets as fast as modern CPUs & I/O subsystems will allow.

For now, this repo is just a place for me to tinker with ideas. This code won't do anything vaguely useful for months!

Under the hood, `light-speed-io` uses [`io_uring`](https://kernel.dk/io_uring.pdf) on Linux for local files, and will use [`object_store`](https://lib.rs/crates/object_store) for all other data I/O.

My first use-case for light-speed-io is to help to speed up reading [Zarr](https://zarr.dev/). After that, I'm interested in helping to create fast readers for "native" geospatial file formats like GRIB2 and EUMETSAT native files. And, even further than that, I'm interested in efficient & fast _computation_ on [out-of-core](https://en.wikipedia.org/w/index.php?title=Out-of-core), chunked, labelled, multi-dimensional data.

See [`planned_design.md`](planned_design.md) for more info.

# Roadmap

(This will almost certainly change!)

The list below is in (rough) chronological order. This roadmap is also represnted in the [GitHub milestones for this project, when sorted alphabetically](https://github.com/JackKelly/light-speed-io/milestones?direction=asc&sort=title&state=open).

### MVP IO backends
- [x] Implement minimal `lsio_uring` IO backend (for loading data from a local SSD)
- [-] [Benchmark `lsio_uring` backend](https://github.com/JackKelly/light-speed-io/milestone/3)
- [ ] [Implement minimal `object_store_bridge` IO backend](https://github.com/JackKelly/light-speed-io/milestone/4)
- [ ] [Compare benchmarks for `lsio_uring` vs `object_store_bridge`](https://github.com/JackKelly/light-speed-io/milestone/7)
- [ ] [Improve usability and robustness](https://github.com/JackKelly/light-speed-io/milestone/8)
- [ ] [Group operations](https://github.com/JackKelly/light-speed-io/milestone/9)

### MVP Compute:
- [ ] Build a general-purpose work-steeling framework for applying arbitrary functions to chunks of data in parallel
- [ ] Wrap a few decompression algorithms
- [ ] MVP Zarr library (just for _reading_ data), with Python API
- [ ] Benchmark `lsio_zarr` vs `zarr-python v3` (from Python)

### Iterate on the IO backends:
- [ ] Optimise (merge and split) IO operations

### Iterate on compute
- [ ] Investigate how to integrate LSIO with xarray, such that chunkwise computation can be "pushed down" to LSIO

### Iterate on file format libraries
- [ ] Implement writing using `lsio_uring`
- [ ] Implement writing using `lsio_object_store_bridge`
- [ ] Implement writing in `lsio_zarr`
- [ ] Implement simple GRIB reader?? (Maybe do this later. This may take a while!)

### MVP End-user applications!
- [ ] Convert GRIB to Zarr?
- [ ] Compute simple stats of a large dataset (to see if we hit our target of processing 1 TB per 5 mins on a laptop!)
- [ ] Load Zarr into a PyTorch training pipeline
- [ ] Load GRIB into a PyTorch training pipeline?
- [ ] Implement merging multiple datasets on-the-fly (e.g. NWP and satellite).
- [ ] Enable Zarr-Python to use LSIO as a storage and codec pipeline?

### Iterate on IO:
- [ ] Speed up reading from cloud storage buckets (using object_store)
- [ ] Maybe experiment with using io_uring for reading from cloud storage buckets
- [ ] Re-use IO buffers
- [ ] Register buffers with `io_uring`

# Project structure

Light Speed IO is organised as a [Cargo workspace](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html) with multiple ([small](https://rust-unofficial.github.io/patterns/patterns/structural/small-crates.html)) crates. The crates are organised in a [flat crate structure](https://matklad.github.io/2021/08/22/large-rust-workspaces.html). The flat crate structure is used by projects such as [Ruff](https://github.com/astral-sh/ruff), [Polars](https://github.com/pola-rs/polars), and [rust-analyser](https://github.com/rust-lang/rust-analyzer).

LSIO crate names use snake_case, following in the footsteps of the [Rust Book](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html) and [Ruff](https://github.com/astral-sh/ruff/tree/main/crates). (The choice of snake_case versus hyphens is, as far as I can tell, entirely arbitrary: [Polars](https://github.com/pola-rs/polars/tree/main/crates) and [rust-analyser](https://github.com/rust-lang/rust-analyzer/tree/master/crates) both use hyphens. I just prefer the look of underscores!)
