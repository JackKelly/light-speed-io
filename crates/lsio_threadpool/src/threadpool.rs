use std::{
    collections::VecDeque,
    sync::{
        atomic::{self, AtomicBool},
        mpsc, Arc,
    },
    thread::{self, Thread},
    time::Duration,
};

use crossbeam::deque;

use crate::worker::WorkStealer;

pub(crate) enum ThreadPoolCommand {
    WakeAtMostNThreads(u32),
    ThreadIsParked(Thread),
}

pub fn threadpool<T, I>(
    n_threads: usize,
    injector: &deque::Injector<I>,
    keep_running: &AtomicBool,
    task: T,
) where
    T: FnMut(WorkStealer<I>, &AtomicBool) + Send + Clone,
    I: Send,
{
    let (tx, rx) = mpsc::channel::<ThreadPoolCommand>();

    // Create work stealing queues:
    let mut local_queues: Vec<Option<deque::Worker<I>>> = (0..n_threads)
        .map(|_| Some(deque::Worker::new_fifo()))
        .collect();
    let stealers: Vec<_> = local_queues
        .iter()
        .map(|local_queue| local_queue.as_ref().unwrap().stealer())
        .collect();

    thread::scope(|s| {
        // Manager thread:
        s.spawn(move || {
            let mut parked_threads = VecDeque::<Thread>::with_capacity(n_threads);
            while keep_running.load(atomic::Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(command) => match command {
                        ThreadPoolCommand::ThreadIsParked(thread) => {
                            parked_threads.push_back(thread);
                        }
                        ThreadPoolCommand::WakeAtMostNThreads(n) => {
                            for _ in 0..n {
                                match parked_threads.pop_front() {
                                    Some(thread) => thread.unpark(),
                                    None => break,
                                }
                            }
                        }
                    },
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                }
            }
        });

        // Worker threads:
        for i in 0..n_threads {
            let work_stealer = WorkStealer::new(
                tx.clone(),
                injector,
                local_queues[i].take().unwrap(),
                &stealers,
            );

            let mut task_clone = task.clone();
            s.spawn(move || (task_clone)(work_stealer, keep_running));
        }

        drop(tx);
    });
}

#[cfg(test)]

mod tests {
    use super::*;

    #[test]
    fn test_threadpool() {
        const N_THREADS: usize = 4;
        const N_TASKS: usize = 32;
        let keep_running = AtomicBool::new(true);
        let injector = deque::Injector::<usize>::new();

        thread::scope(|s| {
            let (output_tx, output_rx) = mpsc::channel::<usize>();

            // Start threadpool of workers:
            s.spawn(|| {
                threadpool(
                    N_THREADS,
                    &injector,
                    &keep_running,
                    move |mut work_stealer: WorkStealer<usize>, keep_running: &AtomicBool| {
                        while keep_running.load(atomic::Ordering::Relaxed) {
                            match work_stealer.find_task() {
                                Some(task) => {
                                    println!("thread: {:?}; task:{task}", thread::current().id());
                                    output_tx.send(task).unwrap();
                                }
                                None => continue,
                            };
                        }
                    },
                );
            });

            // Wait a moment for all the threads to "come up":
            thread::sleep(Duration::from_millis(10));

            // Push tasks onto the global injector queue:
            for i in 0..N_TASKS {
                injector.push(i);
            }

            // Collect outputs and stop the work when all the outputs arrive:
            let keep_running_ref = &keep_running;
            s.spawn(move || {
                let mut outputs = Vec::with_capacity(N_TASKS);
                for _ in 0..N_TASKS {
                    outputs.push(output_rx.recv().unwrap());
                }
                keep_running_ref.store(false, atomic::Ordering::Relaxed);
                outputs.sort();
                assert!(outputs.into_iter().eq(0..N_TASKS));
                assert!(output_rx.try_recv().is_err());
            });
        });
    }
}

// New design ideas:
pub struct ThreadPool<T> {
    injector: deque::Injector<T>,
    keep_running: AtomicBool,
    chan_to_park_manager: mpsc::Sender<ThreadPoolCommand>,
    at_least_one_thread_is_parked: AtomicBool,
}

impl<T> ThreadPool<T> {
    /// Starts threadpool. Each thread will run `task` exactly once.
    /// Typically, `task` will begin with any necessary setup (e.g. instantiating objects for that
    /// thread) and will then enter a loop, something like:
    /// `while keep_running.load(Relaxed) { /* do work */ }`.
    /// `new` also starts a separate thread which is responsible for tracking which threads are
    /// parked (and passes that thread references to keep_running and
    /// at_least_one_thread_is_parked).
    pub fn new<OP>(n_threads: usize, op: OP) -> Self {
        todo!();
        // But how to launch worker threads (which might last longer than `ThreadPool`),
        // whilst still sharing access to `injector` etc.? I think there are two approaches:
        // 1) Move the relevant objects into a single `launcher` thread's stack. And then call
        //    `thread::scope` from the `launcher` thread.
        // 2) Wrap all `ThreadPool` members (except chan_to_park_manager) in `Arc`s.
        //    And clone these when we share with other threads. This does require some heap
        //    allocations. But only once (at setup), and only a tiny number (the number of
        //    threads), so I think it's probably best to use `Arc`s. This also makes it easy to
        //    `clone` entire ThreadPool objects, to share between threads.
    }

    pub fn push(&self, task: T) {
        self.injector.push(task);
        if self
            .at_least_one_thread_is_parked
            .load(atomic::Ordering::Relaxed)
        {
            self.chan_to_park_manager
                .send(ThreadPoolCommand::WakeAtMostNThreads(1))
                .unwrap();
        }
    }
}

impl<T> Drop for ThreadPool<T> {
    fn drop(&mut self) {
        self.keep_running.store(false, atomic::Ordering::Relaxed);
    }
}
