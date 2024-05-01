use std::{ffi::CString, iter::zip, ops::Range, sync::Arc};

use crate::{
    get_range::GetRange,
    open_file::OpenFileBuilder,
    operation::{NextStep, Operation, UringOperation},
    sqe::{build_openat_sqe, build_statx_sqe},
};

const N_CQES_EXPECTED: u8 = 2; // We're expecting CQEs for `openat` and `statx`.

#[derive(Debug)]
pub(crate) struct GetRanges {
    // Creating a new CString allocates memory. And io_uring openat requires a CString.
    // We need to ensure the CString is valid until the completion queue entry arrives.
    // So we keep the CString here, in the `Operation`. This `location` hasn't yet been
    // opened, which is why it's not yet an [`OpenFile`].
    open_file_builder: OpenFileBuilder,
    ranges: Vec<Range<isize>>,
    user_data: Vec<u64>,

    // If both CQEs succeed then we'll capture their outputs in `open_file_builder`. But, in case
    // one or more CQEs reports a failure, we need an additional mechanism to track which CQEs
    // we've received.
    n_cqes_received: u8,
}

impl GetRanges {
    fn new(location: CString, ranges: Vec<Range<isize>>, user_data: Vec<u64>) -> Self {
        assert_eq!(ranges.len(), user_data.len());
        Self {
            open_file_builder: OpenFileBuilder::new(location),
            ranges,
            user_data,
            n_cqes_received: 0,
        }
    }

    // io_uring can't process multiple range requests in a single op. So, once we've opened the
    // file and gotten its metadata, we need to submit one `Operation::GetRange` per byte range.
    fn submit_get_range_ops(
        self,
        local_worker_queue: &crossbeam::deque::Worker<crate::operation::Operation>,
    ) {
        let file = Arc::new(self.open_file_builder.build());
        for (range, user_data) in zip(self.ranges, self.user_data) {
            let get_range_op = GetRange::new(file.clone(), range, user_data);
            local_worker_queue.push(Operation::GetRange(get_range_op));
        }
    }
}

impl UringOperation for GetRanges {
    fn get_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError> {
        let open_entry = build_openat_sqe(index_of_op, self.open_file_builder.location());
        let statx_entry = build_statx_sqe(
            index_of_op,
            self.open_file_builder.location(),
            self.open_file_builder.get_statx_ptr(),
        );
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
        self.n_cqes_received += 1;
        if let Ok(cqe_result_value) = cqe_result {
            match idx_and_opcode.opcode().value() {
                io_uring::opcode::OpenAt::CODE => {
                    self.open_file_builder
                        .set_file_descriptor(io_uring::types::Fd(*cqe_result_value));
                }
                io_uring::opcode::Statx::CODE => {
                    unsafe {
                        self.open_file_builder.assume_statx_is_initialised();
                    };
                }
                _ => panic!("Unrecognised opcode! {idx_and_opcode:?}"),
            };
        };

        if self.n_cqes_received >= N_CQES_EXPECTED {
            if self.open_file_builder.is_ready() {
                self.submit_get_range_ops(local_worker_queue);
                NextStep::Done
            } else {
                // We've seen all the CQEs we were expecting, but `open_file_builder` isn't ready. So
                // at least one of the CQEs must have resulted in an error. Nevertheless, we're "done".
                NextStep::Done
            }
        } else {
            NextStep::Pending
        }
    }
}
