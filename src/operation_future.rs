use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use crate::operation::{Operation, OperationWithCallback};

/// A Future for file operations (where an "operation" is `get`, `put`, etc.)
/// `O` is the output type, and is set in `OperationWithOutput::new`.
#[derive(Debug)]
pub(crate) struct OperationFuture {
    shared_state: Arc<Mutex<InnerState>>,
}

impl OperationFuture {
    pub(crate) fn new(op_type: Operation) -> (Self, OperationWithCallback) {
        let shared_state = Arc::new(Mutex::new(InnerState::new()));

        // When the operation completes, we want to call `wake()` to wake the async executor.
        let shared_state_for_callback = shared_state.clone();
        let callback = move |operation| {
            // Take ownership of shared_state_for_callback:
            let shared_state_for_callback = shared_state_for_callback;
            let mut shared_state_unlocked = shared_state_for_callback.lock().unwrap();
            shared_state_unlocked.set_output_and_wake(operation);
        };

        (
            Self { shared_state },
            OperationWithCallback::new(op_type, callback),
        )
    }
}

impl Future for OperationFuture {
    type Output = Operation;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared_state = self.shared_state.lock().unwrap();
        if shared_state.ready {
            Poll::Ready(shared_state.operation.take().unwrap())
        } else {
            shared_state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct InnerState {
    ready: bool,
    waker: Option<Waker>,
    operation: Option<Operation>,
}

impl InnerState {
    fn new() -> Self {
        Self {
            ready: false,
            waker: None,
            operation: None,
        }
    }

    pub(crate) fn set_output_and_wake(&mut self, operation: Operation) {
        self.operation = Some(operation);
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }
}
