use std::{iter, sync::mpsc, thread};

use crossbeam::deque;

use crate::threadpool::ThreadPoolCommand;

/// This object doesn't implement the actual worker loop.
pub struct WorkStealer<'a, T>
where
    T: Send,
{
    tx_to_threadpool: mpsc::Sender<ThreadPoolCommand>,

    /// Queues for implementing work-stealing:
    injector: &'a deque::Injector<T>,
    local_queue: deque::Worker<T>,
    stealers: &'a [deque::Stealer<T>],
}

impl<'a, T> WorkStealer<'a, T>
where
    T: Send,
{
    pub fn new(
        tx_to_threadpool: mpsc::Sender<ThreadPoolCommand>,
        injector: &'a deque::Injector<T>,
        local_queue: deque::Worker<T>,
        stealers: &'a [deque::Stealer<T>],
    ) -> Self {
        Self {
            tx_to_threadpool,
            injector,
            local_queue,
            stealers,
        }
    }

    pub fn find_task(&mut self) -> Option<T> {
        // Adapted from https://docs.rs/crossbeam-deque/latest/crossbeam_deque/#examples

        // Pop a task from the local queue, if not empty.
        self.local_queue.pop().or_else(|| {
            // Otherwise, we need to look for a task elsewhere.
            iter::repeat_with(|| {
                // Try stealing a batch of tasks from the global queue.
                self.injector
                    .steal_batch_and_pop(&self.local_queue)
                    // Or try stealing a task from one of the other threads.
                    .or_else(|| self.stealers.iter().map(|s| s.steal()).collect())
            })
            // Loop while no task was stolen and any steal operation needs to be retried.
            .find(|s| !s.is_retry())
            // Extract the stolen task, if there is one.
            .and_then(|s| s.success())
        })
    }

    pub fn push(&self, task: T) {
        self.local_queue.push(task);
        self.maybe_unpark_other_threads();
    }

    pub fn park(&self) {
        self.tx_to_threadpool
            .send(ThreadPoolCommand::ThreadIsParked(thread::current()))
            .unwrap();
        thread::park();
    }

    pub fn maybe_unpark_other_threads(&self) {
        let n = self.local_queue.len();
        // TODO: Also check `at_least_one_thread_is_parked`.
        if n > 1 {
            self.tx_to_threadpool
                .send(ThreadPoolCommand::WakeAtMostNThreads(n as _))
                .unwrap();
        }
    }
}
