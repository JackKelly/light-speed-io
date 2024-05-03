use crate::{
    close::Close,
    open_file::OpenFile,
    operation::{NextStep, Operation, UringOperation},
    sqe::build_read_range_sqe,
    user_data::UringUserData,
};
use crossbeam::deque::Worker;
use lsio_aligned_bytes::AlignedBytes;
use lsio_io::{Chunk, Output};
use std::{ops::Range, sync::Arc};

#[derive(Debug)]
pub(crate) struct GetRange {
    file: Arc<OpenFile>, // TODO: Replace Arc with Atomic counter?
    range: Range<isize>,
    user_data: u64,
    buffer: Option<AlignedBytes>, // This is an `Option` so we can `take` it.
}

impl GetRange {
    pub(crate) fn new(file: Arc<OpenFile>, range: Range<isize>, user_data: u64) -> Self {
        // TODO: Split reads of more than 2 GiB into multiple smaller reads! See issue #99.
        if range.len() > 2_147_479_552 {
            panic!(
                "`read` will transfer at most 2 GiB but {} bytes were requested. \
                     See https://github.com/JackKelly/light-speed-io/issues/99",
                range.len()
            );
        }
        Self {
            file,
            range,
            user_data,
            buffer: None,
        }
    }
}

impl UringOperation for GetRange {
    /// This method assume that the file has already been opened (by the [`GetRanges`] operation).
    fn submit_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError> {
        let (entry, buffer) = build_read_range_sqe(index_of_op, &self.file, &self.range);
        self.buffer = Some(buffer);
        unsafe { local_uring_submission_queue.push(&entry) } // TODO: Does `entry` have to stay
                                                             // alive for longer?
    }

    fn process_opcode_and_submit_next_step(
        &mut self,
        idx_and_opcode: &UringUserData,
        cqe_result: &anyhow::Result<i32>,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        // We don't use `local_worker_queue` in this example. But GetRanges will want to pump out
        // lots of GetRange ops into the `local_worker_queue`!
        _local_worker_queue: &Worker<Operation>,
        output_channel: &mut crossbeam::channel::Sender<anyhow::Result<Output>>,
    ) -> NextStep {
        // Check that the opcode of the CQE is what we expected:
        if idx_and_opcode.opcode().value() != io_uring::opcode::Read::CODE {
            panic!("Unrecognised opcode!");
        }
        if let Ok(_cqe_result_value) = cqe_result {
            // TODO: Check we've read the correct number of bytes:
            //       Check `cqe_result_value == self.buffer.len()`.
            // TODO: Retry if we read less data than requested! See issue #100.

            output_channel.send(Ok(Output::Chunk(Chunk {
                buffer: self.buffer.take().unwrap(),
                user_data: self.user_data,
            })));
        };
        // Check if it's time to close the file:
        match Arc::try_unwrap(self.file) {
            Ok(file) => {
                // `Arc::try_unwrap` returns `Ok<T>` if the Arc has exactly one strong reference.
                // In our case, that means that we're the last operation on this file, so it's time
                // to close this file.
                let mut close_op = Close::new(file);
                close_op
                    .submit_first_step(
                        idx_and_opcode.index_of_op() as _,
                        local_uring_submission_queue,
                    )
                    .unwrap();
                NextStep::ReplaceWith(Operation::Close(close_op))
            }
            Err(file) => {
                self.file = file;
                NextStep::Done
            }
        }
    }
}
