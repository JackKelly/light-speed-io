use std::{
    iter,
    sync::{
        atomic::{self, AtomicBool},
        mpsc,
    },
    thread,
};

use crossbeam::deque;

use crate::threadpool::ThreadPoolCommand::{self, *};

/// This object doesn't implement the actual worker loop.
/// Instead, a `WorkerThread` instance should be part of a user-defined
/// object. And that object should implement the worker loop, and can call the
/// helper methods in the `WorkerThread`.
pub struct WorkerThread<'a, T> {
    /// Shared across threads. Set to `true` to stop threads gracefully.
    stop: AtomicBool,

    /// Send instructions to the `ThreadPool`.
    channel_to_threadpool: mpsc::Sender<ThreadPoolCommand>,

    /// Queues for implementing work-stealing:
    global_queue: &'a deque::Injector<T>,
    local_queue: deque::Worker<T>,
    stealers: &'a [deque::Stealer<T>],
}

impl<'a, T> WorkerThread<'a, T> {
    pub fn new(
        stop: AtomicBool,
        threadpool_chan: mpsc::Sender<ThreadPoolCommand>,
        global_queue: &'a deque::Injector<T>,
        stealers: &'a [deque::Stealer<T>],
    ) -> Self {
        Self {
            channel_to_threadpool: threadpool_chan,
            stop,
            global_queue,
            local_queue: deque::Worker::new_fifo(),
            stealers,
        }
    }

    pub fn stop(&self) -> bool {
        self.stop.load(atomic::Ordering::Relaxed)
    }

    pub fn maybe_wake_other_threads(&self) {
        if self.local_queue.len() > 1 {
            let n = self.local_queue.len();
            self.channel_to_threadpool
                .send(WakeAtMostNThreads(n as _))
                .unwrap();
        }
    }

    pub fn park(&self) {
        self.channel_to_threadpool
            .send(ThreadIsParked(thread::current().id()))
            .unwrap();
        thread::park();
    }

    pub fn find_task(
        local: &deque::Worker<T>,
        global: &deque::Injector<T>,
        stealers: &[deque::Stealer<T>],
    ) -> Option<T> {
        // Pop a task from the local queue, if not empty.
        local.pop().or_else(|| {
            // Otherwise, we need to look for a task elsewhere.
            iter::repeat_with(|| {
                // Try stealing a batch of tasks from the global queue.
                global
                    .steal_batch_and_pop(local)
                    // Or try stealing a task from one of the other threads.
                    .or_else(|| stealers.iter().map(|s| s.steal()).collect())
            })
            // Loop while no task was stolen and any steal operation needs to be retried.
            .find(|s| !s.is_retry())
            // Extract the stolen task, if there is one.
            .and_then(|s| s.success())
        })
    }
}
