//! A fixed-capacity ring buffer generic over the element type.
//!
//! [`Ring<T, CAP>`] holds up to `CAP` items of any type `T` in a FIFO order. Pushing into a full
//! ring evicts the oldest item. Storage is a single inline `[Option<T>; CAP]` array — no per-push
//! heap allocation for the buffer itself (individual `T`s may of course allocate). It is a
//! general-purpose container: timestamping and line truncation belong to the caller.

use std::array;

/// A fixed-capacity FIFO ring buffer of `CAP` items of type `T`.
///
/// Oldest-first iteration; pushing past capacity overwrites the oldest item.
pub struct Ring<T, const CAP: usize> {
    buf: [Option<T>; CAP],
    /// Index of the oldest item.
    head: usize,
    /// Number of items currently stored (`0..=CAP`).
    len: usize,
}

impl<T, const CAP: usize> Ring<T, CAP> {
    pub fn new() -> Self {
        Self {
            buf: array::from_fn(|_| None),
            head: 0,
            len: 0,
        }
    }

    /// The maximum number of items the ring can hold.
    pub const fn capacity(&self) -> usize {
        CAP
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the ring is at capacity (the next push will evict the oldest item).
    pub fn is_full(&self) -> bool {
        self.len == CAP
    }

    /// Appends `item`, evicting (and dropping) the oldest item if the ring is full.
    pub fn push(&mut self, item: T) {
        if CAP == 0 {
            return;
        }
        if self.len == CAP {
            // Full: overwrite the oldest slot and advance the head.
            self.buf[self.head] = Some(item);
            self.head = (self.head + 1) % CAP;
        } else {
            let idx = (self.head + self.len) % CAP;
            self.buf[idx] = Some(item);
            self.len += 1;
        }
    }

    /// Returns a reference to the oldest item, or `None` if empty.
    pub fn peek(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            self.buf[self.head].as_ref()
        }
    }

    /// Iterates over all items, oldest first. Double-ended, so callers can scan newest-first via
    /// `.rev()`.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        // Occupied slots are contiguous starting at `head` (wrapping); splitting at `head` yields
        // them in oldest-first order once the empty `None` slots are filtered out.
        let (front, back) = self.buf.split_at(self.head);
        back.iter().chain(front.iter()).filter_map(Option::as_ref)
    }

    /// Mutably iterates over all items, oldest first. Double-ended.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        let (front, back) = self.buf.split_at_mut(self.head);
        back.iter_mut()
            .chain(front.iter_mut())
            .filter_map(Option::as_mut)
    }

    /// Returns up to `n` items, oldest first, without removing them.
    pub fn peek_n(&self, n: usize) -> Vec<&T> {
        self.iter().take(n).collect()
    }

    /// Removes and returns the oldest item, or `None` if empty.
    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        let item = self.buf[self.head].take();
        self.head = (self.head + 1) % CAP;
        self.len -= 1;
        item
    }

    /// Removes and returns up to `n` items, oldest first.
    pub fn pop_n(&mut self, n: usize) -> Vec<T> {
        let count = n.min(self.len);
        (0..count).filter_map(|_| self.pop()).collect()
    }

    pub fn clear(&mut self) {
        for slot in &mut self.buf {
            *slot = None;
        }
        self.head = 0;
        self.len = 0;
    }
}

impl<T, const CAP: usize> Default for Ring<T, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const CAP: usize> FromIterator<T> for Ring<T, CAP> {
    /// Collects items into a ring; if more than `CAP` are provided, only the last `CAP` remain.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut ring = Self::new();
        for item in iter {
            ring.push(item);
        }
        ring
    }
}

#[cfg(test)]
mod tests {
    use super::Ring;

    #[test]
    fn ut_push_peek_pop() {
        let mut r: Ring<i32, 4> = Ring::new();
        assert!(r.is_empty());
        r.push(1);
        r.push(2);
        assert_eq!(r.len(), 2);
        assert_eq!(r.peek(), Some(&1));
        assert_eq!(r.pop(), Some(1));
        assert_eq!(r.pop(), Some(2));
        assert_eq!(r.pop(), None);
        assert!(r.is_empty());
    }

    #[test]
    fn ut_generic_over_string() {
        let mut r: Ring<String, 3> = Ring::new();
        r.push("a".to_string());
        r.push("b".to_string());
        assert_eq!(r.peek().map(String::as_str), Some("a"));
        let all: Vec<&str> = r.iter().map(String::as_str).collect();
        assert_eq!(all, ["a", "b"]);
    }

    #[test]
    fn ut_generic_over_tuple() {
        let mut r: Ring<(u64, String), 2> = Ring::new();
        r.push((1, "x".into()));
        r.push((2, "y".into()));
        let taken = r.pop_n(10);
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0], (1, "x".to_string()));
    }

    #[test]
    fn ut_full_capacity_holds_cap_items() {
        // Unlike the old log buffer (which reserved a slot), the ring holds the full CAP.
        let mut r: Ring<i32, 3> = Ring::new();
        r.push(1);
        r.push(2);
        r.push(3);
        assert!(r.is_full());
        assert_eq!(
            r.peek_n(10).into_iter().copied().collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn ut_overflow_evicts_oldest_in_order() {
        let mut r: Ring<i32, 3> = Ring::new();
        for i in 1..=5 {
            r.push(i);
        }
        // 1 and 2 evicted; window stays at 3, oldest first.
        assert_eq!(
            r.peek_n(10).into_iter().copied().collect::<Vec<_>>(),
            [3, 4, 5]
        );
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn ut_peek_n_and_pop_n_counts() {
        let mut r: Ring<i32, 5> = Ring::new();
        for i in 0..4 {
            r.push(i);
        }
        assert_eq!(r.peek_n(3).len(), 3);
        assert_eq!(r.peek_n(9).len(), 4);
        assert_eq!(r.peek_n(0).len(), 0);
        assert_eq!(r.pop_n(3).len(), 3);
        assert_eq!(r.pop_n(3).len(), 1);
        assert!(r.pop_n(3).is_empty());
    }

    #[test]
    fn ut_clear_then_reuse() {
        let mut r: Ring<i32, 4> = Ring::new();
        r.push(1);
        r.push(2);
        r.clear();
        assert!(r.is_empty());
        assert!(r.peek().is_none());
        r.push(9);
        assert_eq!(r.peek(), Some(&9));
    }

    #[test]
    fn ut_wraparound_after_pops() {
        // Exercise head wraparound: push, pop some, push past the physical end.
        let mut r: Ring<i32, 3> = Ring::new();
        r.push(1);
        r.push(2);
        assert_eq!(r.pop(), Some(1));
        r.push(3);
        r.push(4); // head has advanced; this wraps
        assert_eq!(
            r.peek_n(10).into_iter().copied().collect::<Vec<_>>(),
            [2, 3, 4]
        );
    }

    #[test]
    fn ut_iter_mut_in_order_and_reverse() {
        let mut r: Ring<i32, 3> = Ring::new();
        r.push(1);
        r.push(2);
        assert_eq!(r.pop(), Some(1));
        r.push(3);
        r.push(4); // wraps; order is [2, 3, 4]
        // Mutate in place, oldest-first.
        for (i, v) in r.iter_mut().enumerate() {
            *v += i as i32 * 10;
        }
        assert_eq!(
            r.peek_n(10).into_iter().copied().collect::<Vec<_>>(),
            [2, 13, 24]
        );
        // Reverse scan finds the newest matching item.
        let last = r.iter_mut().rev().find(|v| **v > 0);
        assert_eq!(last.copied(), Some(24));
    }

    #[test]
    fn ut_from_iter_keeps_last_cap() {
        let r: Ring<i32, 3> = (1..=6).collect();
        assert_eq!(
            r.peek_n(10).into_iter().copied().collect::<Vec<_>>(),
            [4, 5, 6]
        );
    }

    #[test]
    fn ut_drops_evicted_items() {
        use std::rc::Rc;
        let probe = Rc::new(());
        let mut r: Ring<Rc<()>, 2> = Ring::new();
        r.push(probe.clone());
        r.push(probe.clone());
        assert_eq!(Rc::strong_count(&probe), 3);
        r.push(probe.clone()); // evicts the oldest, dropping one ref
        assert_eq!(Rc::strong_count(&probe), 3);
        r.clear();
        assert_eq!(Rc::strong_count(&probe), 1);
    }
}
