use bytes::Bytes;
use object_store::{path::Path, Result};
use std::sync::{mpsc, Arc};
use std::thread;
use url::Url;

use crate::operation::Operation;
use crate::operation_future::OperationFuture;
use crate::operation_future::SharedStateForOpFuture;

/// `ObjectStoreToThread` is a bridge between `ObjectStore`'s API and the backend thread
/// implemented in LSIO. `ObjectStoreToThread` (will) implement all `ObjectStore` methods
/// and sends the corresponding `Operation` enum variant to the thread for processing.
#[derive(Debug)]
pub struct ObjectStoreToThread {
    config: Arc<Config>,

    // Different `ObjectStore` methods have different return types.
    // We have a generic `OperationFuture` which is generic over the return type.
    // The channel to the worker thread is typed, and is different for different return types.
    // So we need different channels for different return types.
    // TODO: Think if there's a more elegant way to do this? Maybe by having the
    // `WorkerThread.sender` be a `Sender<Arc<SharedStateForOpFuture<dyn OutputType>>>`
    // where `OutputType` is a marker trait for valid outputs?
    // Although, that said, maybe we _want_ different threads for different operations?
    // So we could be `stat`ing whilst `get`ing?
    thread_for_get_op: WorkerThread<Result<Bytes>>,
}

// We can't re-use `object_store::local::Config` because it's private.
#[derive(Debug)]
struct Config {
    root: Url,
}

#[derive(Debug)]
struct WorkerThread<Output> {
    handle: thread::JoinHandle<()>,
    sender: mpsc::Sender<Arc<SharedStateForOpFuture<Output>>>, // Channel to send ops to the worker thread
}

impl<Output> WorkerThread<Output>
where
    Output: Send + Sync,
{
    pub fn new(
        worker_thread_func: fn(mpsc::Receiver<Arc<SharedStateForOpFuture<Output>>>),
    ) -> Self {
        let (sender, rx) = mpsc::channel();
        let handle = thread::spawn(move || worker_thread_func(rx));
        Self { handle, sender }
    }

    pub fn clone_and_send_shared_state(&self, operation_future: &OperationFuture<Output>) {
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
    pub fn new(
        func_for_get_thread: fn(mpsc::Receiver<Arc<SharedStateForOpFuture<Result<Bytes>>>>),
    ) -> Self {
        Self {
            config: Arc::new(Config {
                root: Url::parse("file:///").unwrap(),
            }),
            thread_for_get_op: WorkerThread::new(func_for_get_thread),
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
        let op_future = OperationFuture::<Result<Bytes>>::new(operation);
        self.thread_for_get_op
            .clone_and_send_shared_state(&op_future);
        op_future.await
    }
}
