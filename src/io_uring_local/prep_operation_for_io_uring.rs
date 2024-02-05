use io_uring::squeue;

use crate::operation::{Operation, OperationWithCallback};

pub(crate) fn prepare_io_uring_entry(op_with_callback: OperationWithCallback) -> squeue::Entry {
    match op_with_callback.get_operation() {
        Some(op) => match op {
            Operation::Get { location, .. } => {
                // TODO:
                // 1. Get filesize. (DON'T do this in the critical section of the Mutex!)
                // 2. Allocate buffer, and assign it to InnerState.output
                // 3. Create squeue::Entry
                // 4. Return a PreparedEntry
                todo!();
            }
        },
        None => todo!(),
    }
}
