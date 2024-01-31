use io_uring::squeue;

use crate::{operation::Operation, operation_future::SharedState};

#[derive(Debug)]
pub(crate) struct PreparedEntry {
    shared_state: SharedState,
    pub(crate) sq_entry: squeue::Entry,
}

pub(crate) fn prepare_io_uring_entry(shared_state: &SharedState) -> PreparedEntry {
    let op = shared_state.lock().unwrap().get_operation();

    match op {
        Operation::Get { location } => {
            // TODO:
            // 1. Get filesize. (DON'T do this in the critical section of the Mutex!)
            // 2. Allocate buffer, and assign it to InnerState.output
            // 3. Create squeue::Entry
            // 4. Return a PreparedEntry
            todo!();
        }
    }
}
