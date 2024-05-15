use std::{
    iter,
    sync::{atomic::Ordering::Relaxed, Arc},
    thread,
};

use crossbeam_deque as deque;

use crate::{park_manager::ParkManagerCommand, shared_state::SharedState};

/// Provides methods that allow user-defined closures to find new tasks to work on,
/// submit new tasks, park this thread, and check if the closure should continue looping.
///
/// Uses do not construct `WorkerThread`s. Instead, [`ThreadPool::new`](crate::ThreadPool::new)
/// creates one `WorkerThread` per thread, and passes that thread's `WorkerThread` to the
/// user-supplied closure for that thread.
pub struct WorkerThread<T>
where
    T: Send,
{
    shared: SharedState<T>,

    /// Queues for implementing work-stealing:
    local_queue: deque::Worker<T>,
    stealers: Arc<Vec<deque::Stealer<T>>>,
}

impl<T> WorkerThread<T>
where
    T: Send,
{
    pub(crate) fn new(
        shared: SharedState<T>,
        local_queue: deque::Worker<T>,
        stealers: Arc<Vec<deque::Stealer<T>>>,
    ) -> Self {
        Self {
            shared,
            local_queue,
            stealers,
        }
    }

    /// Get the next task to work on. This function never blocks.
    pub fn find_task(&self) -> Option<T> {
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

    /// Returns true if the task should keep running.
    pub fn keep_running(&self) -> bool {
        self.shared.keep_running.load(Relaxed)
    }

    /// Park this thread.
    ///
    /// Before parking, this function will register this thread with the `ParkManager`
    /// so that this thread can be automatically unparked when necessary.
    pub fn park(&self) {
        self.shared
            .chan_to_park_manager
            .send(ParkManagerCommand::ThreadIsParked(thread::current()))
            .unwrap_or_else(|e| {
                panic!(
                    "failed to send ThreadIsParked({:?}) message to ParkManager! {e:?}",
                    thread::current(),
                )
            });
        thread::park();
    }

    /// Push a task onto this thread's local queue of tasks.
    ///
    /// Tasks on the local queue may be stolen by other threads!
    pub fn push(&self, task: T) {
        self.local_queue.push(task);
        self.maybe_unpark_other_threads();
    }

    fn maybe_unpark_other_threads(&self) {
        let n = self.local_queue.len();
        if n > 1 {
            self.shared.unpark_at_most_n_threads(n as _);
        }
    }
}
