use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use crate::operation::{self, OperationOutput};

#[derive(Debug)]
pub(crate) struct OperationFuture {
    shared_state: Arc<SharedStateForOpFuture>,
}

impl OperationFuture {
    pub(crate) fn new(operation: operation::Operation) -> Self {
        Self {
            shared_state: Arc::new(SharedStateForOpFuture::new(operation)),
        }
    }

    pub(crate) fn get_shared_state(&self) -> Arc<SharedStateForOpFuture> {
        self.shared_state
    }
}

impl Future for OperationFuture {
    type Output = OperationOutput;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.shared_state.poll(cx)
    }
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct SharedStateForOpFuture {
    waker_and_output: Mutex<WakerAndOutput>,
    operation: operation::Operation,
}

impl SharedStateForOpFuture {
    fn new(operation: operation::Operation) -> Self {
        Self {
            waker_and_output: Mutex::new(WakerAndOutput::new()),
            operation,
        }
    }

    pub(crate) fn set_output_and_wake(&mut self, output: OperationOutput) {
        let mut waker_and_output = self.waker_and_output.lock().unwrap();
        waker_and_output.output = Some(output);
        if let Some(waker) = waker_and_output.waker.take() {
            waker.wake()
        }
    }

    pub fn get_operation(self) -> operation::Operation {
        self.operation
    }

    fn poll(&self, cx: &mut Context<'_>) -> Poll<OperationOutput> {
        let mut waker_and_output = self.waker_and_output.lock().unwrap();
        if waker_and_output.output.is_some() {
            Poll::Ready(waker_and_output.output.take().unwrap())
        } else {
            waker_and_output.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[derive(Debug)]
struct WakerAndOutput {
    output: Option<OperationOutput>,
    waker: Option<Waker>,
}

impl WakerAndOutput {
    fn new() -> Self {
        Self {
            output: None,
            waker: None,
        }
    }
}
