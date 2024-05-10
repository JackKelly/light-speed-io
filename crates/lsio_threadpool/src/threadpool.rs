use std::thread::{self, ThreadId};

pub(crate) enum ThreadPoolCommand {
    WakeAtMostNThreads(u32),
    ThreadIsParked(ThreadId),
}

pub struct ThreadPool {
    thread_handles: Vec<thread::JoinHandle<()>>,
}
