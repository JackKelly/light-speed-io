use bytes::Bytes;
use object_store::path::Path;
use object_store::Result;

/// The `Operation` enum is used to communicate the user's instructions
/// to the backend. The intention is that there will be
/// one `Operation` variant per `ObjectStore` method.
/// This is necessary so we can have a queue of (potentially millions of) operations.
/// `Operation` is independent of the IO backend.
/// This same enum will be used to communicate with all IO backends.
#[derive(Debug)]
pub(crate) enum Operation {
    Get { location: Path },
}

#[derive(Debug)]
pub(crate) enum OperationOutput {
    Get { buffer: Result<Bytes> },
}

impl OperationOutput {
    fn get_output(&self)
}