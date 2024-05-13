use std::{
    iter,
    sync::{atomic::Ordering::Relaxed, Arc},
    thread,
};

use crossbeam::deque;

use crate::threadpool::{ParkManagerCommand, Shared};

/// This object doesn't implement the actual worker loop.
pub struct WorkerThread<T>
where
    T: Send,
{
    shared: Shared<T>,

    /// Queues for implementing work-stealing:
    local_queue: deque::Worker<T>,
    stealers: Arc<Vec<deque::Stealer<T>>>,
}

impl<T> WorkerThread<T>
where
    T: Send,
{
    pub fn new(
        shared: Shared<T>,
        local_queue: deque::Worker<T>,
        stealers: Arc<Vec<deque::Stealer<T>>>,
    ) -> Self {
        Self {
            shared,
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
                self.shared
                    .injector
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
        self.shared
            .chan_to_park_manager
            .send(ParkManagerCommand::ThreadIsParked(thread::current()))
            .unwrap();
        thread::park();
    }

    pub fn maybe_unpark_other_threads(&self) {
        let n = self.local_queue.len();
        // TODO: Also check `at_least_one_thread_is_parked`.
        if self.shared.at_least_one_thread_is_parked.load(Relaxed) && n > 1 {
            self.shared
                .chan_to_park_manager
                .send(ParkManagerCommand::WakeAtMostNThreads(n as _))
                .unwrap();
        }
    }
}
