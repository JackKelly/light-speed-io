//! Provides a common framework for all LSIO IO backends.

use lsio_aligned_bytes::AlignedBytes;
use std::{ops::Range, path::Path};

pub trait Reader {
    /// Submit a GetRanges operation.
    ///
    /// `ranges` specify the byte ranges to read. Negative numbers are relative to the filesize.
    /// (Like indexing lists in Python.) For example:
    ///        0..-1   The entire file.
    ///        0..100  The first 100 bytes.
    ///     -100..-1   The last 100 bytes.
    ///
    /// `user_data` is used to identify each byte_range.
    /// One `user_data` instance per byte_range.
    /// For example, in Zarr, this would be used to identify the
    /// location at which this chunk appears in the merged array.
    ///
    /// # Errors:
    /// If the user submits a `get_ranges` operation with an invalid filename then
    /// the user will receive a single `std::io::Error(std::io::ErrorKind::NotFound)` with context
    /// that describes the filename that failed. If a subset of the `ranges` results in an error
    /// (e.g. reading beyond end of the file) then the user will receive a mixture of `Ok(Output)`
    /// and `Err`, where the `Err` will include context such as the filename and byte range.
    fn get_ranges<'life0, 'life1, 'life2>(
        &mut self,
        location: &'life0 Path,
        ranges: &'life1 [Range<isize>],
        user_data: &'life2 [u64],
    ) -> anyhow::Result<()>;
}

/// `Chunk` is used throughout the LSIO stack. It is the unit of data that's passed from the I/O
/// layer, to the compute layer, and to the application layer. (To be more precise:
/// `Result<Chunk>` is usually what is passed around!).
#[derive(Debug)]
pub struct Chunk {
    buffer: AlignedBytes,
    /// `user_data` can be used to uniquely identify each chunk, for example by providing an index
    /// into an array that provides more information about each chunk.
    user_data: u64,
}

#[derive(Debug)]
pub enum Output {
    Chunk(Chunk),
    // Other variants could be:
    // `BytesWritten`, `Listing(Vec<FileMetadata>)`, etc.
}

