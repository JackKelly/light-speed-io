# light-speed-io
Read many chunks of files at high speed.

For now, this repo is just a place for me to tinker with ideas. This code won't do anything vaguely useful for months!

The ultimate ambition is to enable folks to read huge numbers of chunks from files as fast as modern hardware will allow.

In a single function call, users will submit a long list specifying all the chunks to read, and (optionally) a closure specifying a function to apply to each chunk (e.g. decompression followed by copying the data into a merged array). The aim is to minimise round-trips to RAM by keeping each chunk in the CPU cache. The main trick to keep data in CPU cache is to ensure that all functions to be performed on each chunk are performed in rapid succession (so-called "[temporal locality](https://en.wikipedia.org/wiki/Locality_of_reference)"). Chunks will be processed in parallel across multiple CPU cores.

Under the hood, my hope is that light-speed-io will use [`io_uring`](https://kernel.dk/io_uring.pdf) on Linux for local files and for cloud storage. If that works well, then I may get round to implementing [`I/O Rings`](https://learn.microsoft.com/en-us/windows/win32/api/ioringapi/) on Windows (11+), and [`kqueue`](https://en.wikipedia.org/wiki/Kqueue) on Mac OS X. Before submitting IO to the OS, light-speed-io will coalesce nearby reads, using tunable thresholds for each storage subsystem (for example, a spinning hard disk has very different performance characteristics to a modern SSD).

My first use-case for light-speed-io is to help to speed up reading [Zarr](https://zarr.dev/). After that, I'm interested in helping to create fast readers for "native" geospatial file formats like GRIB2 and EUMETSAT native files.

For more info, please [the draft design doc](https://github.com/JackKelly/light-speed-io/blob/main/design.md). Comments are very welcome!