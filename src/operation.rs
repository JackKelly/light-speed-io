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
        fixed_fd: Option<types::Fixed>,
    },
}

#[derive(Debug)]
pub(crate) enum Output {
    Get { buffer: Vec<u8> },
}

impl Operation {
    fn matches(&self, output: &Output) -> bool {
        match self {
            Operation::Get { .. } => matches!(output, Output::Get { .. }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct OperationWithOutput {
    operation: Operation,
    output: Option<Output>,
    // `output_channel` is an `Option` because `send` consumes itself,
    // so we need to `output_channel.take().unwrap().send(Some(buffer))`.
    output_channel: Option<oneshot::Sender<anyhow::Result<Output>>>,
    error_has_occurred: bool,
}

impl OperationWithOutput {
    pub(crate) fn new(operation: Operation) -> (Self, oneshot::Receiver<anyhow::Result<Output>>) {
        let (output_channel, rx) = oneshot::channel();
        (
            Self {
                operation,
                output: None,
                output_channel: Some(output_channel),
                error_has_occurred: false,
            },
            rx,
        )
    }

    pub(crate) fn operation(&self) -> &Operation {
        &self.operation
    }

    pub(crate) fn operation_mut(&mut self) -> &mut Operation {
        &mut self.operation
    }

    pub(crate) fn set_output(&mut self, output: Output) {
        // Sanity check that the output is the correct variant:
        assert!(&self.operation.matches(&output));
        self.output = Some(output);
    }

    pub(crate) fn send_output(&mut self) {
        self.output_channel
            .take()
            .unwrap()
            .send(Ok(self.output.take().unwrap()))
            .unwrap();
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
