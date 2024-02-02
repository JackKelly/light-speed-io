use bytes::Bytes;
use object_store::{path::Path, Result};
use std::sync::{mpsc, Arc};
use std::thread;
use url::Url;

use crate::operation::{OpType, OperationWithOutput};
use crate::operation_future::OperationFuture;

/// `ObjectStoreToThread` is a bridge between `ObjectStore`'s API and the backend thread
/// implemented in LSIO. `ObjectStoreToThread` (will) implement all `ObjectStore` methods
/// and sends the corresponding `Operation` enum variant to the thread for processing.
#[derive(Debug)]
pub struct ObjectStoreToThread<F, O> {
    config: Arc<Config>,
    worker_thread: WorkerThread<F, O>,
}

// We can't re-use `object_store::local::Config` because it's private.
#[derive(Debug)]
struct Config {
    root: Url,
}

#[derive(Debug)]
struct WorkerThread<F, O>
where
    F: FnOnce(&OpType, O),
{
    handle: thread::JoinHandle<()>,
    sender: mpsc::Sender<OperationWithOutput<F, O>>, // Channel to send ops to the worker thread
}

impl<F, O> WorkerThread<F, O>
where
    F: FnOnce(&OpType, O),
{
    pub fn new(worker_thread_func: fn(mpsc::Receiver<OperationWithOutput<F, O>>)) -> Self {
        let (sender, rx) = mpsc::channel();
        let handle = thread::spawn(move || worker_thread_func(rx));
        Self { handle, sender }
    }

    pub fn send(&self, op_with_output: OperationWithOutput<F, O>) {
        self.sender
            .send(op_with_output)
            .expect("Failed to send message to worker thread!");
    }
}

impl<F, O> std::fmt::Display for ObjectStoreToThread<F, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectStoreToThread({})", self.config.root)
    }
}

impl<F, O> Default for ObjectStoreToThread<F, O> {
    fn default() -> Self {
        Self::new(crate::io_uring_local::thread::worker_thread_func)
    }
}

impl<F, O> ObjectStoreToThread<F, O>
where
    F: FnOnce(&OpType, O),
{
    /// Create new filesystem storage with no prefix
    pub fn new(func_for_get_thread: fn(mpsc::Receiver<OperationWithOutput<F, O>>)) -> Self {
        Self {
            config: Arc::new(Config {
                root: Url::parse("file:///").unwrap(),
            }),
            worker_thread: WorkerThread::new(func_for_get_thread),
        }
    }
}

// This code block will eventually become `impl ObjectStore for ObjectStoreToThread` but,
// for now, I'm just implementing one method at a time (whilst being careful to
// use the exact same function signatures as `ObjectStore`).
impl<F, O> ObjectStoreToThread<F, O> {
    // TODO: `ObjectStoreToThread` shouldn't implement `get` because `ObjectStore::get` has a default impl.
    //       Instead, `ObjectStoreToThread` should impl `get_opts` which returns a `Result<GetResult>`.
    //       But I'm keeping things simple for now!
    pub async fn get(&self, location: &Path) -> Result<Bytes> {
        let operation = OpType::Get {
            location: location.clone(),
        };
        let (op_future, op_with_output) =
            OperationFuture::<Result<Bytes>>::new::<FnOnce(&OpType, Result<Bytes>)>(operation);
        self.worker_thread.sender.send(op_with_output).expect("oo");
        op_future.await
    }
}
