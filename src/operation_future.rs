use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use crate::operation;

/// A Future for file operations (where an "operation" is get, put, etc.)
#[derive(Debug)]
pub(crate) struct OperationFuture<Output> {
    shared_state: Arc<SharedStateForOpFuture<Output>>,
}

impl<Output> OperationFuture<Output>
where
    Output: Send + Sync,
{
    pub(crate) fn new(operation: operation::Operation) -> Self {
        Self {
            shared_state: Arc::new(SharedStateForOpFuture::new(operation)),
        }
    }

    pub(crate) fn get_shared_state(&self) -> Arc<SharedStateForOpFuture<Output>> {
        self.shared_state
    }
}

impl<Output> Future for OperationFuture<Output>
where
    Output: Send + Sync,
{
    type Output = Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.shared_state.poll(cx)
    }
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct SharedStateForOpFuture<Output> {
    waker_and_output: Mutex<WakerAndOutput<Output>>,
    operation: operation::Operation,
}

impl<Output> SharedStateForOpFuture<Output>
where
    Output: Send + Sync,
{
    fn new(operation: operation::Operation) -> Self {
        Self {
            waker_and_output: Mutex::new(WakerAndOutput::<Output>::new()),
            operation,
        }
    }

    pub(crate) fn set_output_and_wake(&mut self, output: Output) {
        let mut waker_and_output = self.waker_and_output.lock().unwrap();
        waker_and_output.output = Some(output);
        if let Some(waker) = waker_and_output.waker.take() {
            waker.wake()
        }
    }

    pub fn get_operation(self) -> operation::Operation {
        self.operation
    }

    fn poll(&self, cx: &mut Context<'_>) -> Poll<Output> {
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
struct WakerAndOutput<Output> {
    output: Option<Output>,
    waker: Option<Waker>,
}

impl<Output> WakerAndOutput<Output>
where
    Output: Send + Sync,
{
    fn new() -> Self {
        Self {
            output: None,
            waker: None,
        }
    }
}
