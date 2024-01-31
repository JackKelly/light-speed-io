use bytes::Bytes;
use object_store::Result;

#[derive(Debug)]
pub(crate) enum Output {
    Get { buffer: Result<Bytes> },
    Foo, // TODO: Remove this! I'm just putting this here so we don't get 'unreachable pattern' warnings in match statements!
}
