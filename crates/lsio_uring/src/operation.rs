use crossbeam::deque::Worker;

use crate::{get_range::GetRange, get_ranges::GetRanges, user_data::UringUserData};

/// We keep a `Tracker<Operation>` in each thread to track progress of each operation:
#[derive(Debug)]
pub(crate) enum Operation {
    GetRanges(GetRanges),
    GetRange(GetRange),
    Close(Close),
}

impl Operation {
    /// If io_uring reports an error, then this function will return an `std::io::Error` with the
    /// context set twice: First to the `Operation`, and then to the `NextStep`.
    pub(crate) fn process_cqe_and_get_next_step(
        self,
        index_of_op: usize,
        cqe: &io_uring::cqueue::Entry,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> Option<Operation> {
        let idx_and_opcode = UringUserData::from(cqe.user_data());
        let cqe_result = cqe_error_to_anyhow_error(cqe.result());
        self.process_opcode_and_get_next_step(
            &idx_and_opcode,
            &cqe_result,
            local_uring_submission_queue,
            local_worker_queue,
            output_channel,
        )
    }
}

impl UringOperation for Operation {
    fn get_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError> {
        use Operation::*;
        match self {
            // TODO: Use a Rust macro to reduce the code duplication in this `match` block.
            GetRanges(s) => s.get_first_step(index_of_op, local_uring_submission_queue),
            GetRange(s) => s.get_first_step(index_of_op, local_uring_submission_queue),
        }
    }

    fn process_opcode_and_get_next_step(
        self,
        // TODO: Needs to be renamed, to distinguish from our
        // `Chunk.user_data`. Maybe rename to `IdxAndOpcode`?
        idx_and_opcode: &UringUserData,
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> Option<Operation> {
        use Operation::*;
        match self {
            // TODO: Use a Rust macro to reduce the code duplication in this `match` block.
            GetRanges(s) => s.process_opcode_and_get_next_step(
                idx_and_opcode,
                cqe_result,
                local_uring_submission_queue,
                local_worker_queue,
                output_channel,
            ),
            GetRange(s) => s.process_opcode_and_get_next_step(
                idx_and_opcode,
                cqe_result,
                local_uring_submission_queue,
                local_worker_queue,
                output_channel,
            ),
        }
    }
}
/// ------------------ COMMON TO ALL URING OPERATIONS ---------------------
/// Some aims of this design:
/// - Allocate on the stack
/// - Cleanly separate the code that implements the state machine for handling each operation.
/// - Gain the benefits of using the typestate pattern, whilst still allowing us to keep the types
/// in a vector. See issue #117.
pub(crate) trait UringOperation {
    fn get_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError>;

    fn process_opcode_and_get_next_step(
        self,
        // TODO: Needs to be renamed, to distinguish from our
        // `Chunk.user_data`. Maybe rename to `IdxAndOpcode`?
        idx_and_opcode: &UringUserData,
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        // We don't use `local_worker_queue` in this example. But GetRanges will want to pump out
        // lots of GetRange ops into the `local_worker_queue`!
        local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> Option<Operation>;
}

fn cqe_error_to_anyhow_error(cqe_result: i32) -> anyhow::Result<i32> {
    if cqe_result < 0 {
        let nix_err = nix::Error::from_raw(-cqe_result);
        Err(anyhow::Error::new(nix_err).context(format!(
            "{nix_err} (reported by io_uring completion queue entry (CQE))",
        )))
    } else {
        Ok(cqe_result)
    }
}
