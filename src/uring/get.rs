use io_uring::{cqueue, opcode, types};
use std::ffi::CString;
use tokio::sync::oneshot;

use crate::operation;
use crate::uring;
use crate::uring::operation::{
    build_openat_sqe, create_linked_read_close_sqes, InnerState, NextStep,
};

#[derive(Debug)]
pub(super) struct Get {
    pub(super) path: CString,
    pub(super) fixed_fd: Option<types::Fixed>,
    pub(super) inner: InnerState,
}

impl Get {
    pub(super) fn new(
        path: CString,
        output_channel: oneshot::Sender<anyhow::Result<operation::OperationOutput>>,
    ) -> Self {
        Self {
            path,
            inner: InnerState::new(output_channel),
            fixed_fd: None,
        }
    }
}

impl uring::Operation for Get {
    fn process_cqe(&mut self, cqe: cqueue::Entry) {
        self.inner.process_cqe(cqe);
    }

    fn next_step(&mut self, index_of_op: usize) -> NextStep {
        self.inner.n_steps_completed += 1;
        match self.inner.last_cqe.as_ref() {
            // Build the first SQE:
            None => {
                assert_eq!(
                    self.inner.n_steps_completed, 1,
                    "`next_step` has been called {} times, yet `last_cqe` is None. Have you forgotten to call `process_cqe`?",
                    self.inner.n_steps_completed
                );
                NextStep::SubmitFirstEntriesToOpenFile(build_openat_sqe(&self.path, index_of_op))
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
                        NextStep::Done
                    } else {
                        self.fixed_fd = Some(types::Fixed(cqe.result() as u32));
                        let (entries, buffer) = create_linked_read_close_sqes(
                            &self.path,
                            self.fixed_fd.as_ref().unwrap(),
                            index_of_op,
                        );
                        self.inner.output = Some(buffer);
                        NextStep::SubmitSubsequentEntries(entries)
                    }
                }
                opcode::Read::CODE => {
                    if self.inner.error_has_occurred {
                        // We're not done yet, because we need to wait for the close op.
                        // The close op is linked to the read op.
                        // TODO: Return Done if we unlink read and close.
                        NextStep::Error
                    } else {
                        self.inner.send_output();
                        NextStep::OutputHasBeenSent
                    }
                }
                opcode::Close::CODE => NextStep::Done,
                _ => panic!("Unrecognised opcode!"),
            },
        }
    }
}
