use std::{
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self},
        Arc,
    },
    thread::{self, JoinHandle},
};

use crossbeam::deque;

use crate::{
    park_manager::{ParkManager, ParkManagerCommand},
    shared_state::SharedState,
    worker::WorkerThread,
};

#[derive(Debug)]
pub struct ThreadPool<T>
where
    T: Send,
{
    /// thread_handles includes the worker threads and the ParkManager thread
    thread_handles: Vec<JoinHandle<()>>,
    shared: SharedState<T>,
}

impl<T> ThreadPool<T>
where
    T: Send + 'static,
{
    /// Starts threadpool. Each thread will run `op` exactly once.
    /// Typically, `op` will begin with any necessary setup (e.g. instantiating objects for that
    /// thread) and will then enter a loop, something like:
    /// `while keep_running.load(Relaxed) { /* do work */ }`.
    /// `new` also starts a separate thread which is responsible for tracking which threads are
    /// parked
    pub fn new<OP>(n_worker_threads: usize, op: OP) -> Self
    where
        OP: Fn(WorkerThread<T>) + Send + Clone + 'static,
    {
        let (chan_to_park_manager, rx_for_park_manager) = mpsc::channel();
        let shared = SharedState {
            injector: Arc::new(deque::Injector::new()),
            keep_running: Arc::new(AtomicBool::new(true)),
            chan_to_park_manager,
            at_least_one_thread_is_parked: Arc::new(AtomicBool::new(false)),
        };

        // thread_handles includes all the worker threads and the Park Manager.
        let mut thread_handles = Vec::with_capacity(n_worker_threads + 1);

        // Spawn ParkManager thread:
        thread_handles.push(ParkManager::start(
            rx_for_park_manager,
            Arc::clone(&shared.at_least_one_thread_is_parked),
            n_worker_threads,
        ));

        // Create work stealing queues:
        let mut local_queues: Vec<deque::Worker<T>> = (0..n_worker_threads)
            .map(|_| deque::Worker::new_fifo())
            .collect();
        let stealers: Arc<Vec<deque::Stealer<T>>> = Arc::new(
            local_queues
                .iter()
                .map(|local_queue| local_queue.stealer())
                .collect(),
        );

        // Spawn worker threads:
        thread_handles.extend((0..n_worker_threads).map(|_| {
            let work_stealer = WorkerThread::new(
                shared.clone(),
                local_queues.pop().unwrap(),
                Arc::clone(&stealers),
            );

            let op_clone = op.clone();
            thread::spawn(move || (op_clone)(work_stealer))
        }));

        Self {
            thread_handles,
            shared,
        }
    }

    pub fn push(&self, task: T) {
        self.shared.injector.push(task);
        self.shared.unpark_at_most_n_threads(1);
    }
}

impl<T> Drop for ThreadPool<T>
where
    T: Send,
{
    fn drop(&mut self) {
        self.shared.keep_running.store(false, Relaxed);
        self.shared
            .chan_to_park_manager
            .send(ParkManagerCommand::Stop)
            .unwrap();
        for handle in self.thread_handles.drain(..) {
            handle.thread().unpark();
            handle.join().unwrap();
        }
    }
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
                    move |mut work_stealer: WorkerThread<usize>, keep_running: &AtomicBool| {
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
