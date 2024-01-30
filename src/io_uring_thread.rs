use std::{
    sync::{mpsc::Sender, Arc, RwLock},
    thread::JoinHandle,
};

use crate::operation_future::SharedState;

#[derive(Debug)]
pub(crate) struct WorkerThread {
    pub(crate) handle: JoinHandle<()>,
    pub(crate) sender: Sender<Arc<RwLock<SharedState>>>, // Channel to send ops to the worker thread
}
