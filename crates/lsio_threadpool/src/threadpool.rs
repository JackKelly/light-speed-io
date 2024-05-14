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
    worker_thread_handles: Vec<JoinHandle<()>>,
    park_manager_thread_handle: Option<JoinHandle<()>>,
    shared: SharedState<T>,
}

impl<T> ThreadPool<T>
where
    T: Send + 'static,
{
    /// Starts a new threadpool with `n_worker_threads` threads. Also clones and runs `op` on
    /// each thread. `op` takes one argument: a `WorkerThread<T>` which provides helpful methods
    /// for the task.
    ///
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

        // Spawn ParkManager thread:
        let park_manager_thread_handle = Some(ParkManager::start(
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
        let worker_thread_handles = (0..n_worker_threads)
            .map(|_| {
                let work_stealer = WorkerThread::new(
                    shared.clone(),
                    local_queues.pop().unwrap(),
                    Arc::clone(&stealers),
                );

                let op_clone = op.clone();
                thread::spawn(move || (op_clone)(work_stealer))
            })
            .collect();

        Self {
            worker_thread_handles,
            park_manager_thread_handle,
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
        // Stop and join the worker threads:
        self.shared.keep_running.store(false, Relaxed);
        for handle in self.worker_thread_handles.drain(..) {
            handle.thread().unpark();
            handle.join().unwrap();
        }

        // Stop and join the ParkManager:
        self.shared
            .chan_to_park_manager
            .send(ParkManagerCommand::Stop)
            .unwrap();
        self.park_manager_thread_handle
            .take()
            .unwrap()
            .join()
            .unwrap();
    }
}

#[cfg(test)]

mod tests {
    use std::{sync::mpsc::TryRecvError, time::Duration};

    use super::*;

    #[test]
    fn test_threadpool() {
        const N_THREADS: usize = 4;
        const N_TASKS: usize = 32;

        let (output_tx, output_rx) = mpsc::channel::<usize>();

        let pool = ThreadPool::new(N_THREADS, move |worker_thread: WorkerThread<usize>| {
            while worker_thread.keep_running() {
                match worker_thread.find_task() {
                    Some(task) => {
                        println!("{:?} task:{task}", thread::current().id());
                        output_tx.send(task).unwrap();
                    }
                    None => {
                        println!("{:?} PARKING", thread::current().id());
                        worker_thread.park();
                    }
                };
            }
        });

        // Wait a moment for all the threads to "come up":
        thread::sleep(Duration::from_millis(10));

        // Push tasks onto the global injector queue:
        for i in 0..N_TASKS {
            pool.push(i);
            if i == N_TASKS / 2 {
                // Wait a moment to let the worker threads park, to check they wake up again!
                thread::sleep(Duration::from_millis(10));
            }
        }

        // Collect outputs and stop the work when all the outputs arrive:
        let mut outputs: Vec<usize> = output_rx.iter().take(N_TASKS).collect();
        outputs.sort();
        assert!(outputs.into_iter().eq(0..N_TASKS));
        assert!(matches!(
            output_rx.try_recv().unwrap_err(),
            TryRecvError::Empty
        ));
        drop(pool);
        assert!(matches!(
            output_rx.try_recv().unwrap_err(),
            TryRecvError::Disconnected
        ));
    }
}
