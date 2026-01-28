use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fmt::{Debug, Display},
};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}, {})", self.start, self.end)
    }
}

impl Range {
    pub fn new(start: usize, size: usize) -> Self {
        Self {
            start,
            end: start + size,
        }
    }
    pub fn length(&self) -> usize {
        self.end - self.start
    }
}

impl Ord for Range {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.start < other.start {
            Ordering::Less
        } else if other.start < self.start {
            Ordering::Greater
        } else if self.end < other.end {
            Ordering::Less
        } else if self.end > other.end {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl PartialOrd for Range {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::Range;

    #[test]
    fn ut_range_new() {
        let range = Range::new(123, 45);
        assert_eq!(range.start, 123);
        assert_eq!(range.end, 168);

        let range = Range::new(321, 45);
        assert_eq!(range.start, 321);
        assert_eq!(range.end, 366);
    }

    #[test]
    fn ut_range_length() {
        let range = Range::new(123, 45);
        assert_eq!(range.length(), 45);

        let range = Range::new(321, 54);
        assert_eq!(range.length(), 54);
    }

    #[test]
    fn ut_range_cmp() {
        let range0 = Range::new(100, 100);

        let range1 = Range::new(0, 50);
        let range2 = Range::new(200, 50);
        let range3 = Range::new(50, 100);
        let range4 = Range::new(125, 50);
        let range5 = Range::new(100, 50);
        let range6 = Range::new(150, 50);
        let range7 = Range::new(100, 100);

        assert_eq!(range0.cmp(&range1), Ordering::Greater);
        assert_eq!(range1.cmp(&range0), Ordering::Less);

        assert_eq!(range0.cmp(&range2), Ordering::Less);
        assert_eq!(range2.cmp(&range0), Ordering::Greater);

        assert_eq!(range0.cmp(&range3), Ordering::Greater);
        assert_eq!(range3.cmp(&range0), Ordering::Less);

        assert_eq!(range0.cmp(&range4), Ordering::Less);
        assert_eq!(range4.cmp(&range0), Ordering::Greater);

        assert_eq!(range0.cmp(&range5), Ordering::Greater);
        assert_eq!(range5.cmp(&range0), Ordering::Less);

        assert_eq!(range0.cmp(&range6), Ordering::Less);
        assert_eq!(range6.cmp(&range0), Ordering::Greater);

        assert_eq!(range0.cmp(&range7), Ordering::Equal);
    }

    #[test]
    fn ut_range_partial_cmp() {
        let range0 = Range::new(100, 100);

        let range1 = Range::new(0, 50);
        let range2 = Range::new(200, 50);
        let range3 = Range::new(50, 100);
        let range4 = Range::new(125, 50);
        let range5 = Range::new(100, 50);
        let range6 = Range::new(150, 50);
        let range7 = Range::new(100, 100);

        assert_eq!(range0.partial_cmp(&range1), Some(Ordering::Greater));
        assert_eq!(range1.partial_cmp(&range0), Some(Ordering::Less));

        assert_eq!(range0.partial_cmp(&range2), Some(Ordering::Less));
        assert_eq!(range2.partial_cmp(&range0), Some(Ordering::Greater));

        assert_eq!(range0.partial_cmp(&range3), Some(Ordering::Greater));
        assert_eq!(range3.partial_cmp(&range0), Some(Ordering::Less));

        assert_eq!(range0.partial_cmp(&range4), Some(Ordering::Less));
        assert_eq!(range4.partial_cmp(&range0), Some(Ordering::Greater));

        assert_eq!(range0.partial_cmp(&range5), Some(Ordering::Greater));
        assert_eq!(range5.partial_cmp(&range0), Some(Ordering::Less));

        assert_eq!(range0.partial_cmp(&range6), Some(Ordering::Less));
        assert_eq!(range6.partial_cmp(&range0), Some(Ordering::Greater));

        assert_eq!(range0.partial_cmp(&range7), Some(Ordering::Equal));
    }
}
