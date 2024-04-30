use std::{ffi::CString, ops::Range};

use crate::operation::{NextStep, UringOperation};

#[derive(Debug)]
pub(crate) struct GetRanges {
    // Creating a new CString allocates memory. And io_uring openat requires a CString.
    // We need to ensure the CString is valid until the completion queue entry arrives.
    // So we keep the CString here, in the `Operation`. This `location` hasn't yet been
    // opened, which is why it's not yet an [`OpenFile`].
    open_file_builder: OpenFileBuilder,
    ranges: Vec<Range<isize>>,
    user_data: Vec<u64>,
}

impl UringOperation for GetRanges {
    fn get_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError> {
        unsafe {
            local_uring_submission_queue.push(&open_entry)?;
            local_uring_submission_queue.push(&statx_entry)?;
        };
        Ok(())
    }

    fn process_opcode_and_get_next_step(
        &mut self,
        idx_and_opcode: &crate::user_data::UringUserData,
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &crossbeam::deque::Worker<crate::operation::Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep {
        // TODO: Handle cqe error.
        match idx_and_opcode.opcode().value() {
            io_uring::opcode::OpenAt::CODE => {
                self.open_file_builder.set_file_descriptor(fd);
            },
            io_uring::opcode::Statx::CODE => {
                self.open_file_builder.set_from_statx(statx_result);
            },
            _ => panic!("Unrecognised opcode! {idx_and_opcode:?}");
        };

        // Check is `self.location` has had all the necessary fields set:
        if self.open_file_builder.is_ready() {
            self.submit_get_range_ops(local_worker_queue);
            NextStep::Done
        } else {
            NextStep::Pending
        }
    }
}
