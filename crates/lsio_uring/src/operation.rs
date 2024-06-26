use lsio_threadpool::WorkerThread;

use crate::{close::Close, get_range::GetRange, get_ranges::GetRanges, user_data::UringUserData};

/// We keep a `Tracker<Operation>` in each thread to track progress of each operation:
#[derive(Debug)]
pub(crate) enum Operation {
    GetRanges(GetRanges),
    GetRange(GetRange),
    Close(Close),
}

impl Operation {
    fn apply_func_to_all_inner_structs<F, R>(&mut self, mut f: F) -> R
    where
        F: FnMut(&mut dyn UringOperation) -> R,
    {
        use Operation::*;
        match self {
            GetRanges(s) => f(s),
            GetRange(s) => f(s),
            Close(s) => f(s),
        }
    }
}

impl UringOperation for Operation {
    fn submit_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError> {
        self.apply_func_to_all_inner_structs(|s| {
            UringOperation::submit_first_step(s, index_of_op, local_uring_submission_queue)
        })
    }

    fn process_opcode_and_submit_next_step(
        &mut self,
        idx_and_opcode: &UringUserData,
        cqe_result: i32,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        worker_thread: &WorkerThread<Operation>,
        output_channel: &mut crossbeam_channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep {
        self.apply_func_to_all_inner_structs(|s| {
            UringOperation::maybe_send_error(s, idx_and_opcode, cqe_result, output_channel);
            UringOperation::process_opcode_and_submit_next_step(
                s,
                idx_and_opcode,
                cqe_result,
                local_uring_submission_queue,
                worker_thread,
                output_channel,
            )
        })
    }
}

/// ------------------ COMMON TO ALL URING OPERATIONS ---------------------
/// Some aims of this design:
/// - Allocate on the stack
/// - Cleanly separate the code that implements the state machine for handling each operation.
/// - Gain the benefits of using the typestate pattern, whilst still allowing us to keep the types
/// in a vector. See issue #117.
pub(crate) trait UringOperation: std::fmt::Debug {
    fn submit_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError>;

    fn process_opcode_and_submit_next_step(
        &mut self,
        idx_and_opcode: &UringUserData,
        cqe_result: i32,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        worker_thread: &WorkerThread<Operation>,
        output_channel: &mut crossbeam_channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep;

    fn maybe_send_error(
        &self,
        idx_and_opcode: &UringUserData,
        cqe_result: i32,
        output_channel: &mut crossbeam_channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) {
        if cqe_result < 0 {
            // TODO: We probably want a custom Error struct (or enum?) which has machine-readable
            // fields for filename, byte_range(s), user_data, error code, opcode. But this
            // `anyhow::Error` will do for now.
            let nix_err = nix::Error::from_raw(-cqe_result);
            let context = format!(
                "{nix_err} (reported by io_uring completion queue entry (CQE)). More details: \
                    idx_and_opcode: {idx_and_opcode:?}. cqe_result: {cqe_result}. self: {self:?}",
            );
            let err = Err(anyhow::Error::new(nix_err).context(context));
            output_channel.send(err).unwrap();
        }
    }
}

pub(crate) enum NextStep {
    Pending,
    Done,
    ReplaceWith(Operation),
}
