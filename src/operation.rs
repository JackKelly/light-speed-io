use std::ffi::CString;

use object_store::Result;

pub struct OperationWithCallback {
    // This is a `Option` so we can `take` it.
    operation: Option<Operation>,

    pub fd: Option<u32>,

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
            fd: None,
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
        // Creating a new CString allocates memory. And io_uring openat requires a CString.
        // We need to ensure the CString is valid until the completion queue entry arrives.
        // So we keep the CString here, in the `Operation`.
        path: CString,

        // This is an `Option` for two reasons: 1) `buffer` will start life
        // _without_ an actual buffer! 2) So we can `take` the buffer.
        buffer: Option<Result<Vec<u8>>>,
    },
}
