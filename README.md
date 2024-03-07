# Light Speed IO (LSIO)
The ultimate ambition is to enable folks to read many small sequences of data from disk as fast as modern hardware will allow.

For now, this repo is just a place for me to tinker with ideas. This code won't do anything vaguely useful for months!

Under the hood, my hope is that `light-speed-io` will extend `object_store` to use [`io_uring`](https://kernel.dk/io_uring.pdf) on Linux for local files and for cloud storage. If that works well, then I may get round to implementing [`I/O Rings`](https://learn.microsoft.com/en-us/windows/win32/api/ioringapi/) on Windows (11+), and [`kqueue`](https://en.wikipedia.org/wiki/Kqueue) on Mac OS X.

My first use-case for light-speed-io is to help to speed up reading [Zarr](https://zarr.dev/). After that, I'm interested in helping to create fast readers for "native" geospatial file formats like GRIB2 and EUMETSAT native files. And, even further than that, I'm interested in efficient & fast _computation_ on [out-of-core](https://en.wikipedia.org/w/index.php?title=Out-of-core), chunked, labelled, multi-dimensional data.

## Benchmarking & profiling (on Linux)

```shell
sudo apt install vmtouch gnuplot
```

LSIO reads 1,000 small files during benchmarking. First, create these files using [`fio`](https://fio.readthedocs.io/en/latest/fio_doc.html). `fio` will also give one estimate of how fast you should expect your drive to go:
```shell
sudo mkdir /tmp/fio
sudo chmod a+rw /tmp/fio
fio benches/fio.ini
```

OPTIONAL: Clear all caches:
```shell
sudo sysctl -w vm.drop_caches=3
```

OPTIONAL: Monitor IO performance. Run [`iostat`](https://man7.org/linux/man-pages/man1/iostat.1.html) in a new terminal.
The average queue size is the `aqu-sz` column.
```shell
iostat -xm --pretty 1 -p <device>
```

Compile and run all benchmarks:
```shell
cargo bench
```

OPTIONAL: Enable perf counters and profile the benchmark code:
```shell
echo "0" | \
    sudo tee '/proc/sys/kernel/yama/ptrace_scope' | \
    sudo tee '/proc/sys/kernel/perf_event_paranoid' | \
    sudo tee '/proc/sys/kernel/kptr_restrict'

perf stat target/release/deps/uring_get-<HASH PRINTED BY CARGO BENCH> uring_get --bench --profile-time 5
```
