use bytes::Bytes;
use object_store::{path::Path, Result};
use std::sync::Arc;
use url::Url;

#[derive(Debug)]
pub struct IoUringLocal {
    config: Arc<Config>,
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
        Self {
            config: Arc::new(Config {
                root: Url::parse("file:///").unwrap(),
            }),
        }
    }
}

// This block will eventually become `impl ObjectStore for IoUringLocal` but,
// for now, I'm just implementing one method at a time (whilst being careful to
// use the exact same function signatures as `ObjectStore`).
impl IoUringLocal {
    // TODO: `IoUringLocal` shouldn't implement `get` because `ObjectStore::get` has a default impl.
    // Instead, `IoUringLocal` should impl `get_opts` which returns a `Result<GetResult>`.
    // But I'm keeping things simple for now!
    pub async fn get(&mut self, location: &Path) -> Result<Bytes> {}
}
