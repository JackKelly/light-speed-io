use std::sync::{
    atomic::{AtomicBool, Ordering::Relaxed},
    mpsc, Arc,
};

use crossbeam::deque;

use crate::park_manager::ParkManagerCommand;

/// `ThreadPool` owns a `SharedState<T>`, and each `WorkerThread` owns a cloned `SharedState<T>`.
#[derive(Debug)]
pub(crate) struct SharedState<T>
where
    T: Send,
{
    pub(crate) injector: Arc<deque::Injector<T>>,
    pub(crate) keep_running: Arc<AtomicBool>,
    pub(crate) chan_to_park_manager: mpsc::Sender<ParkManagerCommand>,
    pub(crate) at_least_one_thread_is_parked: Arc<AtomicBool>,
}

impl<T> SharedState<T>
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

impl<T> Clone for SharedState<T>
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
