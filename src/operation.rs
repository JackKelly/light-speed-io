use bytes::Bytes;
use object_store::path::Path;
use object_store::Result;

#[derive(Debug)]
pub(crate) struct OperationWithOutput<F, O>
where
    F: FnOnce(&OpType, O),
{
    op_type: OpType,
    output: Option<O>,

    // The callback function will be called when the operation completes.
    // The callback function can be an empty closure.
    callback: F,
}

impl<F, O> OperationWithOutput<F, O>
where
    F: FnOnce(&OpType, O),
{
    pub(crate) fn new(op_type: OpType, callback: F) -> Self {
        let output = match op_type {
            OpType::Get => Option::<Result<Bytes>>::None,
        };

        Self {
            op_type,
            output,
            callback,
        }
    }

    pub(crate) fn execute_callback(&mut self) {
        self.callback(&self.op_type, self.output.take());
    }
}

/// `OpType` is used to communicate the user's instructions
/// to the backend. The intention is that there will be
/// one `OpType` variant per `ObjectStore` method.
/// This is necessary so we can have a queue of (potentially millions of) operations.
/// `OpType` is independent of the IO backend.
/// This same enum will be used to communicate with all IO backends.
#[derive(Debug)]
pub(crate) enum OpType {
    Get { location: Path },
    Foo, // TODO: Remove
}
