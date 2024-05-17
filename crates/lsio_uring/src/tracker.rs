use std::collections::VecDeque;

pub(crate) struct Tracker<T> {
    pub(crate) ops_in_flight: Vec<Option<T>>,
    pub(crate) next_index: VecDeque<usize>,
    len: usize,
}

impl<T> Tracker<T> {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            ops_in_flight: (0..n).map(|_| None).collect(),
            next_index: (0..n).collect(),
            len: 0,
        }
    }

    pub(crate) fn get_next_index(&mut self) -> Option<usize> {
        self.next_index.pop_front()
    }

    pub(crate) fn put(&mut self, index: usize, op: T) {
        self.ops_in_flight[index].replace(op);
        self.len += 1;
    }

    pub(crate) fn get(&mut self, index: usize) -> Option<TrackerGuard<T>> {
        if self.ops_in_flight[index].is_none() {
            None
        } else {
            Some(TrackerGuard {
                index,
                tracker: self,
            })
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn is_full(&self) -> bool {
        self.next_index.is_empty()
    }
}

pub(crate) struct TrackerGuard<'a, T> {
    index: usize,
    tracker: &'a mut Tracker<T>,
}

impl<'a, T> TrackerGuard<'a, T> {
    /// Safety: If TrackerGuard exists, then we know that `self.index` is valid.
    /// So `as_mut` can never fail.
    pub(crate) fn as_mut(&mut self) -> &mut T {
        self.tracker.ops_in_flight[self.index].as_mut().unwrap()
    }

    pub(crate) fn remove(&mut self) -> T {
        self.tracker.next_index.push_back(self.index);
        self.tracker.len -= 1;
        self.tracker.ops_in_flight[self.index].take().unwrap()
    }

    pub(crate) fn replace(&mut self, op: T) {
        self.tracker.ops_in_flight[self.index].replace(op);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_op_tracker() {
        let mut tracker = Tracker::new(2);

        // Check that removing an item before inserting an item returns None.
        assert!(tracker.get(0).is_none());

        // Put one string into the tracker, and then remove that string.
        let i0 = tracker.get_next_index().unwrap();
        assert_eq!(i0, 0);
        let s0 = "string0".to_string();
        tracker.put(i0, s0.clone());
        assert_eq!(tracker.get(i0).unwrap().remove(), s0);
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
        assert_eq!(tracker.get(i1).unwrap().remove(), s1);
        assert_eq!(tracker.get(i2).unwrap().remove(), s2);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn test_panic_if_wrong_index() {
        let mut tracker: Tracker<String> = Tracker::new(2);
        tracker.get(100);
    }
}
