use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
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
    pub(crate) shared_state: Arc<RwLock<SharedState>>,
}

/// Shared state between the future and the waiting thread. Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
#[derive(Debug)]
pub(crate) struct SharedState {
    result: Option<Result<Bytes>>,
    waker: Option<Waker>,
    operation: Operation,
}

/// Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
impl Future for OperationFuture {
    type Output = Result<Bytes>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared_state = self.shared_state.write().unwrap();
        if shared_state.result.is_some() {
            Poll::Ready(shared_state.result.take().unwrap())
        } else {
            shared_state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Adapted from:
/// https://rust-lang.github.io/async-book/02_execution/03_wakeups.html#applied-build-a-timer
impl OperationFuture {
    pub fn new(operation: Operation) -> Self {
        let shared_state = Arc::new(RwLock::new(SharedState {
            result: None,
            waker: None,
            operation,
        }));

        Self { shared_state }
    }
}
impl SharedState {
    pub fn set_result_and_wake(&mut self, result: Result<Bytes>) {
        let mut shared_state = self.shared_state.write().unwrap();
        shared_state.result = Some(result);
        if let Some(waker) = shared_state.waker.take() {
            waker.wake()
        }
    }
}
