# Light Speed IO (LSIO)
The ultimate ambition is to enable folks to read many small sequences of data from disk as fast as modern hardware will allow.

For now, this repo is just a place for me to tinker with ideas. This code won't do anything vaguely useful for months!

In a single function call, users will submit a long list specifying all the chunks to read, and (optionally) a sequence of functions to apply to each chunk (e.g. decompression followed by copying the data into a merged array). The aim is to minimise round-trips to RAM by performing all computation while the data is in CPU cache. The main trick to keep data in CPU cache is to ensure that all functions to be performed on a given chunk are performed in rapid succession (so-called "[temporal locality](https://en.wikipedia.org/wiki/Locality_of_reference)"). Chunks will be processed in parallel across multiple CPU cores.

Like Python's [`fsspec`](https://filesystem-spec.readthedocs.io/en/latest/) and Rust's [`object_store`](https://docs.rs/object_store/latest/object_store/), `light-speed-io` will aim to provide a uniform API for interacting with multiple storage systems (cloud storage buckets, local disks, and HPC). Unlike `fsspec` or `object_store`, `light-speed-io` will optimise the read operations: for example, when reading from a spinning disk, light-speed-io will coalesce nearby reads, using tunable thresholds. When reading from cloud object storage, LSIO will break large reads into multiple small reads, and submit those reads in parallel. All this optimisation should be entirely transparent to the user: the user shouldn't have to change their behaviour to optimise for each storage backend they're using. LSIO will do that optimisation for the user.

Under the hood, my hope is that light-speed-io will use [`io_uring`](https://kernel.dk/io_uring.pdf) on Linux for local files and for cloud storage. If that works well, then I may get round to implementing [`I/O Rings`](https://learn.microsoft.com/en-us/windows/win32/api/ioringapi/) on Windows (11+), and [`kqueue`](https://en.wikipedia.org/wiki/Kqueue) on Mac OS X.

My first use-case for light-speed-io is to help to speed up reading [Zarr](https://zarr.dev/). After that, I'm interested in helping to create fast readers for "native" geospatial file formats like GRIB2 and EUMETSAT native files. And, even further than that, I'm interested in efficient & fast _computation_ on [out-of-core](https://en.wikipedia.org/w/index.php?title=Out-of-core), chunked, labelled, multi-dimensional data.

For more info, please [the draft design doc](https://github.com/JackKelly/light-speed-io/blob/main/design.md). Comments are very welcome!

## Compilation

LSIO is a mixed python/rust project which uses PyO3 and Maturin to bind Rust to Python.

1. [Install `pyenv`](https://github.com/pyenv/pyenv-installer#installation--update--uninstallation) (not to be confused with `virtualenv`!)
2. Set up a Python virtual environment, and activate that env.
3. Use `pyenv` to install a Python interpreter, _with the shared library for Python_. See [the PyO3 docs](https://pyo3.rs/main/getting_started#virtualenvs):
    - `env PYTHON_CONFIGURE_OPTS="--enable-shared" pyenv install 3.11`
4. Use `pyenv-virtualenv` (which should have been installed automatically by the `pyenv` install script) to set up and configure a virtual env:
    - `pyenv virtualenv 3.11 light-speed-io-3.11`
    - `pyenv activate light-speed-io-3.11`
5. `pip install maturin`
6. Now we can use normal `cargo` commands (within this `pyenv`): `cargo test` etc.
7. To generate a Python wheel: `maturin develop`
