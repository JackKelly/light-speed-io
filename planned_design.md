# Summary

The ultimate aim is to provide a suite of tools for working with large, labelled, multi-dimensional datasets as efficiently as possible on modern hardware. By "large" I mean datasets that are too large to fit into RAM. By "labelled" I mean datasets where each array dimension can be associated with coordinates. For example, a dataset of satellite imagery might have 4 dimensions: x, y, time, and spectral channel. The x and y dimensions might be labelled with longitude and latitude coordinates, respectively.

Please see [this blog post](https://jack-kelly.com/blog/2023-07-28-speeding-up-zarr) for more details of the background and motivations behind this project.

This git repository will (probably) contain multiple crates (see [issue #94](https://github.com/JackKelly/light-speed-io/issues/94)). Each crate will implement "one thing". Each crate will exist in one of five levels of abstraction. And there will be a Python API to each level of abstraction. See the "planned design" diagram below.

## Fitting into the ecosystem
Today, there are many awesome software packages for working with large, labelled, multi-dimensional datasets (such as xarray, fsspec, dask, satpy, etc.). My aim is to help speed up this existing stack: Either by providing tools that those existing Python packages can hook into, or by providing new tools which play nicely with the existing stack.

## Why bother to build `light-speed-io`? What gap does it fill?
LSIO is all about computational speed _and_ efficiency! Today, using existing packages, you can achieve high throughput by spinning up a large cluster. But that's expensive, power-hungry, and tedious. The aim of LSIO is to enable high throughput and low latency on a single machine.

## How to be efficient and fast?
By being as [sympathetic](https://dzone.com/articles/mechanical-sympathy) as possible to the hardware.

That sounds abstract! In concrete terms, one central aim is for the machine to do as little work as possible. Specifically:

Minimise the number of:
- round-trips to RAM,
- system calls,
- heap allocations,
- memory copies.

Maximise:
- the use of the CPU cache,
- and exploit all the levels of parallelism available within a single machine.

Use efficient IO APIs like io_uring.

## Concrete goals
Some example concrete goals include:
- Compute summary statistics of multi-terabyte dataset on a laptop, at a speed of about 5 minutes per terabyte (from a fast local SSD), with minimal RAM requirements.
- Train a large machine learning model from two Zarr datasets (e.g. satellite imagery and numerical weather predictions) at a sustained bandwidth to the GPU of at least 1 gigabyte per second (from local SSDs or from a cloud storage bucket), whilst performing some light processing on the data on-the-fly.

## Priorities
My first area of focus is on high-speed IO for local SSDs on Linux, to speed up training ML models from sharded Zarr datasets. But I'm definitely also interested in helping speed things up when data is stored in cloud object storage, and in helping to speed up general data analytics tasks on multi-dimensional data.

## How long will this take?
Implementing the complete design sketched out in this doc will take _years_!

By the end of 2024, I hope to have MVP implementations of "level 1 (I/O)" and "level 2 (parallel compute on chunks)" and a basic Zarr implementation for level 4. But please don't hold me to that!

# Which crates would live in this repo? What would they do? And how would they communicate with each other? 

![Planned design for LSIO](planned_design.svg)
([Original Google Draw version of this diagram](https://docs.google.com/drawings/d/1cpRai2k9y2Y9v4ieaof33FT27uB4JlK_rJL9Lvbj4MM/edit?usp=sharing).)

(See [this code sketch](https://github.com/JackKelly/light-speed-io/blob/new-design-March-2024/src/new_design_march_2024.rs) for some concrete ideas for how this can work.)

My hope is to categorise the crates into several different levels of abstraction:

## Abstraction level 1: Data input/output
This is lowest level of abstraction: the level closest to the hardware.

### Common interface
These IO crates will share a common interface: They'll have two MPMC `crossbeam:channel`s:
- Instruction channel: The user will send vectors of `enum IoOperation`s through this channel to express the user's IO requests (such as "get 1,000 chunks of `/foo/bar`"). These `IoOperation`s will probably be grouped ([#68](https://github.com/JackKelly/light-speed-io/issues/68)), such that the IO crate will guarantee that all operations in group _n_ are completed before any IO operations in group _n+1_ are started.
- Output channel: To return completed data to the user (these will also be grouped) (see [#105](https://github.com/JackKelly/light-speed-io/issues/105)).

LSIO will also enable buffer recycling whereby the user can optionally tell the IO crate "hey, I've finished with this buffer, so you can re-use it" (to minimise the number of heap allocations). ([#38](https://github.com/JackKelly/light-speed-io/issues/38)). This will probably be implemented via the `drop` method on `AlignedBuffer`.

### Crates
- [ ] `aligned_buffer`
- [ ] `lsio_io_uring_local` (this is what I'm currently working on): provide a small threadpool which performs IO using io_uring.
- [ ] [`lsio_io_python_bridge` #39)[https://github.com/JackKelly/light-speed-io/issues/39]
- [ ] [`object_store_bridge` #107](https://github.com/JackKelly/light-speed-io/issues/107) (also see [Ideas for fast cloud storage #10](https://github.com/JackKelly/light-speed-io/issues/10))
- [ ] maybe other crates for high-performance local storage on MacOS and/or Windows.

## Abstraction level 2: Parallel compute on chunks

### Common interface
These crates will all consume the `output channel` from the IO layer.

### Crates
- [ ] `lsio_compute`: Perform parallel computation on data. Users can supply any function to be applied to each chunk. The actual computation will probably be orchestrated by Rayon. This crate will implement functions for operating on the `struct Chunks` that represents each buffer with its metadata (see #105).
- [ ] `lsio_codecs`: Compression / decompression

## Abstraction level 3: Automatically scheduling compute & IO
The aim is to do to as little work as possible to satisfy the user's requests: don't repeat work (if we can avoid it) and don't do work that doesn't contribute to the final outcome.

- [ ] `lsio_scheduler`

## Abstraction level 4: Crates that load / write to a specific file format

These crates will each include a Python API.

### Crates
- [ ] `lsio_zarr`
- [ ] `lsio_grib`
- [ ] etc.

## Abstraction level 5: Domain-specific computation

### Crates
- [ ] `lsio_rechunker`
- [ ] `lsio_array`

