use core::sync;
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
/// Instead, a `WorkStealer` instance should be part of a user-defined
/// object. And that object should implement the worker loop, and can call the
/// helper methods in the `WorkStealer`.
pub struct WorkStealer<'a, T>
where
    T: Send,
{
    /// Shared across threads. Set to `true` to stop threads gracefully.
    stop: &'a AtomicBool,

    /// Send instructions to the `ThreadPool`.
    channel_to_threadpool: mpsc::Sender<ThreadPoolCommand>,

    /// Queues for implementing work-stealing:
    global_queue: &'a deque::Injector<T>,
    local_queue: deque::Worker<T>,
    stealers: &'a [deque::Stealer<T>],
}

impl<'a, T> WorkStealer<'a, T>
where
    T: Send,
{
    pub fn new(
        stop: &'a AtomicBool,
        channel_to_threadpool: mpsc::Sender<ThreadPoolCommand>,
        global_queue: &'a deque::Injector<T>,
        local_queue: deque::Worker<T>,
        stealers: &'a [deque::Stealer<T>],
    ) -> Self {
        Self {
            stop,
            channel_to_threadpool,
            global_queue,
            local_queue,
            stealers,
        }
    }

    pub fn ask_to_wake_other_threads(&self) {
        let n = self.local_queue.len();
        if n > 1 {
            self.channel_to_threadpool
                .send(WakeAtMostNThreads(n as _))
                .unwrap();
        }
    }

    pub fn stop(&self) -> bool {
        self.stop.load(atomic::Ordering::Relaxed)
    }

    pub fn park(&self) {
        self.channel_to_threadpool
            .send(ThreadIsParked(thread::current().id()))
            .unwrap();
        thread::park();
    }

    pub fn find_task(&mut self) -> Option<T> {
        // Adapted from https://docs.rs/crossbeam-deque/latest/crossbeam_deque/#examples

        // Pop a task from the local queue, if not empty.
        self.local_queue.pop().or_else(|| {
            // Otherwise, we need to look for a task elsewhere.
            iter::repeat_with(|| {
                // Try stealing a batch of tasks from the global queue.
                self.global_queue
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

#[cfg(test)]

mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_work_stealer() {
        let stop = AtomicBool::new(false);
        let (channel_to_threadpool, _threadpool_rx) = mpsc::channel();
        const N_THREADS: usize = 4;
        const N_TASKS: usize = N_THREADS * 10;

        // Create work stealing queues:
        let global_queue: deque::Injector<usize> = deque::Injector::new();
        let mut local_queues: Vec<Option<deque::Worker<usize>>> = (0..N_THREADS)
            .map(|_| Some(deque::Worker::new_fifo()))
            .collect();
        let stealers: Vec<_> = local_queues
            .iter()
            .map(|local_queue| local_queue.as_ref().unwrap().stealer())
            .collect();

        thread::scope(|s| {
            let (output_tx, output_rx) = mpsc::channel();

            // Spawn N_THREADS threads
            for i in 0..N_THREADS {
                let mut work_stealer = WorkStealer::new(
                    &stop,
                    channel_to_threadpool.clone(),
                    &global_queue,
                    local_queues[i].take().unwrap(),
                    &stealers,
                );

                let my_output_tx = output_tx.clone();
                s.spawn(move || {
                    while !work_stealer.stop() {
                        match work_stealer.find_task() {
                            Some(task) => {
                                println!("thread: {:?}; task:{task}", thread::current().id());
                                my_output_tx.send(task).unwrap();
                            }
                            None => continue,
                        };
                    }
                });
            }

            drop(output_tx);

            // Push "tasks" onto the global queue:
            for i in 0..N_TASKS {
                global_queue.push(i);
            }

            // Kill all the worker threads after a little while:
            let stop_ref = &stop;
            s.spawn(move || {
                let mut outputs = Vec::with_capacity(N_TASKS);
                for _ in 0..N_TASKS {
                    outputs.push(output_rx.recv().unwrap());
                }
                stop_ref.store(true, atomic::Ordering::Relaxed);
                outputs.sort();
                assert!(outputs.into_iter().eq(0..N_TASKS));
                assert!(output_rx.try_recv().is_err());
            });
        });
    }
}
