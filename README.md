# Light Speed IO (LSIO)
The ultimate ambition is to enable folks to read many small sequences of data from disk as fast as modern hardware will allow.

For now, this repo is just a place for me to tinker with ideas. This code won't do anything vaguely useful for months!

Under the hood, my hope is that `light-speed-io` will extend `object_store` to use [`io_uring`](https://kernel.dk/io_uring.pdf) on Linux for local files and for cloud storage. If that works well, then I may get round to implementing [`I/O Rings`](https://learn.microsoft.com/en-us/windows/win32/api/ioringapi/) on Windows (11+), and [`kqueue`](https://en.wikipedia.org/wiki/Kqueue) on Mac OS X.

My first use-case for light-speed-io is to help to speed up reading [Zarr](https://zarr.dev/). After that, I'm interested in helping to create fast readers for "native" geospatial file formats like GRIB2 and EUMETSAT native files. And, even further than that, I'm interested in efficient & fast _computation_ on [out-of-core](https://en.wikipedia.org/w/index.php?title=Out-of-core), chunked, labelled, multi-dimensional data.

## Benchmarking & profiling

```shell
sudo sysctl -w vm.drop_caches=3  // Clear all caches
cargo bench
perf stat target/release/deps/io_uring_local-<HASH PRINTED BY CARGO BENCH> io_uring_local --bench --profile-time 5
```
