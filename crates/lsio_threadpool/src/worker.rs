use std::iter;

use crossbeam::deque;

/// This object doesn't implement the actual worker loop.
pub struct WorkStealer<'a, T>
where
    T: Send,
{
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
        injector: &'a deque::Injector<T>,
        local_queue: deque::Worker<T>,
        stealers: &'a [deque::Stealer<T>],
    ) -> Self {
        Self {
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
}
