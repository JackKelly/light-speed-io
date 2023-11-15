/// This is just me sketching out pseudo-code for the design of the API,
/// and sketching out some of the important internals.
/// 
/// Use-cases that this design needs to be tested against:
/// 1. Load ends of files (e.g. Zarr shard_index)
/// 2. Cache the lengths of files.
/// 3. Load huge numbers of files (e.g. non-sharded Zarrs)
/// 4. Load huge numbers of chunks from a small number of files.
/// 5. "Scatter" data to multiple arrays 
///    (e.g. loading uncompressed Zarr / EUMETSAT / GRIB files into final numpy array)
/// 6. Per chunk: Decompress, process, and copy to final array.
/// 7. Allow LSIO to merge nearby chunks.

fn main() -> () {
    // Set config options (latency, bandwidth, maybe others)
    let config = LocalIoConfig::SSD_PCIe_gen4;
    
    // Or do this :)
    let config = LocalIoConfig::FromFile("filename");
    let config = LocalIoConfig::AutoCalibrate;
    
    // Init:
    let reader = IoUringLocal::from_config(&config);

    // Define which chunks to load:
    let chunks = vec![
        FileChunks{
            path: "/foo/bar",
            range: Range::EntireFile, // Read all of file
        },
        FileChunks{
            path: "/foo/baz", 
            range: Range::MultiRange(
                vec![
                    // Should these be specified using Rust's builtin ranges?
                    // Rust ranges can't express "get the last n elements".
                    // But maybe I should create a little crate which allows 
                    // for Ranges from the end, like -10.. (the last 10 elements) or -10..-5.
                    Chunk{  // Read the first 1,000 bytes:
                        offset: Offset::FromStart(0), 
                        len: Len::Bytes(1000),
                    },
                    Chunk{  // Read the last 200 bytes:
                        offset: Offset::FromEnd(200),
                        len: Len::ToEnd,
                    },
                    ],
            ),
        },
        ];

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
// But, how to express that SSD_PCIe_gen4 isn't a valid config for, say, network IO?
// Maybe don't pass in a config Enum variant,
// instead have a ssd_pcie_gen4() method on IoUringLocal?
enum LocalIoConfig {
    SSD_PCIe_gen4,
    AutoCalibrate,
    FromFile(PathBuf),
}

trait LocalIo {
    fn from_config(config: &LocalIoConfig) -> Self {
        match config {
            SSD_PCIe_gen4 => LocalIoConfig {
                latency_ms: 0.001,
                bandwidth_gbps: 8,
            },
            AutoCalibrate => {
                // TODO: Automatically calibrate.
            },
            FromFile(filename) => {
                // TODO: Load filename.
            }
        }
    }
}
struct IoUringLocal {
    latency_ms: f64,
    bandwidth_gbps: f64,
}

impl LocalIo for IoUringLocal {}

struct FileChunks {
    path: &Path,
    chunks: Vector<Chunk>,
}

struct Chunk {
    offset: u64,
    len: u64,
}

trait Reader {
    fn read_chunks(&self, chunks: &Vec<FileChunks>) -> Future<Vec<Vec<u8>>>;
}
