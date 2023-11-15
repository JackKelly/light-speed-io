/// This is just me sketching out pseudo-code for the design of the API,
/// and sketching out some of the important internals.

fn main() -> () {
    // Set config options (latency, bandwidth, maybe others)
    let config = Config::ssd_pcie_gen4_default();

    // Or do this :)
    let config = Config::auto_calibrate();

    // Define which chunks to load:
    let chunks = vec![FileChunks{path1, chunks1}, FileChunks{path2, chunks2}];

    // Start async loading of data from disk:
    let future = light_speed_io::IoUringLocal::read_chunks(&chunks, &config);


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
    let future = reader.read_entire_files(vec!["foo/bar", "foo/baz"]);
    // Under the hood, this needs to first chain {open, statx} in io_uring to get the filesizes, 
    // then have a threadpool allocate appropriate-sized buffers, 
    // and then chain {read, close} in io_uring to get the data.

    // Read many chunks from a small number of files
    let future = reader.read_chunks(&chunks);
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
