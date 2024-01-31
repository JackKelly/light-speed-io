use bytes::Bytes;
use object_store::{path::Path, Result};
use std::sync::{mpsc, Arc};
use std::thread;
use url::Url;

use crate::operation::{Operation, OperationOutput};
use crate::operation_future::OperationFuture;
use crate::operation_future::SharedStateForOpFuture;

/// `ObjectStoreToThread` is a bridge between `ObjectStore`'s API and the backend thread
/// implemented in LSIO. `ObjectStoreToThread` (will) implement all `ObjectStore` methods
/// and sends the corresponding `Operation` enum variant to the thread for processing.
#[derive(Debug)]
pub struct ObjectStoreToThread {
    config: Arc<Config>,
    worker_thread: WorkerThread,
}

// We can't re-use `object_store::local::Config` because it's private.
#[derive(Debug)]
struct Config {
    root: Url,
}

#[derive(Debug)]
struct WorkerThread {
    handle: thread::JoinHandle<()>,
    sender: mpsc::Sender<Arc<SharedStateForOpFuture>>, // Channel to send ops to the worker thread
}

impl WorkerThread {
    pub fn new(worker_thread_func: fn(mpsc::Receiver<Arc<SharedStateForOpFuture>>)) -> Self {
        let (sender, rx) = mpsc::channel();
        let handle = thread::spawn(move || worker_thread_func(rx));
        Self { handle, sender }
    }

    pub fn clone_and_send_shared_state(&self, operation_future: &OperationFuture) {
        self.sender
            .send(operation_future.get_shared_state().clone())
            .expect("Failed to send message to worker thread!");
    }
}

impl std::fmt::Display for ObjectStoreToThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectStoreToThread({})", self.config.root)
    }
}

impl Default for ObjectStoreToThread {
    fn default() -> Self {
        Self::new(crate::io_uring_local::thread::worker_thread_func)
    }
}

impl ObjectStoreToThread {
    /// Create new filesystem storage with no prefix
    pub fn new(worker_thread_func: fn(mpsc::Receiver<Arc<SharedStateForOpFuture>>)) -> Self {
        Self {
            config: Arc::new(Config {
                root: Url::parse("file:///").unwrap(),
            }),
            worker_thread: WorkerThread::new(worker_thread_func),
        }
    }
}

// This code block will eventually become `impl ObjectStore for ObjectStoreToThread` but,
// for now, I'm just implementing one method at a time (whilst being careful to
// use the exact same function signatures as `ObjectStore`).
impl ObjectStoreToThread {
    // TODO: `ObjectStoreToThread` shouldn't implement `get` because `ObjectStore::get` has a default impl.
    //       Instead, `ObjectStoreToThread` should impl `get_opts` which returns a `Result<GetResult>`.
    //       But I'm keeping things simple for now!
    pub async fn get(&self, location: &Path) -> Result<Bytes> {
        let operation = Operation::Get {
            location: location.clone(),
        };
        let op_future = OperationFuture::new(operation);
        self.worker_thread.clone_and_send_shared_state(&op_future);
        op_future.await
    }
}
