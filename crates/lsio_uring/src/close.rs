use crate::{
    open_file::OpenFile,
    operation::{NextStep, UringOperation},
    sqe::build_close_sqe,
};

#[derive(Debug)]
pub(crate) struct Close {
    file: OpenFile,
}

impl Close {
    pub(crate) fn new(file: OpenFile) -> Self {
        Self { file }
    }
}

impl UringOperation for Close {
    fn submit_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError> {
        let entry = build_close_sqe(index_of_op, *self.file.file_descriptor());
        unsafe { local_uring_submission_queue.push(&entry) }
    }

    fn process_opcode_and_submit_next_step(
        self,
        idx_and_opcode: &crate::user_data::UringUserData,
        _cqe_result: i32,
        _local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        _local_worker_queue: &crossbeam::deque::Worker<crate::operation::Operation>,
        _output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep {
        if idx_and_opcode.opcode().value() != io_uring::opcode::Close::CODE {
            panic!("Unrecognised opcode!");
        }
        NextStep::Done
    }
}
