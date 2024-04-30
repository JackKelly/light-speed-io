use std::{ffi::CString, ops::Range};

use crate::operation::UringOperation;

#[derive(Debug)]
pub(crate) struct GetRanges {
    // Creating a new CString allocates memory. And io_uring openat requires a CString.
    // We need to ensure the CString is valid until the completion queue entry arrives.
    // So we keep the CString here, in the `Operation`. This `location` hasn't yet been
    // opened, which is why it's not yet an [`OpenFile`].
    location: CString,
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
        &self,
        idx_and_opcode: &crate::user_data::UringUserData,
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        local_worker_queue: &crossbeam::deque::Worker<crate::operation::Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> Option<crate::operation::Operation> {
        todo!()
    }
}
