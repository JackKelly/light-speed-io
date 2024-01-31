use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use crate::operation;
use crate::output::Output;

pub(crate) type SharedState = Arc<Mutex<InnerState>>;

/// A Future for file operations (where an "operation" is get, put, etc.)
#[derive(Debug)]
pub(crate) struct OperationFuture {
    shared_state: SharedState,
}

impl OperationFuture {
    pub(crate) fn new(operation: operation::Operation) -> Self {
        Self {
            shared_state: Arc::new(Mutex::new(InnerState::new(operation))),
        }
    }

    pub(crate) fn get_shared_state(&self) -> &SharedState {
        &self.shared_state
    }
}

impl Future for OperationFuture {
    type Output = Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.shared_state.lock().unwrap().poll(cx)
    }
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct InnerState {
    ready: bool,
    operation: operation::Operation,
    waker: Option<Waker>,
    output: Option<Output>,
}

impl InnerState {
    fn new(operation: operation::Operation) -> Self {
        Self {
            ready: false,
            operation,
            waker: None,
            output: None,
        }
    }

    pub(crate) fn set_output(&mut self, output: Output) {
        self.output = Some(output);
    }

    pub(crate) fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }

    pub fn get_operation(&self) -> operation::Operation {
        // TODO: Instead of cloning (which might be expensive), maybe the
        // `operation` shouldn't be behind a Mutex. Then we could share a reference.
        self.operation.clone()
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Output> {
        if self.ready {
            Poll::Ready(self.output.take().unwrap())
        } else {
            self.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
