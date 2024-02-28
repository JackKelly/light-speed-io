use io_uring::{cqueue, opcode, types};
use std::ffi::CString;
use std::ops::Range;
use tokio::sync::oneshot;

use crate::operation;
use crate::uring;
use crate::uring::operation::{
    build_openat_sqe, create_linked_read_range_close_sqes, InnerState, NextStep,
};

#[derive(Debug)]
pub(super) struct GetRange {
    pub(super) path: CString,
    pub(super) range: Range<isize>,
    pub(super) fixed_fd: Option<types::Fixed>,
    pub(super) inner: InnerState,
}

impl GetRange {
    pub(super) fn new(
        path: CString,
        range: Range<isize>,
        output_channel: oneshot::Sender<anyhow::Result<operation::OperationOutput>>,
    ) -> Self {
        Self {
            path,
            range,
            inner: InnerState::new(output_channel),
            fixed_fd: None,
        }
    }
}

impl uring::Operation for GetRange {
    fn process_cqe(&mut self, cqe: cqueue::Entry) {
        self.inner.process_cqe(cqe);
    }

    fn next_step(&mut self, index_of_op: usize) -> NextStep {
        self.inner.n_steps_completed += 1;
        match self.inner.last_cqe.as_ref() {
            // Build the first SQE:
            None => {
                self.inner.check_n_steps_completed_is_1();
                NextStep::SubmitEntries {
                    entries: build_openat_sqe(&self.path, index_of_op),
                    register_file: true,
                }
            }

            // Build subsequent SQEs:
            Some(cqe) => match self
                .inner
                .last_opcode
                .expect("last_opcode not set, even though last_cqe is set!")
            {
                opcode::OpenAt::CODE => {
                    if self.inner.error_has_occurred {
                        // If we failed to open the file, then there's no point submitting linked
                        // read-close operations. So we're done.
                        NextStep::Done {
                            unregister_file: true,
                        }
                    } else {
                        self.fixed_fd = Some(types::Fixed(cqe.result() as u32));
                        let (entries, buffer) = create_linked_read_range_close_sqes(
                            &self.range,
                            self.fixed_fd.as_ref().unwrap(),
                            index_of_op,
                        );
                        self.inner.output = Some(buffer);
                        NextStep::SubmitEntries {
                            entries,
                            register_file: false,
                        }
                    }
                }
                opcode::Read::CODE => {
                    if !self.inner.error_has_occurred {
                        self.inner.send_output();
                    }
                    // We're not done yet, because we need to wait for the close op.
                    // The close op is linked to the read op.
                    // TODO: Return Done if we unlink read and close.
                    NextStep::Pending
                }
                opcode::Close::CODE => NextStep::Done {
                    unregister_file: true,
                },
                _ => panic!("Unrecognised opcode!"),
            },
        }
    }
}
