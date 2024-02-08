use bytes::Bytes;
use object_store::{path::Path, Result};
use std::sync::{mpsc, Arc};
use std::thread;
use url::Url;

use crate::io_uring_local;
use crate::operation::{Operation, OperationWithCallback};
use crate::operation_future::OperationFuture;

/// `ObjectStoreAdapter` is a bridge between `ObjectStore`'s API and the backend thread
/// implemented in LSIO. `ObjectStoreAdapter` (will) implement all `ObjectStore` methods
/// and sends the corresponding `Operation` enum variant to the thread for processing.
#[derive(Debug)]
pub struct ObjectStoreAdapter {
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
    sender: mpsc::Sender<OperationWithCallback>, // Channel to send ops to the worker thread
}

impl WorkerThread {
    pub fn new(worker_thread_func: fn(mpsc::Receiver<OperationWithCallback>)) -> Self {
        let (sender, rx) = mpsc::channel();
        let handle = thread::spawn(move || worker_thread_func(rx));
        Self { handle, sender }
    }

    pub fn send(&self, op_with_output: OperationWithCallback) {
        self.sender
            .send(op_with_output)
            .expect("Failed to send message to worker thread!");
    }
}

impl std::fmt::Display for ObjectStoreAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectStoreAdapter({})", self.config.root)
    }
}

impl Default for ObjectStoreAdapter {
    fn default() -> Self {
        Self::new(io_uring_local::worker_thread_func)
    }
}

impl ObjectStoreAdapter {
    /// Create new filesystem storage with no prefix
    pub fn new(func_for_get_thread: fn(mpsc::Receiver<OperationWithCallback>)) -> Self {
        Self {
            config: Arc::new(Config {
                root: Url::parse("file:///").unwrap(),
            }),
            worker_thread: WorkerThread::new(func_for_get_thread),
        }
    }
}

// This code block will eventually become `impl ObjectStore for ObjectStoreAdapter` but,
// for now, I'm just implementing one method at a time (whilst being careful to
// use the exact same function signatures as `ObjectStore`).
impl ObjectStoreAdapter {
    // TODO: `ObjectStoreAdapter` shouldn't implement `get` because `ObjectStore::get` has a default impl.
    //       Instead, `ObjectStoreAdapter` should impl `get_opts` which returns a `Result<GetResult>`.
    //       But I'm keeping things simple for now!
    pub async fn get(&self, location: &Path) -> Result<Bytes> {
        let operation = Operation::Get {
            location: location.clone(), // TODO: Pass in a reference?
            buffer: None,
            fd: None,
        };
        let (op_future, op_with_output) = OperationFuture::new(operation);
        self.worker_thread.send(op_with_output);
        match op_future.await {
            Operation::Get { buffer, .. } => buffer.unwrap().map(|buf| Bytes::from(buf)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_with_io_uring_local() {
        let filename = Path::from("/home/jack/dev/rust/light-speed-io/README.md");
        let store = ObjectStoreAdapter::default();
        let b = store.get(&filename);
        println!("{:?}", std::str::from_utf8(&b.await.unwrap()[..]).unwrap());
    }
}
