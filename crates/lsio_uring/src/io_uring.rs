use std::{ffi::CString, os::unix::ffi::OsStrExt};

use crate::get_ranges::GetRanges;
use crate::operation::Operation;
use crate::worker::UringWorker;
use lsio_io::{Completion, Output, Reader};
use lsio_threadpool::{ThreadPool, WorkerThread};

pub struct IoUring {
    threadpool: ThreadPool<Operation>,
    output_rx: crossbeam_channel::Receiver<anyhow::Result<Output>>,
}

impl IoUring {
    pub fn new(n_worker_threads: usize) -> Self {
        let (output_tx, output_rx) = crossbeam_channel::bounded(1_024);
        Self {
            threadpool: ThreadPool::new(
                n_worker_threads,
                move |worker_thread: WorkerThread<Operation>| {
                    let mut uring_worker = UringWorker::new(worker_thread, output_tx.clone());
                    uring_worker.run();
                },
            ),
            output_rx,
        }
    }
}

impl Completion for IoUring {
    fn completion(&self) -> &crossbeam_channel::Receiver<anyhow::Result<Output>> {
        &self.output_rx
    }
}

impl Reader for IoUring {
    fn get_ranges(
        &mut self,
        location: &std::path::Path,
        ranges: Vec<std::ops::Range<isize>>,
        user_data: Vec<u64>,
    ) -> anyhow::Result<()> {
        let location = CString::new(location.as_os_str().as_bytes())
            .expect("Failed to convert path '{path}' to CString");
        let task = Operation::GetRanges(GetRanges::new(location, ranges, user_data));
        self.threadpool.push(task);
        Ok(())
    }
}
