use anyhow::{Error, Result};
use io_uring::{cqueue, squeue};
use std::ffi::CString;
use std::iter::zip;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::Arc;

///---------------  COMMON TO ALL I/O BACKENDS  ---------------------

/// `Chunk` is used throughout the LSIO stack. It is the unit of data that's passed from the I/O
/// layer, to the compute layer, and to the application layer.
#[derive(Debug)]
struct Chunk<M> {
    buffer: Vec<u8>, // TODO: Use `AlignedBuffer` or `Bytes`.
    metadata: M,
}

#[derive(Debug)]
enum IoOutput<M> {
    Chunk(Chunk<M>),
}

/// IO Operations (common to all I/O backends).
#[derive(Debug)]
enum IoOperation<M> {
    /// Submit a GetRanges operation.
    ///
    /// # Errors:
    /// If the user submits a GetRanges operation with an invalid filename then
    /// the user will receive a single Err(std::io::ErrorKind::NotFound) with context
    /// that describes the filename that failed.
    /// If a subset of the `byte_ranges` results in an error (e.g. reading beyond
    /// end of the file) then the user will receive a mixture of `Ok(Output::Buffer)`
    /// and `Err`, where the `Err` will include context such as the filename
    /// and byte_range.
    GetRanges {
        filename: PathBuf, // Or should we use `object_store::Path`?
        /// The byte ranges for the file. Negative numbers are relative to the filesize.
        /// (Like indexing lists in Python.) For example:
        ///        0..-1   The entire file.
        ///        0..100  The first 100 bytes.
        ///     -100..-1   The last 100 bytes.
        byte_ranges: Vec<Range<isize>>,
        /// metadata used to identify each byte_range.
        /// One metadata instance per byte_range.
        /// For example, in Zarr, this would be used to identify the
        /// location at which this chunk appears in the merged array.
        metadata: Option<Vec<M>>,
    },
    PutRanges {
        filename: PathBuf, // Or should we use `object_store::Path`?
        byte_ranges: Vec<Range<isize>>,
        /// Chunks of data to be written to IO.
        /// TODO: Do we need `metadata` when writing? Maybe not??
        chunks: Vec<Chunk<M>>,
    },
}

// TODO: Update this text!
//
// Once the GetRangesUserOp passes to the uring threadpool,
// the first worker thread which grabs this GetRangesUserOp
// will get the filesize (if necessary) and then optimise the byte_ranges and submit some
// combination of
// `UnchangedGetOp`, `MergedGetOp`, and `SplitGetOp` to the Rayon
// task queue. Each of these will implement the `UringTask` trait
// (which has methods for `process_cqe` and `next`).
//
// for example:
//
// Let's say the user asks for one 4 GByte file.
// Linux cannot load anything larger than 2 GB in one go.
// But we don't know the size immediately because the user used byte_range=0..-1.
// The steps will be:
// 1. Get the filesize from io_uring and, concurrently, open the file.
// 2. As soon as the filesize is returned, we see that this is a big file, and so we need to submit
//    a `SplitGetOp`.
// 3. Set split_byte_ranges to [0..2GB, 2GB..4GB].
// 4. Set next_to_submit to 0, and set n_completed to 0.
// 5. Submit the uring operations, using the appropriate pointer offset.
// 7. When both `read` operations have completed, submit the user_buffer and metadata to the
//    completion queue.
//
// Now let's say that the user asks for 1 million chunks from a single file.
// Some of these chunks are close, so we merge them.
// The user has asked for only 1,000 buffers to be allocated at any given time.
// 1. The first uring worker thread gets the filesize if necessary.
// 2. Then submits a mix of operations to the Rayon task queue.
// 3. Each MergedGetOp just submits a single read to uring,
// and when that read completes, it submits read-only slices (but how to keep the buffer alive, if
// we're only passing back &[u8]? Maybe use Bytes?).
trait OptimiseByteRanges<M> {
    type FilenameType: Clone;
    fn optimise(io_operation: IoOperation<M>, max_gap: usize, max_file_size: usize) -> Vec<Self>
    where
        Self: Sized,
    {
        // TODO: Implement optimisation. Use the methods below (`new_unchanged_byte_range`
        // etc.) to create the backend-specific enum variants. For now, I'm just creating
        // a "stub" by always returning `Unchanged`.
        match io_operation {
            IoOperation::GetRanges {
                filename,
                byte_ranges,
                metadata,
            } => {
                // TODO: Handle case when metadata is None.
                let filename = Self::convert_filename(filename);
                zip(byte_ranges, metadata.unwrap())
                    .map(|(byte_range, meta)| {
                        Self::new_unchanged_byte_range(
                            filename.clone(),
                            byte_range,
                            None,
                            Some(meta),
                        )
                    })
                    .collect()
            }
            IoOperation::PutRanges {
                filename,
                byte_ranges,
                chunks,
            } => todo!(),
        }
    }

    fn convert_filename(filename: PathBuf) -> Self::FilenameType;

    // A single byte range which has not been split, or merged with other byte ranges.
    fn new_unchanged_byte_range(
        filename: Self::FilenameType,
        byte_range: Range<isize>,
        buffer: Option<Vec<u8>>,
        metadata: Option<M>,
    ) -> Self;

    // A single. large byte range split into multiple, smaller byte ranges.
    fn new_split_byte_range(
        filename: Self::FilenameType,
        split_byte_ranges: Vec<Range<isize>>,
        user_byte_range: Range<isize>,
        user_buffer: Option<Vec<u8>>,
        user_metadata: Option<M>,
    ) -> Self;

    // Multiple byte ranges merged into a single, large byte range.
    fn new_merged_byte_range(
        filename: Self::FilenameType,
        merged_byte_range: Range<isize>,
        merged_buffer: Option<Vec<u8>>,
        user_byte_ranges: Vec<Range<isize>>,
        user_metadata: Option<Vec<M>>,
    ) -> Self;
}

///--------------------- URING-SPECIFIC CODE ------------------------
enum UringOptimisedByteRanges<M> {
    Unchanged {
        filename: Arc<CString>,
        byte_range: Range<isize>,
        buffer: Option<Vec<u8>>,
        metadata: Option<M>,
    },

    // A single user operation has been split into multiple get operations.
    // For example, a user submitted a 4 GByte file, but Linux cannot read more than 2 GB at once.
    // Each `Split` will be processed by only one worker thread, so we don't need locks.
    Split {
        filename: Arc<CString>,
        split_byte_ranges: Vec<Range<isize>>,
        next_to_submit: usize,
        n_completed: usize,

        // User information
        user_byte_range: Range<isize>,
        user_buffer: Option<Vec<u8>>,
        user_metadata: Option<M>,
    },

    // Multiple user-operations have been merged into a single operation
    Merged {
        filename: Arc<CString>,
        merged_byte_range: Range<isize>,
        merged_buffer: Option<Vec<u8>>,

        // User information
        user_byte_ranges: Vec<Range<isize>>,
        user_metadata: Option<Vec<M>>,
    },
}

impl<M> OptimiseByteRanges<M> for UringOptimisedByteRanges<M> {
    type FilenameType = Arc<CString>;

    fn convert_filename(filename: PathBuf) -> Self::FilenameType {
        Arc::new(
            CString::new(filename.as_os_str().as_bytes())
                .expect("Failed to convert filename {filename} to CString."),
        )
    }

    fn new_unchanged_byte_range(
        filename: Self::FilenameType,
        byte_range: Range<isize>,
        buffer: Option<Vec<u8>>,
        metadata: Option<M>,
    ) -> Self {
        Self::Unchanged {
            filename,
            byte_range,
            buffer,
            metadata,
        }
    }
    fn new_split_byte_range(
        filename: Self::FilenameType,
        split_byte_ranges: Vec<Range<isize>>,
        user_byte_range: Range<isize>,
        user_buffer: Option<Vec<u8>>,
        user_metadata: Option<M>,
    ) -> Self {
        Self::Split {
            filename,
            split_byte_ranges,
            next_to_submit: 0,
            n_completed: 0,
            user_byte_range,
            user_buffer,
            user_metadata,
        }
    }
    fn new_merged_byte_range(
        filename: Self::FilenameType,
        merged_byte_range: Range<isize>,
        merged_buffer: Option<Vec<u8>>,
        user_byte_ranges: Vec<Range<isize>>,
        user_metadata: Option<Vec<M>>,
    ) -> Self {
        Self::Merged {
            filename,
            merged_byte_range,
            merged_buffer,
            user_byte_ranges,
            user_metadata,
        }
    }
}

enum UringOperation<M> {
    GetRange {
        byte_range: UringOptimisedByteRanges<M>,
        fixed_file_descriptor: Option<usize>, // TODO: Use types::Fixed
    },
    PutRange {
        byte_range: UringOptimisedByteRanges<M>,
        fixed_file_descriptor: Option<usize>, // TODO: Use types::Fixed
    },
}

#[derive(Debug)]
enum NextStep<M> {
    SubmitEntries {
        entries: Vec<squeue::Entry>,
        // If true, then these squeue entries will register one file.
        register_file: bool,
    },
    Pending {
        output: Option<IoOutput<M>>,
    },
    // We're done! Remove this operation from the list of ops in flight.
    Done {
        // true means that the CQE reports that it has unregistered one file.
        unregister_file: bool,
        output: Option<IoOutput<M>>,
    },
}

impl<M> UringOperation<M> {
    fn get_first_step(&self, index_of_op: usize) -> NextStep<M> {
        match &self {
            Self::GetRange {
                byte_range,
                fixed_file_descriptor,
            } => todo!(),
            Self::PutRange {
                byte_range,
                fixed_file_descriptor,
            } => todo!(),
        }
    }

    /// # Errors:
    /// If io_uring reports an error, then this function will return an `std::io::Error` with the
    /// context set twice: First to the `UringOperation`, and then to the `NextStep`.
    fn process_cqe_and_get_next_step(
        &self,
        cqe: cqueue::Entry,
        index_of_op: usize,
    ) -> Result<NextStep<M>> {
        let opcode = get_opcode_from_user_data(cqe.user_data());

        // Check if the CQE reports an error. We can't return the error yet
        // because we need to know if we're expecting any more CQEs associated with this operation.
        // NOTE: A big improvement over the previous version of the code is that we can now send
        // every error that occurs (because we now have a limitless output Channel)!
        let maybe_error = cqe_error_to_anyhow_error(cqe.result());

        match &self {
            Self::GetRange {
                byte_range,
                fixed_file_descriptor,
            } => self.process_cqe_for_get_range(
                byte_range,
                fixed_file_descriptor,
                opcode,
                maybe_error,
            ),
            Self::PutRange {
                byte_range,
                fixed_file_descriptor,
            } => self.process_cqe_for_put_range(
                byte_range,
                fixed_file_descriptor,
                opcode,
                maybe_error,
            ),
        }
    }

    fn process_cqe_for_get_range(
        &self,
        byte_range: UringOptimisedByteRanges<M>,
        fixed_file_descriptor: Option<usize>,
        opcode: u8,
        maybe_error: Option<Error>,
    ) -> Result<NextStep<M>> {
        todo!();
    }
}

fn main() {
    let (tx, rx) = channel();

    let get_ranges_op = IoOperation::GetRanges {
        filename: PathBuf::from("foo/bar"),
        byte_ranges: vec![0..100, 500..-1],
        metadata: Some(vec![0, 1]),
    };

    tx.send(get_ranges_op).unwrap();

    let recv = rx.recv().unwrap();
    println!("{recv:?}");
}
