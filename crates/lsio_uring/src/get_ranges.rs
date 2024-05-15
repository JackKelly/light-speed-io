use std::{ffi::CString, iter::zip, ops::Range, sync::Arc};

use lsio_threadpool::WorkerThread;

use crate::{
    get_range::GetRange,
    open_file::OpenFileBuilder,
    operation::{NextStep, Operation, UringOperation},
    sqe::{build_openat_sqe, build_statx_sqe},
};

const N_CQES_EXPECTED: u8 = 2; // We're expecting CQEs for `openat` and `statx`.

#[derive(Debug)]
pub(crate) struct GetRanges {
    open_file_builder: Option<OpenFileBuilder>,
    ranges: Vec<Range<isize>>,
    user_data: Vec<u64>,

    // If both CQEs succeed then we'll capture their outputs in `open_file_builder`. But, in case
    // one or more CQEs reports a failure, we need an additional mechanism to track how many CQEs
    // we've received.
    n_cqes_received: u8,
}

impl GetRanges {
    pub(crate) fn new(location: CString, ranges: Vec<Range<isize>>, user_data: Vec<u64>) -> Self {
        assert_eq!(ranges.len(), user_data.len());
        Self {
            open_file_builder: Some(OpenFileBuilder::new(location)),
            ranges,
            user_data,
            n_cqes_received: 0,
        }
    }

    // io_uring can't process multiple range requests in a single op. So, once we've opened the
    // file and gotten its metadata, we need to submit one `Operation::GetRange` per byte range.
    fn submit_get_range_ops(&mut self, worker_thread: &WorkerThread<Operation>) {
        let file = Arc::new(self.open_file_builder.take().unwrap().build());
        for (range, user_data) in zip(&self.ranges, &self.user_data) {
            let get_range_op = GetRange::new(file.clone(), range.to_owned(), *user_data);
            worker_thread.push(Operation::GetRange(get_range_op));
        }
    }
}

impl UringOperation for GetRanges {
    fn submit_first_step(
        &mut self,
        index_of_op: usize,
        local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
    ) -> Result<(), io_uring::squeue::PushError> {
        let open_entry = build_openat_sqe(
            index_of_op,
            self.open_file_builder.as_ref().unwrap().location(),
        );
        let statx_entry =
            build_statx_sqe(index_of_op, &mut self.open_file_builder.as_mut().unwrap());
        unsafe {
            local_uring_submission_queue.push(&open_entry)?;
            local_uring_submission_queue.push(&statx_entry)?;
        };
        Ok(())
    }

    fn process_opcode_and_submit_next_step(
        &mut self,
        idx_and_opcode: &crate::user_data::UringUserData,
        cqe_result: i32,
        _local_uring_submission_queue: &mut io_uring::squeue::SubmissionQueue,
        worker_thread: &WorkerThread<Operation>,
        _output_channel: &mut crossbeam_channel::Sender<anyhow::Result<lsio_io::Output>>,
    ) -> NextStep {
        self.n_cqes_received += 1;
        if cqe_result >= 0 {
            match idx_and_opcode.opcode().value() {
                io_uring::opcode::OpenAt::CODE => {
                    self.open_file_builder
                        .as_mut()
                        .unwrap()
                        .set_file_descriptor(io_uring::types::Fd(cqe_result));
                }
                io_uring::opcode::Statx::CODE => {
                    unsafe {
                        self.open_file_builder
                            .as_mut()
                            .unwrap()
                            .assume_statx_is_initialised();
                    };
                }
                _ => panic!("Unrecognised opcode! {idx_and_opcode:?}"),
            };
        };

        assert!(self.n_cqes_received <= N_CQES_EXPECTED);
        if self.n_cqes_received == N_CQES_EXPECTED {
            if self.open_file_builder.as_mut().unwrap().is_ready() {
                self.submit_get_range_ops(worker_thread);
                NextStep::Done
            } else {
                // We've seen all the CQEs we were expecting, but `open_file_builder` isn't ready. So
                // at least one of the CQEs must have resulted in an error. Nevertheless, we're "done".
                NextStep::Done
            }
        } else {
            // We're expecting one more CQE.
            NextStep::Pending
        }
    }
}
