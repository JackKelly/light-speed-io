/// This is just me sketching out pseudo-code for the design of the API,
/// and sketching out some of the important internals.
/// 
/// Use-cases that this design needs to be tested against:
/// 1. Load ends of files (e.g. Zarr shard_index)
/// 2. Cache the lengths of files.
/// 3. Load huge numbers of files (e.g. non-sharded Zarrs)
/// 4. Load huge numbers of chunks from a small number of files.
/// 5. "Scatter" data to multiple arrays 
///    (e.g. loading uncompressed Zarr / EUMETSAT / GRIB files into final numpy array using DMA!)
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
            range: FileRange::EntireFile, // Read all of file
        },
        FileChunks{
            path: "/foo/baz", 
            range: FileRange::MultiRange(
                vec![
                    // Rust ranges can't express "get the last n elements".
                    // I'll assume I can create a little crate which allows 
                    // for Ranges from the end, like -10.. (the last 10 elements) or -10..-5.
                    ..1000, // Read the first 1,000 bytes
                    -200.., // Read the last 200 bytes
                    ],
            ),
        },
        ];

    // Start async loading of data from disk:
    let future = reader.read_chunks(&chunks);
    let data: Vec<Vec<u8>> = future.wait();

    // Or, read chunks and apply a function:
    let mut final_array = Array();
    let processing_fn = |chunk_idx: u64, chunk: &[u8]| -> &[u8] {
        // * decompress
        // * process
        // * move chunk to final array (the address of the final array would be passed into the closure?)
    };
    let future = read.read_chunks_and_apply(&chunks, processing_fn);
}

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
                // TODO: Load config from filename.
            }
        }
    }

    fn read_chunks(&self, chunks: &Vec<FileChunks>) -> Future<Vec<Vec<u8>>> {
        // Split `chunks` and send to threads (if there are enough chunks to be worth the hassle)
        // If there are >1,000 files to open then we'll have to process them in batches,
        // so as not to exceed the max filehandles per process.``
        // Perhaps split using Rayon?!? Although I'm not sure how Rayon would work with io_uring?!
        // Within each thread:
        // Loop through `chunks` to find the length of the buffers we need to pre-allocate.
        // (We might be loading anything from a few bytes to a few GB, so we can't just pre-allocate large
        // arrays and hope for the best!)
        // For chunks for which we already know the chunksize (maybe we have the filesize in cache, or the chunk 
        // explicitly tells us its length), immediately allocate the buffer and submit a read SQE to io_uring.
        // Some FileChunks will require us to get the length of the file before we can calculate the length of the buffer.
        // let nbytes = self.get_size(filename);
        // Only get the file sizes when necessary.
        // Allocate buffers.
        // Submit a read submission queue entry to io_uring. (Each thread will have its own io_uring)
        // Thought: Maybe, in a later version of LSIO, we should have *two* threads per io_uring:
        //   One thread submits requests to io_uring, and then goes away when it has finished.
        //   The other thread processes requests. This way, we can could start decompressing / copying 
        //      chunks before we've even got all the filesizes back.
        // Once we've submitted all read requests, then process the completion queue entries.
        // In a later version of LSIO, some of those CQEs will contain the filesize, and we'll have to submit a read request.
    }

    fn get_nbytes(&self, path: &PathBuf) -> u64 {
        // TODO: Use POSIX standard function name.
        // first check our local cache of file sizes. If that's empty, then
        // for the MVP, just use Rust's standard way to get the file length. Later, we may use io_uring by 
        // chaining {open, statx, close}.
        // Cache nbytes before returning.
    }
}
struct IoUringLocal {
    latency_ms: f64,
    bandwidth_gbps: f64,
    cache_of_nbytes_per_filename: map<PathBuf, u64>,
}

struct FileChunks {
    path: &Path,
    range: FileRange,
}

enum FileRange {
    EntireFile,
    MultiRange(Vec<Range>),
}

impl LocalIo for IoUringLocal {}
