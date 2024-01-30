use bytes::Bytes;
use object_store::{path::Path, Result};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use url::Url;

use crate::io_uring_thread::WorkerThread;
use crate::operation_future::{Operation, OperationFuture};

#[derive(Debug)]
pub struct IoUringLocal {
    config: Arc<Config>,
    worker_thread: WorkerThread,
}

// We can't re-use `object_store::local::Config` because it's private.
#[derive(Debug)]
struct Config {
    root: Url,
}

impl std::fmt::Display for IoUringLocal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IoUringLocal({})", self.config.root)
    }
}

impl Default for IoUringLocal {
    fn default() -> Self {
        Self::new()
    }
}

impl IoUringLocal {
    /// Create new filesystem storage with no prefix
    pub fn new() -> Self {
        // TODO: Set up thread and Sender!
        Self {
            config: Arc::new(Config {
                root: Url::parse("file:///").unwrap(),
            }),
            worker_thread: WorkerThread {
                handle: todo!(),
                sender: todo!(),
            },
        }
    }
}

// This block will eventually become `impl ObjectStore for IoUringLocal` but,
// for now, I'm just implementing one method at a time (whilst being careful to
// use the exact same function signatures as `ObjectStore`).
impl IoUringLocal {
    // TODO: `IoUringLocal` shouldn't implement `get` because `ObjectStore::get` has a default impl.
    //       Instead, `IoUringLocal` should impl `get_opts` which returns a `Result<GetResult>`.
    //       But I'm keeping things simple for now!
    // TODO: `ObjectStore::get` returns a pinned `Box`, not a pinned `Arc`!
    pub fn get(&mut self, location: &Path) -> Pin<Arc<dyn Future<Output = Arc<Result<Bytes>>>>> {
        let location = location.clone();
        let operation = Operation::Get { location };
        let op_future = Arc::pin(OperationFuture::new(operation));
        self.worker_thread
            .sender
            .send(op_future.clone())
            .expect("Failed to send message to worker thread!");
        op_future
    }
}
