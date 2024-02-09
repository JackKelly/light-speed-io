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
    waker_and_op: Arc<Mutex<WakerAndOperation>>,
}

impl OperationFuture {
    pub(crate) fn new(operation: Operation) -> (Self, OperationWithCallback) {
        let waker_and_op = Arc::new(Mutex::new(WakerAndOperation::new()));

        // When the operation completes, we want to call `wake()` to wake the async executor.
        let waker_and_op_for_callback = waker_and_op.clone();
        let callback = move |operation| {
            // Take ownership of shared_state_for_callback:
            println!("Start of callback");
            let mut waker_and_op_locked = waker_and_op_for_callback.lock().unwrap();
            waker_and_op_locked.set_operation_and_wake(operation);
            println!("End of callback");
        };

        (
            Self { waker_and_op },
            OperationWithCallback::new(operation, callback),
        )
    }
}

impl Future for OperationFuture {
    type Output = Operation;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut waker_and_op_locked = self.waker_and_op.lock().unwrap();
        if waker_and_op_locked.operation.is_some() {
            println!("About to return Poll::Ready");
            Poll::Ready(waker_and_op_locked.operation.take().unwrap())
        } else {
            waker_and_op_locked.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct WakerAndOperation {
    waker: Option<Waker>,
    operation: Option<Operation>,
}

impl WakerAndOperation {
    fn new() -> Self {
        Self {
            waker: None,
            operation: None,
        }
    }

    pub(crate) fn set_operation_and_wake(&mut self, operation: Operation) {
        println!("At start of `set_operation_and_wake`");
        self.operation = Some(operation);
        if let Some(waker) = self.waker.take() {
            println!("Before `waker.wake()`");
            waker.wake();
            println!("After `waker.wake()`");
        }
    }
}
