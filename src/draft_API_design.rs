/// This is just me sketching out pseudo-code for the design of the API,
/// and sketching out some of the important internals.

fn main() -> () {
    // Initialise
    let reader = LightSpeedIO::builder()
        .base_path("/mnt/storage_ssd")  // So LSIO can infer which backend to use
        .latency_miliseconds(0.001)  // So LSIO knows when to merge nearby reads
        .bandwidth_gb_per_sec(10)
        .build();

    // Question: Maybe reader doesn't have to be stateful? Maybe we can just do stateless function
    // calls like read_rel_to_file_ends. But then we wouldn't be able to pre-allocate memory, etc.
    // But maybe that's not necessary?
    // Advantages of stateful (like above):
    // - Nice API to customise, but customisation could still be done with stateless calls:
    //     read_builder().foo(x).read("filename");
    //   or: 
    //     let config = SSDConfig::auto_calibrate();
    //       or:
    //     let config = SSDConfig::new().latency_ms(0.001).bw_gbps(10);
    //     let future = LightSpeedIO::IoUringLocal::read(&filename, &config)
    // - 

    // Or maybe it'd be better to specify the precise struct for the workload,
    // and only the Python API will automatically find the right class, given the
    // base path? This way, Rust can do more compile-time checks.
    let reader = LightSpeedIO::IoUringSSD::builder()
        .latency_miliseconds(0.001)  // So LSIO knows when to merge nearby reads
        .bandwidth_gb_per_sec(10)
        .build();

    // Read the shard_indexes from the end of files
    let file_chunks_to_load = vec![
        FileChunks{path, Chunk{offset: 100, len: 1000}},
        ];

    let future = reader.read_rel_to_file_ends(&file_chunks_to_load);
    // Under the hood, this needs to first chain {open, statx} in io_uring to get the filesizes, 
    // so we can compute the offset,
    // and then, as soon as a cqe is available, submit a 
    // chain of {read, close} to io_uring to get the data.

    let data: Vec<Vec<u8>> = future.wait();

    // Read all of some files (e.g. reading many unsharded Zarr chunks)
    let future = reader.read_entire_files(vec!["foo/bar", "foo/baz"])
    // Under the hood, this needs to first chain {open, statx} in io_uring to get the filesizes, 
    // then have a threadpool allocate appropriate-sized buffers, 
    // and then chain {read, close} in io_uring to get the data.

    // Read many chunks from a small number of files
    let future = reader.read_chunks(vec![FileChunks{path, chunks}, FileChunks{path2, chunks2}]);
    // This time, we don't need the filesize ahead of time! So don't bother doing `statx`.
}

// -------------- CONFIG -----------------------
struct Config {
    latency_ms: f64,
    bandwidth_gbps: f64,
}

impl Config {
    fn ssd_pcie_gen4_default() -> Self {
        Config {
            latency_ms: 0.001,
            bandwidth_gbps: 8,
        }
    }
}

struct FileChunks {
    path: &Path,
    chunks: Vector<Chunk>,
}

enum Chunk {
    offset(u64),
    len(u32),
}

trait Reader {
    fn read_chunks(&self, chunks: &Vec<FileChunks>) -> Future<Vec<Vec<u8>>>;
}
