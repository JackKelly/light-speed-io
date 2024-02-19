use std::collections::VecDeque;

use crate::operation::OperationWithCallback;

pub(crate) struct OpTracker {
    // The original intention was for `ops_in_flight` to be a `Vec<Option<OperationWithCallback>>`
    // but that was surprisingly slow (about 830 MiB/s on Jack's Intel NUC, and about 15,000 page faults per sec).
    // Using a `Vec<Option<Box>>` was the fastest option I tried: about 1.3 GiB/s, and only 8 k page faults per sec.
    // That's the same bandwidth, but about twice the number of page faults as just passing the pointer
    // returned by `Box::into_raw` to `user_data()`. I tried lots of options (see PR #63).
    // TODO: Try removing this Box, after #43 is implemented.
    pub(crate) ops_in_flight: Vec<Option<Box<OperationWithCallback>>>,
    pub(crate) next_index: VecDeque<usize>,
}

impl OpTracker {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            ops_in_flight: (0..n).map(|_| None).collect(),
            next_index: (0..n).collect(),
        }
    }

    pub(crate) fn get_next_index(&mut self) -> usize {
        self.next_index
            .pop_front()
            .expect("next_index should not be empty!")
    }

    pub(crate) fn put(&mut self, index: usize, op: OperationWithCallback) {
        let op = Box::new(op);
        self.ops_in_flight[index].replace(op);
    }

    pub(crate) fn remove(&mut self, index: usize) -> OperationWithCallback {
        self.next_index.push_back(index);
        *self.ops_in_flight[index]
            .take()
            .expect("No Operation found at index {index}!")
    }
}
