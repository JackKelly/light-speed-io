use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, RecvError},
        Arc,
    },
    thread,
};

pub(crate) enum ParkManagerCommand {
    WakeAtMostNThreads(u32),
    ThreadIsParked(thread::Thread),
    Stop,
}

pub(crate) struct ParkManager {
    rx: mpsc::Receiver<ParkManagerCommand>,
    at_least_one_thread_is_parked: Arc<AtomicBool>,
    parked_threads: VecDeque<thread::Thread>,
}

impl ParkManager {
    pub(crate) fn start(
        rx: mpsc::Receiver<ParkManagerCommand>,
        at_least_one_thread_is_parked: Arc<AtomicBool>,
        n_worker_threads: usize,
    ) -> thread::JoinHandle<()> {
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

    fn thread_is_parked(&mut self, t: thread::Thread) {
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
