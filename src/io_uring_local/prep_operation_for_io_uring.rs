use io_uring::squeue;

use crate::operation::OpType;

#[derive(Debug)]
pub(crate) struct SQEntryWithOperation {
    operation: OpType,
    pub(crate) sq_entry: squeue::Entry,
}

pub(crate) fn prepare_io_uring_entry(operation: OpType) -> SQEntryWithOperation {
    match operation {
        OpType::Get { location, .. } => {
            // TODO:
            // 1. Get filesize. (DON'T do this in the critical section of the Mutex!)
            // 2. Allocate buffer, and assign it to InnerState.output
            // 3. Create squeue::Entry
            // 4. Return a PreparedEntry
            todo!();
        }
    }
}
