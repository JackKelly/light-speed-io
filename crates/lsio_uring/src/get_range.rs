use crate::uring::operation::ALIGN;
use io_uring::{cqueue, opcode, types};
use std::cmp::max;
use std::ffi::CString;
use std::ops::Range;
use tokio::sync::oneshot;

use crate::operation;
use crate::uring;
use crate::uring::operation::{
    build_openat_sqe, create_linked_read_range_close_sqes, InnerState, NextStep,
};

impl GetRange {
    pub(crate) fn new(file: Arc<OpenFile>, range: Range<isize>, user_data: u64) -> Self {
        // TODO: Split reads of more than 2 GiB into multiple smaller reads! See issue #99.
        if range.len() > 2_147_479_552 {
            panic!(
                "`read` will transfer at most 2 GiB but {} bytes were requested. \
                     See https://github.com/JackKelly/light-speed-io/issues/99",
                len_requested_by_user
            );
        }
        Self {
            file,
            range,
            user_data,
            // TODO: Maybe we should actually allocate the buffer _here_? Then `get_first_step`
            // wouldn't have to take a `mut` ref to `self`. And we're more likely to know the
            // alignement at runtime at this point in the code? And we could keep track of the
            // _aligned_ byte range that we read from disk; and the byte range requested by the
            // user.
            buffer: None,
        }
    }
}

impl UringOperation for GetRange {
    /// This method assume that the file has already been opened (by the [`GetRanges`] operation).
    fn get_first_step(
        &mut self,
        index_of_op: usize,
        uring_submission_queue: &VecDeque<squeue::Entry>,
    ) {
        // TODO: Actually, I think these UringOperations should take a reference to the worker
        // queue, and the output queue. Then we can directly write into those queues!
        let (entry, buffer) = build_read_range_sqe(index_of_op, &self.file, &self.range);
        self.buffer = Some(buffer);
        vec![entry]
    }

    fn process_opcode_and_get_next_step(
        &mut self,
        user_data: &UringUserData,
        cqe_result: Result<i32>,
    ) -> Result<NextStep> {
        match user_data.opcode().value() {
            opcode::Read::CODE => {
                if let Ok(cqe_result_value) = cqe_result {
                    // TODO: Check we've read the correct number of bytes.
                    // TODO: Retry if we read less data than requested! See issue #100.
                    // We're not done yet, because we need to wait for the close op.
                    NextStep::PendingWithOutput(Output::Chunk(self.chunk.take()))
                } else {
                    todo!(); // TODO: Handle when there's an error!
                }
            }
            _ => panic!("Unrecognised opcode!"),
        }
    }
}
