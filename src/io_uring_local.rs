use std::sync::Arc;
use object_store::local::LocalFileSystem;
use url::Url;

#[derive(Debug)]
pub struct IoUringLocal {
    config: Arc<Config>,

    // Used so we can delegate method calls to LocalFileSystem.
    local_file_system: LocalFileSystem,
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
            local_file_system: LocalFileSystem::new(),
        }
    }
}

