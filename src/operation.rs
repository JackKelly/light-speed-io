use std::ffi::CString;

use io_uring::types;
use tokio::sync::oneshot;

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
        buffer: Option<anyhow::Result<Vec<u8>>>,
        fixed_fd: Option<types::Fixed>,
    },
}

#[derive(Debug)]
pub(crate) struct OperationWithChannel {
    pub(crate) operation: Operation,
    // `output_channel` is an `Option` because `send` consumes itself,
    // so we need to `output_channel.take().unwrap().send(Some(buffer))`.
    output_channel: Option<oneshot::Sender<anyhow::Result<Vec<u8>>>>,
    error_has_occurred: bool,
}

impl OperationWithChannel {
    pub(crate) fn new(operation: Operation) -> (Self, oneshot::Receiver<anyhow::Result<Vec<u8>>>) {
        let (output_channel, rx) = oneshot::channel();
        (
            Self {
                operation,
                output_channel: Some(output_channel),
                error_has_occurred: false,
            },
            rx,
        )
    }

    pub(crate) fn send_result(&mut self) {
        match self.operation {
            Operation::Get { ref mut buffer, .. } => {
                self.output_channel
                    .take()
                    .unwrap()
                    .send(buffer.take().unwrap())
                    .unwrap();
            }
        }
    }

    pub(crate) fn error_has_occurred(&self) -> bool {
        self.error_has_occurred
    }

    pub(crate) fn send_error(&mut self, error: anyhow::Error) {
        if self.error_has_occurred {
            eprintln!("The output_channel has already been consumed (probably by a previous error)! But a new error has been reported: {error}");
            return;
        }
        self.error_has_occurred = true;

        let error = error.context(format!("Operation = {:?}", self.operation));

        self.output_channel
            .take()
            .unwrap()
            .send(Err(error))
            .unwrap();
    }
}
