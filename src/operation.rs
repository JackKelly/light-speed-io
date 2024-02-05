use std::fs;

use object_store::path::Path;
use object_store::Result;

pub(crate) struct OperationWithCallback {
    // This is a `Option` so we can `take` it.
    operation: Option<Operation>,

    // The callback function will be called when the operation completes.
    // The callback function can be an empty closure.
    // This is an `Option` so we can `take` it.
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

    pub(crate) fn get_mut_operation(&mut self) -> &mut Option<Operation> {
        &mut self.operation
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

        // This is an `Option` for two reasons: 1) `buffer` will start life
        // _without_ an actual buffer! 2) So we can `take` the buffer.
        buffer: Option<Result<Vec<u8>>>,

        // Keeping the file descriptor in this struct is just a quick hack to ensure that
        // we keep the file descriptor open until io_uring has finished with this task.
        // TODO: Remove the file descriptor from this struct once we let io_uring open files!
        // See Issue #1.
        fd: Option<fs::File>,
    },
}
