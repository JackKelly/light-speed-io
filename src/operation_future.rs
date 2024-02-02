use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use crate::operation::{OpType, OperationWithOutput};

/// A Future for file operations (where an "operation" is `get`, `put`, etc.)
/// `O` is the output type, and is set in `OperationWithOutput::new`.
#[derive(Debug)]
pub(crate) struct OperationFuture<O> {
    shared_state: Arc<Mutex<InnerState<O>>>,
}

impl<O> OperationFuture<O> {
    pub(crate) fn new<F>(op_type: OpType) -> (Self, OperationWithOutput<F, O>)
    where
        F: FnOnce(&OpType, O),
    {
        let shared_state = Arc::new(Mutex::new(InnerState::new()));

        // When the operation completes, we want to call `wake()` to wake the async executor.
        let shared_state_for_callback = shared_state.clone();
        let callback = move |_: &OpType, output: O| {
            // Take ownership of shared_state_for_callback:
            let shared_state_for_callback = shared_state_for_callback;
            let mut shared_state_unlocked = shared_state_for_callback.lock().unwrap();
            shared_state_unlocked.set_output_and_wake(output);
        };

        (
            Self { shared_state },
            OperationWithOutput::new(op_type, callback),
        )
    }
}

impl<O> Future for OperationFuture<O> {
    type Output = O;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared_state = self.shared_state.lock().unwrap();
        if shared_state.ready {
            Poll::Ready(shared_state.output.take().unwrap())
        } else {
            shared_state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct InnerState<O> {
    ready: bool,
    waker: Option<Waker>,
    output: Option<O>,
}

impl<O> InnerState<O> {
    fn new() -> Self {
        Self {
            ready: false,
            waker: None,
            output: None,
        }
    }

    pub(crate) fn set_output_and_wake(&mut self, output: O) {
        self.output = Some(output);
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }
}
