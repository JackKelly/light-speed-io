use std::sync::Arc;

use io_uring::squeue;

use crate::{operation::Operation, operation_future::SharedStateForOpFuture, output::Output};

pub(crate) struct PreparedEntry {
    shared_state_for_op_future: Arc<SharedStateForOpFuture>,
    submission_queue_entry: squeue::Entry,
}

pub(crate) fn prepare_io_uring_entry(shared_state: SharedStateForOpFuture) -> PreparedEntry {
    match shared_state.get_operation() {
        Operation::Get{location} => {
            // TODO:
            // 1. Get filesize
            // 2. Allocate buffer, and assign it to 
            // 3. Create squeue::Entry
            // 4. Return a PreparedEntry
            todo!();
        }
    }
}
