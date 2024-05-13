use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, RecvError},
        Arc,
    },
    thread::{self, JoinHandle, Thread},
};

use crossbeam::deque;

use crate::worker::WorkerThread;

pub(crate) enum ParkManagerCommand {
    WakeAtMostNThreads(u32),
    ThreadIsParked(Thread),
    Stop,
}

struct ParkManager {
    rx: mpsc::Receiver<ParkManagerCommand>,
    at_least_one_thread_is_parked: Arc<AtomicBool>,
    parked_threads: VecDeque<Thread>,
}

impl ParkManager {
    pub(crate) fn start(
        rx: mpsc::Receiver<ParkManagerCommand>,
        at_least_one_thread_is_parked: Arc<AtomicBool>,
        n_worker_threads: usize,
    ) -> JoinHandle<()> {
        let mut park_manager = Self {
            rx,
            at_least_one_thread_is_parked,
            parked_threads: VecDeque::with_capacity(n_worker_threads),
        };
        thread::Builder::new()
            .name("ParkManager".to_string())
            .spawn(move || park_manager.main_loop())
            .expect("Failed to spawn the ParkManager thread!")
    }

    fn main_loop(&mut self) {
        use ParkManagerCommand::*;
        loop {
            match self.rx.recv() {
                Ok(cmd) => match cmd {
                    ThreadIsParked(t) => self.thread_is_parked(t),
                    WakeAtMostNThreads(n) => self.wake_at_most_n_threads(n),
                    Stop => break,
                },
                Err(RecvError) => break,
            }
        }
    }

    fn thread_is_parked(&mut self, t: Thread) {
        self.at_least_one_thread_is_parked.store(true, Relaxed);
        debug_assert!(!self.parked_threads.iter().any(|pt| pt.id() == t.id()));
        self.parked_threads.push_back(t);
    }

    fn wake_at_most_n_threads(&mut self, n: u32) {
        for _ in 0..n {
            match self.parked_threads.pop_front() {
                Some(thread) => thread.unpark(),
                None => break,
            }
        }
        if self.parked_threads.is_empty() {
            self.at_least_one_thread_is_parked.store(false, Relaxed);
        }
    }
}

/// Shared with worker threads
#[derive(Debug)]
pub(crate) struct Shared<T>
where
    T: Send,
{
    pub(crate) injector: Arc<deque::Injector<T>>,
    pub(crate) keep_running: Arc<AtomicBool>,
    pub(crate) chan_to_park_manager: mpsc::Sender<ParkManagerCommand>,
    pub(crate) at_least_one_thread_is_parked: Arc<AtomicBool>,
}

impl<T> Shared<T>
where
    T: Send,
{
    pub(crate) fn unpark_at_most_n_threads(&self, n: u32) {
        if self.at_least_one_thread_is_parked.load(Relaxed) {
            self.chan_to_park_manager
                .send(ParkManagerCommand::WakeAtMostNThreads(n))
                .unwrap();
        }
    }
}

impl<T> Clone for Shared<T>
where
    T: Send,
{
    fn clone(&self) -> Self {
        Self {
            injector: Arc::clone(&self.injector),
            keep_running: Arc::clone(&self.keep_running),
            chan_to_park_manager: self.chan_to_park_manager.clone(),
            at_least_one_thread_is_parked: Arc::clone(&self.at_least_one_thread_is_parked),
        }
    }
}

#[derive(Debug)]
pub struct ThreadPool<T>
where
    T: Send,
{
    /// thread_handles includes the worker threads and the ParkManager thread
    thread_handles: Vec<JoinHandle<()>>,
    shared: Shared<T>,
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
        let shared = Shared {
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
