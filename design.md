# Summary

The ultimate aim is to provide a suite of tools for working with large, labelled, multi-dimensional datasets as efficiently as possible on modern hardware. By "large" I mean datasets that are too large to fit into RAM. By "labelled" I mean datasets where each dimension can have coordinates associated with it. For example, a dataset of satellite imagery might have 4 dimensions: x, y, time, and channel. The x and y dimensions might be labelled with longitude and latitude coordinates, respectively. 

There already exist some awesome projects to work with large, labelled, multi-dimensional datasets (such as xarray, fsspec, dask, satpy, etc.). My aim is to help speed up this existing stack: Either by providing tools that those existing Python packages can hook into, or by providing new tools which play nicely with the existing stack.

A concrete goal is to be able to compute summary statistics of multi-terabyte datasets on a laptop, at a speed of about 5 minutes per terabyte (from a fast local SSD), with minimal RAM requirements.

My first area of focus is on high-speed IO for local SSDs on Linux. But I'm definitely also interested in helping speed things up when data is stored in cloud object storage.

# What crates would live in this repo? What would they do? And how would they communicate with each other? 

![Planned design for LSIO](design.svg)
([Original Google Draw version of this diagram](https://docs.google.com/drawings/d/1cpRai2k9y2Y9v4ieaof33FT27uB4JlK_rJL9Lvbj4MM/edit?usp=sharing).)

My hope is to categorise the crates into several different levels of abstraction:

## Abstraction level 1: Data input/output
This is lowest level of abstraction: the level closest to the hardware.

### Common interface
These IO crates will share a common interface: They'll have three `Channels`:
- Instruction channel: The user will send `enum IoOperation`s through this channel to express the user's IO requests (such as "get 1,000 chunks of `/foo/bar`"). These instructions will probably be grouped (#68), such that the IO crate will guarantee that all operations in group _n_ are completed before any IO operations in group _n+1_ are started.
- Output channel: To return completed data to the user (these will also be grouped) (see #105).
- Buffer recycling channel: For the user to optionally tell the IO crate "hey, I've finished with this buffer, so you can re-use it" (to minimise the number of heap allocations). (#38)

### Crates
- [ ] `lsio_io_uring_local` (this is what I'm currently working on): provide a small threadpool which performs IO using io_uring.
- [ ] #107 (also see #10)
- [ ] #102
- [ ] maybe other crates for high-performance local storage on MacOS and/or Windows.

## Abstraction level 2: Parallel compute on chunks

### Common interface
These crates will all consume the `output channel` from the IO layer. See [this code sketch](https://github.com/JackKelly/light-speed-io/issues/104#issuecomment-1999780779) for an outline of how this could work.

### Crates
- [ ] #39
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

