use std::collections::VecDeque;

pub(crate) struct Tracker<T> {
    // The original intention was for `ops_in_flight` to be a `Vec<Option<T>>`
    // but that was surprisingly slow (about 830 MiB/s on Jack's Intel NUC, and about 15,000 page faults per sec).
    // Using a `Vec<Option<Box>>` was the fastest option I tried: about 1.3 GiB/s, and only 8 k page faults per sec.
    // That's the same bandwidth, but about twice the number of page faults as just passing the pointer
    // returned by `Box::into_raw` to `user_data()`. I tried lots of options (see PR #63).
    // TODO: Try removing this Box, after #43 is implemented.
    pub(crate) ops_in_flight: Vec<Option<Box<T>>>,
    pub(crate) next_index: VecDeque<usize>,
}

impl<T> Tracker<T> {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            ops_in_flight: (0..n).map(|_| None).collect(),
            next_index: (0..n).collect(),
        }
    }

    pub(crate) fn get_next_index(&mut self) -> Option<usize> {
        self.next_index.pop_front()
    }

    pub(crate) fn put(&mut self, index: usize, op: T) {
        let op = Box::new(op);
        self.ops_in_flight[index].replace(op);
    }

    pub(crate) fn remove(&mut self, index: usize) -> Option<T> {
        self.ops_in_flight[index].take().map(|t| {
            self.next_index.push_back(index);
            *t
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_op_tracker() {
        let mut tracker = Tracker::new(2);

        // Check that removing an item before inserting an item returns None.
        assert!(tracker.remove(0).is_none());

        // Put one string into the tracker, and then remove that string.
        let i0 = tracker.get_next_index().unwrap();
        assert_eq!(i0, 0);
        let s0 = "string0".to_string();
        tracker.put(i0, s0.clone());
        assert_eq!(tracker.remove(i0).unwrap(), s0);
        // The tracker is now empty.

        // Put another string into the tracker. Don't remove it yet.
        let i1 = tracker.get_next_index().unwrap();
        assert_eq!(i1, 1);
        let s1 = "string1".to_string();
        tracker.put(i1, s1.clone());

        // Put another string into the tracker. Don't remove it yet.
        let i2 = tracker.get_next_index().unwrap();
        assert_eq!(i2, 0);
        let s2 = "string2".to_string();
        tracker.put(i2, s2.clone());

        // Check that we can't put any more strings into tracker
        assert!(tracker.get_next_index().is_none());

        // Check the strings are correct
        assert_eq!(tracker.remove(i1).unwrap(), s1);
        assert_eq!(tracker.remove(i2).unwrap(), s2);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn test_panic_if_wrong_index() {
        let mut tracker: Tracker<String> = Tracker::new(2);
        tracker.remove(100);
    }
}
