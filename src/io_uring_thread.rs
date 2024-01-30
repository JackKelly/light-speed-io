use std::{
    sync::{mpsc::Sender, Arc},
    thread::JoinHandle,
};

use crate::operation_future::SharedState;

#[derive(Debug)]
pub(crate) struct WorkerThread {
    pub(crate) handle: JoinHandle<()>,
    pub(crate) sender: Sender<Arc<SharedState>>, // Channel to send ops to the worker thread
}
