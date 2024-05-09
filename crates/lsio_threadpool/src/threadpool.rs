use std::thread::ThreadId;

pub(crate) enum ThreadPoolCommand {
    WakeAtMostNThreads(u32),
    ThreadIsParked(ThreadId),
}
