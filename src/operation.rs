use std::sync::Arc;

use bytes::Bytes;
use object_store::path::Path;
use object_store::Result;

pub(crate) struct OperationWithCallback {
    operation: Option<Operation>,

    // The callback function will be called when the operation completes.
    // The callback function can be an empty closure.
    callback: Option<Box<dyn FnOnce(Operation) + Send + Sync>>,
}

impl OperationWithCallback {
    pub(crate) fn new<F>(operation: Operation, callback: F) -> Self
    where
        F: FnOnce(Operation) + Send + Sync + 'static,
    {
        Self {
            operation: Some(operation),
            callback: Some(Box::new(callback)),
        }
    }

    pub(crate) fn execute_callback(&mut self) {
        let callback = self.callback.take().unwrap();
        callback(self.operation.take().unwrap());
    }

    pub(crate) fn get_operation(&self) -> &Option<Operation> {
        &self.operation
    }
}

/// `Operation` is used to communicate the user's instructions
/// to the backend. The intention is that there will be
/// one `Operation` variant per `ObjectStore` method.
/// This is necessary so we can have a queue of (potentially millions of) operations.
/// `Operation` is independent of the IO backend.
/// This same enum will be used to communicate with all IO backends.
#[derive(Debug)]
pub(crate) enum Operation {
    Get {
        location: Path,
        buffer: Option<Result<Bytes>>,
    },
}
