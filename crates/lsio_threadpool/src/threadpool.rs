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
    ///
    /// ```
    /// use lsio_threadpool::threadpool::ThreadPool;
    /// const N_THREADS: usize = 4;
    /// let pool = ThreadPool::new(N_THREADS, |worker_thread| {
    ///     while worker_thread.keep_running() {
    ///         match worker_thread.find_task() {
    ///             Some(task) => process_task(task),
    ///             None => worker_thread.park(),
    ///         }
    ///     }
    /// });
    ///
    /// fn process_task(task: u8) {
    ///     /* do something */
    /// }
    /// ```
    ///
    /// `new` also starts a separate thread which is responsible for tracking parked threads.
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
    use std::{
        collections::HashMap,
        sync::{mpsc::TryRecvError, Mutex},
        thread::ThreadId,
        time::Duration,
    };

    use super::*;

    fn add_one_to_hash(hash: &Arc<Mutex<HashMap<ThreadId, usize>>>) {
        let mut log = hash.lock().unwrap();
        log.entry(thread::current().id())
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }

    #[test]
    fn test_threadpool() {
        const N_THREADS: usize = 4;
        const MULTIPLIER: usize = 8;
        const N_TASKS: usize = N_THREADS * MULTIPLIER;

        let (output_tx, output_rx) = mpsc::channel::<usize>();

        // This HashMap maps from ThreadId to the number of times that thread gets Some(task).
        let n_tasks_per_thread = Arc::new(Mutex::new(HashMap::new()));

        // This HashMap maps from ThreadId to the number of times that thread has parked.
        let n_parks_per_thread = Arc::new(Mutex::new(HashMap::new()));

        let pool = ThreadPool::new(N_THREADS, {
            let n_tasks_per_thread = Arc::clone(&n_tasks_per_thread);
            let n_parks_per_thread = Arc::clone(&n_parks_per_thread);
            move |worker_thread: WorkerThread<usize>| {
                while worker_thread.keep_running() {
                    match worker_thread.find_task() {
                        Some(task) => {
                            output_tx.send(task).unwrap();
                            add_one_to_hash(&n_tasks_per_thread);
                            // Give other threads a chance to do work. Without this `sleep`,
                            // one thread tends to to the majority of the work!
                            thread::sleep(Duration::from_micros(1));
                        }
                        None => {
                            add_one_to_hash(&n_parks_per_thread);
                            worker_thread.park();
                        }
                    };
                }
            }
        });

        // Push tasks onto the global injector queue:
        for i in 0..N_TASKS {
            if i % N_THREADS == 0 {
                // Wait a moment to let the worker threads park, to check they wake up again!
                // Also wait at the start, to let the worker threads "come up".
                thread::sleep(Duration::from_millis(10));
            }
            pool.push(i);
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

        // Check the n_tasks_per_thread and n_parks_per_thread statistics:
        let unwrap_and_check_len = |log: Arc<Mutex<HashMap<ThreadId, usize>>>| {
            let log = Mutex::into_inner(Arc::into_inner(log).unwrap()).unwrap();
            assert_eq!(log.len(), N_THREADS);
            log
        };
        let n_tasks_per_thread = unwrap_and_check_len(n_tasks_per_thread);
        let n_parks_per_thread = unwrap_and_check_len(n_parks_per_thread);

        const MIN_TASKS_PER_THREAD: usize = 2;
        for (thread_id, n_tasks) in n_tasks_per_thread.iter() {
            assert!(
                *n_tasks >= MIN_TASKS_PER_THREAD,
                "{thread_id:?} only did {n_tasks} tasks, which is < the threshold {MIN_TASKS_PER_THREAD} tasks!"
            );
        }
        for (thread_id, n_parks) in n_parks_per_thread.iter() {
            assert!(
                *n_parks == MULTIPLIER || *n_parks == MULTIPLIER + 1,
                "{thread_id:?} did not park the correct number of times!"
            );
        }
    }
}
