#![doc = include_str!("../README.md")]

use get_ranges::GetRanges;
use lsio_io::{Output, Reader};
use lsio_threadpool::{ThreadPool, WorkerThread};
use operation::Operation;

pub(crate) mod close;
pub(crate) mod get_range;
pub(crate) mod get_ranges;
pub(crate) mod opcode;
pub(crate) mod open_file;
pub(crate) mod operation;
pub(crate) mod sqe;
pub(crate) mod tracker;
pub(crate) mod user_data;
pub(crate) mod worker;

struct IoUring {
    threadpool: ThreadPool<Operation>,
    output_rx: crossbeam_channel::Receiver<Output>,
}

impl IoUring {
    fn new(n_worker_threads: usize) -> Self {
        Self {
            threadpool: ThreadPool::new(
                n_worker_threads,
                |worker_thread: WorkerThread<Operation>| todo!(),
            ),
            output_rx,
        }
    }
}

impl Reader for IoUring {
    fn get_ranges(
        &mut self,
        location: std::path::Path,
        ranges: Vec<std::ops::Range<isize>>,
        user_data: Vec<u64>,
    ) -> anyhow::Result<()> {
        let task = Operation::GetRanges(GetRanges::new(location, ranges, user_data));
        self.threadpool.push(task);
        Ok(())
    }
}
