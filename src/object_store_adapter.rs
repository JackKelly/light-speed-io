use bytes::Bytes;
use object_store::path::Path as ObjectStorePath;
use snafu::{ensure, Snafu};
use std::ffi::CString;
use std::future::Future;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{mpsc, Arc};
use std::thread;
use url::Url;

use crate::io_uring_local;
use crate::operation::{Operation, OperationWithOutput, Output};

/// A specialized `Error` for filesystem object store-related errors
/// From `object_store::local`
#[derive(Debug, Snafu)]
#[allow(missing_docs)]
pub(crate) enum Error {
    #[snafu(display("Unable to convert URL \"{}\" to filesystem path", url))]
    InvalidUrl {
        url: Url,
    },

    NotFound {
        path: PathBuf,
        source: io::Error,
    },

    AlreadyExists {
        path: String,
        source: io::Error,
    },

    #[snafu(display("Filenames containing trailing '/#\\d+/' are not supported: {}", path))]
    InvalidPath {
        path: String,
    },
}

// From `object_store::local`
impl From<Error> for object_store::Error {
    fn from(source: Error) -> Self {
        match source {
            Error::NotFound { path, source } => Self::NotFound {
                path: path.to_string_lossy().to_string(),
                source: source.into(),
            },
            Error::AlreadyExists { path, source } => Self::AlreadyExists {
                path,
                source: source.into(),
            },
            _ => Self::Generic {
                store: "ObjectStoreAdapter",
                source: Box::new(source),
            },
        }
    }
}

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

// From `object_store::local`
impl Config {
    /// Return an absolute filesystem path of the given file location
    fn path_to_filesystem(&self, location: &ObjectStorePath) -> anyhow::Result<PathBuf> {
        ensure!(
            is_valid_file_path(location),
            InvalidPathSnafu {
                path: location.as_ref()
            }
        );
        self.prefix_to_filesystem(location)
    }

    /// Return an absolute filesystem path of the given location
    fn prefix_to_filesystem(&self, location: &ObjectStorePath) -> anyhow::Result<PathBuf> {
        let mut url = self.root.clone();
        url.path_segments_mut()
            .expect("url path")
            // technically not necessary as Path ignores empty segments
            // but avoids creating paths with "//" which look odd in error messages.
            .pop_if_empty()
            .extend(location.parts());

        url.to_file_path()
            .map_err(|_| Error::InvalidUrl { url }.into())
    }
}

#[derive(Debug)]
struct WorkerThread {
    handle: thread::JoinHandle<()>,
    sender: mpsc::Sender<OperationWithOutput>, // Channel to send ops to the worker thread
}

impl WorkerThread {
    pub fn new(worker_thread_func: fn(mpsc::Receiver<OperationWithOutput>)) -> Self {
        let (sender, rx) = mpsc::channel();
        let handle = thread::spawn(move || worker_thread_func(rx));
        Self { handle, sender }
    }

    pub fn send(&self, op_with_chan: OperationWithOutput) {
        self.sender
            .send(op_with_chan)
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
    pub fn new(func_for_get_thread: fn(mpsc::Receiver<OperationWithOutput>)) -> Self {
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
    pub fn get(
        &self,
        location: &ObjectStorePath,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Bytes>> + Send + Sync>> {
        let path = self.config.path_to_filesystem(location).unwrap();
        let path = CString::new(path.as_os_str().as_bytes())
            .expect("Failed to convert path '{path}' to CString.");

        let operation = Operation::Get {
            path,
            fixed_fd: None,
        };

        let (op_with_chan, rx) = OperationWithOutput::new(operation);

        self.worker_thread.send(op_with_chan);

        Box::pin(async {
            rx.await.unwrap().map(|o| {
                let Output::Get { buffer } = o;
                Bytes::from(buffer)
            })
        })
    }
}

// From `object_store::local`
fn is_valid_file_path(path: &ObjectStorePath) -> bool {
    match path.filename() {
        Some(p) => match p.split_once('#') {
            Some((_, suffix)) if !suffix.is_empty() => {
                // Valid if contains non-digits
                !suffix.as_bytes().iter().all(|x| x.is_ascii_digit())
            }
            _ => true,
        },
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use nix::errno::Errno;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_get_with_io_uring_local() {
        let filenames = vec![
            ObjectStorePath::from("/home/jack/dev/rust/light-speed-io/README.md"),
            ObjectStorePath::from("/home/jack/dev/rust/light-speed-io/Cargo.toml"),
            ObjectStorePath::from("/this/path/does/not/exist"),
        ];
        let store = ObjectStoreAdapter::default();
        let mut futures = Vec::new();
        for filename in &filenames {
            futures.push(store.get(filename));
        }

        for (i, future) in futures.into_iter().enumerate() {
            let result = future.await;
            if i < 2 {
                let b = result.unwrap();
                println!("Loaded {} bytes", b.len());
                println!("{:?}", std::str::from_utf8(&b[..]).unwrap());
            } else {
                let err = result.unwrap_err();
                dbg!(&err);
                println!("err={err}. err.root_cause()={}", err.root_cause());
                assert!(err.is::<nix::Error>());
                assert_eq!(err.downcast_ref::<Errno>().unwrap(), &Errno::ENOENT);
            }
        }
    }
}
