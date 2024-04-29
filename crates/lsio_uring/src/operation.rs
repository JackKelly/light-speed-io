use std::{ffi::CString, ops::Range};

/// We keep a `Tracker<Operation>` in each thread to track progress of each operation:
#[derive(Debug)]
pub enum Operation {
    GetRanges(GetRanges),
    GetRange(GetRange),
}

impl Operation {
    pub(crate) fn get_first_step(&self, index_of_op: usize) -> NextStep {
        self.get_inner_struct().get_first_step(index_of_op)
    }

    /// If io_uring reports an error, then this function will return an `std::io::Error` with the
    /// context set twice: First to the `Operation`, and then to the `NextStep`.
    pub(crate) fn process_cqe_and_get_next_step(
        &self,
        index_of_op: usize,
        cqe: cqueue::Entry,
    ) -> Result<NextStep<M>> {
        let opcode = OpCode::new(cqe.user_data());

        // Check if the CQE reports an error. We can't return the error yet
        // because we need to know if we're expecting any more CQEs associated with this operation.
        // NOTE: A big improvement over the previous version of the code is that we can now send
        // every error that occurs (because we now have a limitless output Channel)!
        let maybe_error = cqe_error_to_anyhow_error(cqe.result());
        self.get_inner_struct()
            .process_cqe_and_get_next_step(opcode, maybe_error, index_of_op);
    }

    fn get_inner_struct(&self) -> impl UringOperation {
        use Operation::*;
        match &self {
            GetRange(n) | GetRanges(n) => n,
        }
    }
}

/// ------------------ COMMON TO ALL URING OPERATIONS ---------------------
/// Some aims of this design:
/// - Allocate on the stack
/// - Cleanly separate the code that implements the state machine for handling each operation.
/// - Gain the benefits of using the typestate pattern, whilst still allowing us to keep the types
/// in a vector. See issue #117.
trait UringOperation {
    fn get_first_step(&self, index_of_op: usize) -> Vec<squeue::Entry>;

    /// Notes on the return type:
    /// We could imagine a world in which we want to return a buffer _and_ an error, such as when
    /// io_uring reads less data than is requested. We have simplified, and assumed that this
    /// specific case will always be an error, hence it's fine to return a Result<NextStep>.
    fn process_opcode_and_get_next_step(
        &self,
        index_of_op: usize,
        opcode: OpCode,
        maybe_error: Option<Error>,
    ) -> Result<NextStep>;
}

#[derive(Debug)]
enum NextStep {
    SubmitEntries(Vec<squeue::Entry>),
    /// We're not completely done yet. For example, perhaps the file hasn't been closed yet.
    /// But the output is ready.
    PendingWithOutput(Output),
    /// We're not done yet. And there's no output ready.
    Pending,
    /// We're done! Remove this operation from the list of ops in flight.
    DoneWithOutput(Output),
    Done,
}

#[derive(Debug)]
struct OpenFile {
    location: CString,
    file_descriptor: types::Fd,
    size: usize, // We always have to `statx` the file to get the `alignment`, so we'll always get
    // the size, too.
    alignment: u32,
}

#[derive(Debug)]
struct GetRanges {
    // Creating a new CString allocates memory. And io_uring openat requires a CString.
    // We need to ensure the CString is valid until the completion queue entry arrives.
    // So we keep the CString here, in the `Operation`. This `location` hasn't yet been
    // opened, which is why it's not yet an [`OpenFile`].
    location: CString,
    ranges: Vec<Range<isize>>,
    user_data: Vec<u64>,
}

#[derive(Debug)]
struct GetRange {
    file: Arc<OpenFile>, // TODO: Replace Arc with Atomic counter?
    range: Range<isize>,
    user_data: u64,
    buffer: Option<AlignedBytes>, // This is an `Option` so we can `take` it.
}
