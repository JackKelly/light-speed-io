/// `Operation`s are used to communicate the user's instructions
/// to the backend. The intention is that there will be
/// one `Operation` variant per `ObjectStore` method.
/// This is necessary so we can have a queue of (potentially millions of) operations.
/// `Operation` is independent of the IO backend.
use std::{ffi::CString, ops::Range};

use io_uring::types;
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct Get<CQE> {
    pub(crate) core: OperationCore<Vec<u8>, CQE>,
}

#[derive(Debug)]
pub struct GetRange<CQE> {
    pub(crate) core: OperationCore<Vec<u8>, CQE>,
    range: Range<i32>,
}

#[derive(Debug)]
pub struct GetRanges<CQE> {
    pub(crate) core: OperationCore<Vec<Vec<u8>>, CQE>,
    ranges: Vec<Range<i32>>,
}

impl<CQE> Get<CQE>
where
    CQE: std::fmt::Debug,
{
    pub fn new(path: CString) -> Self {
        Self {
            core: OperationCore::new(path),
        }
    }
}

impl<CQE> GetRange<CQE>
where
    CQE: std::fmt::Debug,
{
    pub fn new(path: CString, range: Range<i32>) -> Self {
        Self {
            core: OperationCore::new(path),
            range,
        }
    }
}

impl<CQE> GetRanges<CQE>
where
    CQE: std::fmt::Debug,
{
    pub fn new(path: CString, ranges: Vec<Range<i32>>) -> Self {
        Self {
            core: OperationCore::new(path),
            ranges,
        }
    }
}

pub(crate) trait Operation {}
impl<CQE> Operation for Get<CQE> {}
unsafe impl<CQE> Send for Get<CQE> {}
impl<CQE> Operation for GetRange<CQE> {}
unsafe impl<CQE> Send for GetRange<CQE> {}
impl<CQE> Operation for GetRanges<CQE> {}
unsafe impl<CQE> Send for GetRanges<CQE> {}

#[derive(Debug)]
pub(crate) struct OperationCore<Output, CQE> {
    // Creating a new CString allocates memory. And io_uring openat requires a CString.
    // We need to ensure the CString is valid until the completion queue entry arrives.
    // So we keep the CString here, in the `Operation`.
    path: CString,
    fixed_fd: Option<types::Fixed>,
    output: Option<Output>,
    // `output_channel` is an `Option` because `send` consumes itself,
    // so we need to `output_channel.take().unwrap().send(Some(buffer))`.
    output_channel: Option<oneshot::Sender<anyhow::Result<Output>>>,
    error_has_occurred: bool,
    cqe: Option<CQE>,
}

impl<Output, CQE> OperationCore<Output, CQE>
where
    Output: std::fmt::Debug,
    CQE: std::fmt::Debug,
{
    pub(crate) fn new(path: CString) -> Self {
        Self {
            path,
            fixed_fd: None,
            output: None,
            output_channel: None,
            error_has_occurred: false,
            cqe: None,
        }
    }

    pub(crate) fn set_output_channel(&mut self) -> oneshot::Receiver<anyhow::Result<Output>> {
        let (output_channel, rx) = oneshot::channel();
        self.output_channel = Some(output_channel);
        rx
    }

    pub(crate) fn set_output(&mut self, output: Output) {
        assert!(self.output.is_none());
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

        let error = error.context(format!("OperationCore = {:?}", self));

        self.output_channel
            .take()
            .unwrap()
            .send(Err(error))
            .unwrap();
    }
}
