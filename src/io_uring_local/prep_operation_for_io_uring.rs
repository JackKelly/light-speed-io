use std::sync::Arc;

use io_uring::squeue;

use crate::operation_future::SharedStateForOpFuture;

pub(crate) struct PreparedEntry<Output> {
    shared_state_for_op_future: Arc<SharedStateForOpFuture<Output>>,
    submission_queue_entry: squeue::Entry,
    output: Option<Output>,
}

pub(crate) fn prepare_io_uring_entry<Output>(
    shared_state: SharedStateForOpFuture<Output>,
) -> PreparedEntry<Output> 
where
    Output: Send + Sync,
{
    match shared_state.get_operation() {
        Get => {
            todo!();
        }
    }
}
