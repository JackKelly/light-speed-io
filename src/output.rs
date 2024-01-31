use bytes::Bytes;
use object_store::Result;

#[derive(Debug)]
pub(crate) enum Output {
    Get { buffer: Result<Bytes> },
}
