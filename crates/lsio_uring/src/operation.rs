use crossbeam::deque::Worker;

use crate::{close::Close, get_range::GetRange, get_ranges::GetRanges, user_data::UringUserData};

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
        &mut self,
        index_of_op: usize,
        cqe: &io_uring::cqueue::Entry,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep {
        let idx_and_opcode = UringUserData::from(cqe.user_data());
        let error_context = || format!("idx_and_opcode: {idx_and_opcode:?}. cqe: {cqe:?}");
        let cqe_result = cqe_error_to_anyhow_error(cqe.result(), error_context);
        self.process_opcode_and_submit_next_step(
            &idx_and_opcode,
            &cqe_result,
            local_uring_submission_queue,
            local_worker_queue,
            output_channel,
        )
    }

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
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep {
        self.apply_func_to_all_inner_structs(|s| {
            UringOperation::send_error_or_process_opcode_and_get_next_step(
                s,
                idx_and_opcode,
                cqe_result,
                local_uring_submission_queue,
                local_worker_queue,
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

    fn send_error_or_process_opcode_and_get_next_step(
        &mut self,
        idx_and_opcode: &UringUserData,
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep {
        if let Err(err) = cqe_result {
            output_channel.send(Err(err.context(format!("{self:?}"))));
        }
        self.process_opcode_and_submit_next_step(
            idx_and_opcode,
            cqe_result,
            local_uring_submission_queue,
            local_worker_queue,
            output_channel,
        )
    }

    fn process_opcode_and_submit_next_step(
        &mut self,
        idx_and_opcode: &UringUserData,
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep;
}

pub(crate) enum NextStep {
    Pending,
    Done,
    ReplaceWith(Operation),
}

fn cqe_error_to_anyhow_error(cqe_result: i32, context: impl Fn() -> String) -> anyhow::Result<i32> {
    if cqe_result < 0 {
        let nix_err = nix::Error::from_raw(-cqe_result);
        let full_context = format!(
            "{nix_err} (reported by io_uring completion queue entry (CQE)). {}",
            context()
        );
        Err(anyhow::Error::new(nix_err).context(full_context))
    } else {
        Ok(cqe_result)
    }
}
