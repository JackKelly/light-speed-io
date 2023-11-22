/// This is just me sketching out pseudo-code for the design of the API,
/// and sketching out some of the important internals.
/// 
/// Use-cases that this design needs to be tested against:
/// - [x] Load ends of files (e.g. Zarr shard_index)
/// - [x] Cache the lengths of files.
/// - [x] Load huge numbers of files (e.g. non-sharded Zarrs)
/// - [x] Load huge numbers of chunks from a small number of files.
/// - [x] "Scatter" data to multiple arrays 
///       (e.g. loading uncompressed Zarr / EUMETSAT / GRIB files into final numpy array using DMA!)
/// - [x] Per chunk: Decompress, process, and copy to final array.
/// - [x] Allow LSIO to merge nearby chunks.

fn main() -> () {
    // Set config options (latency, bandwidth, maybe others)
    let config = SSD_PCIE_GEN4;
    // Or do this :)
    let config = IoConfig::auto_calibrate();
    
    // Init:
    let reader = IoUringLocal::new(&config);

    // Define which chunks to load:
    let chunks = vec![
        FileChunks{
            path: "/foo/bar",
            byte_range: ByteRange::EntireFile, // Read all of file
        },
        FileChunks{
            path: "/foo/baz", 
            byte_range: ByteRange::MultiRange(
                vec![
                    ..1000,     // Read the first 1,000 bytes
                    -500..,     // Read the last    500 bytes
                    -500..-100, // Read 400 bytes, until the 100th byte before the end
                    ],
            ),
            // I had considered also including a `destinations` field, holding Vec of mutable references to
            // the target memory buffers. But - at this point in the code - we 
            // don't know the file sizes, so we can't allocate buffers yet for EntireFiles.
        },
        ];

    // Start async loading of data from disk:
    let future = reader.read_chunks(&chunks);
    let data: Vec<Vec<u8>> = future.wait();

    // Or, read chunks and apply a function:
    let mut final_array = Array();
    let chunk_idx_to_array_loc = Vec::new();
    let processing_fn = |chunk_idx: u64, chunk: &[u8]| -> &[u8] {
        // ******** DECOMPRESS ************
        // If we don't know the size of the uncompressed chunk, then 
        // deliberately over-allocate, and shrink later...
        const OVER_ALLOCATION_RATIO: usize = 4;
        let mut decompressed_chunk = Vec::with_capacity(OVER_ALLOCATION_RATIO * chunk.size());
        decompress(&chunk, &mut decompressed_chunk);
        decompressed_chunk.shrink_to_fit();

        // ******** PROCESS ***********
        decompressed_chunk = decompressed_chunk / 2;  // to give a very simple example!

        // ******** COPY TO FINAL ARRAY **************
        final_array[chunk_idx_to_array_loc[chunk_idx]] = decompressed_chunk;
    };
    let future = read.read_chunks_and_apply(&chunks, processing_fn);
    future.wait();
    pass_to_python(&final_array);
}

pub struct IoConfig {
    pub latency_millisecs: f64,
    pub bandwidth_gbytes_per_sec: f64,
}

impl IoConfig {
    fn auto_calibrate() -> Self {}
    // Use Serde to save / load IoConfig to disk.
}

const SSD_PCIE_GEN4: IoConfig = IoConfig{latency_millisecs: 0.001, bandwidth_gbytes_per_sec: 8};

trait Reader {
    fn new(config: &IoConfig) -> Self { Self {config} }

    fn read_chunks(&self, chunks: &Vec<FileChunks>) -> Future<Vec<Vec<u8>>> {
        // (Implement all the general-purpose functionality in the Reader trait,
        //  and implement the io_uring-specific stuff in IoUringLocal.)
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

    fn get_file_size_in_bytes(&self, path: &PathBuf) -> u64 {
        // first check our local cache of file sizes. If that's empty, then
        // for the MVP, just use Rust's standard way to get the file length. Later, we may use io_uring by 
        // chaining {open, statx, close}.
        // Cache nbytes before returning.
    }
}
struct IoUringLocal {
    config: &IoConfig,
    cached_file_sizes_in_bytes: map<PathBuf, u64>,
}

impl LocalIo for IoUringLocal {
    // Implement io_uring-specific stuff...
}

struct FileChunks {
    path: &Path,
    byte_range: ByteRange,
}

enum ByteRange {
    EntireFile,
    MultiRange(Vec<Range>),
}