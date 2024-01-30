use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use object_store::{path::Path, Result};

// One enum variant per `ObjectStore` method.
#[derive(Debug)]
pub(crate) enum Operation {
    Get { location: Path },
}

#[derive(Debug)]
pub(crate) struct OperationFuture {
    pub(crate) shared_state: Arc<SharedState>,
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct SharedState {
    result_and_waker: Mutex<ResultAndWaker>,
    operation: Operation,
}

#[derive(Debug)]
struct ResultAndWaker {
    result: Option<Result<Bytes>>,
    waker: Option<Waker>,
}

/// Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
impl Future for OperationFuture {
    type Output = Result<Bytes>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut result_and_waker = self.shared_state.result_and_waker.lock().unwrap();
        if result_and_waker.result.is_some() {
            Poll::Ready(result_and_waker.result.take().unwrap())
        } else {
            result_and_waker.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
impl OperationFuture {
    pub fn new(operation: Operation) -> Self {
        let result_and_waker = ResultAndWaker {
            result: None,
            waker: None,
        };

        let shared_state = Arc::new(SharedState {
            result_and_waker: Mutex::new(result_and_waker),
            operation,
        });

        Self { shared_state }
    }
}

impl SharedState {
    pub fn set_result_and_wake(&mut self, result: Result<Bytes>) {
        let mut result_and_waker = self.result_and_waker.lock().unwrap();
        result_and_waker.result = Some(result);
        if let Some(waker) = result_and_waker.waker.take() {
            waker.wake()
        }
    }

    pub fn get_operation(self) -> Operation {
        self.operation
    }
}
