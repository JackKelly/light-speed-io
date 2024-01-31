use std::sync::Arc;

use io_uring::squeue;

use crate::{operation::OperationOutput, operation_future::SharedStateForOpFuture};

pub(crate) struct PreparedEntry {
    shared_state_for_op_future: Arc<SharedStateForOpFuture>,
    submission_queue_entry: squeue::Entry,
    output: Option<OperationOutput>,
}

pub(crate) fn prepare_io_uring_entry(shared_state: SharedStateForOpFuture) -> PreparedEntry {
    match shared_state.get_operation() {
        Get => {
            todo!();
        }
    }
}
