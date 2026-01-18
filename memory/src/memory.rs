use crate::range::Range;
use crate::slice::Slice;
use crate::value::{Kind, Type};
use std::collections::BTreeMap;
use std::{collections::HashMap, fmt::Debug, hash::Hash};

#[derive(Debug, Default)]
pub struct Memory<K>
where
    K: Hash + Eq + Clone + Default,
{
    slices: HashMap<K, BTreeMap<Range, Slice>>,
}

impl<K> Memory<K>
where
    K: Hash + Eq + Clone + Default,
{
    pub fn add_ranges(&mut self, id: K, kind: &Kind, ranges: &[Range]) -> bool {
        let mut ranges = ranges.iter();
        match self.slices.entry(id.clone()) {
            std::collections::hash_map::Entry::Vacant(e) => {
                if let Some(r) = ranges.next() {
                    let mut m = BTreeMap::new();
                    m.insert(r.clone(), Slice::from_range(kind, r.clone()));
                    e.insert(m);
                }
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }

        let m = self.slices.get_mut(&id).unwrap();
        for r in ranges {
            if let Some((_, slice)) = m
                .iter_mut()
                .find(|(range, _)| r.start == range.end || r.end == range.start)
            {
                if !slice.extend(kind, r) {
                    // Failed to extend slice, abort
                    return false;
                }
            } else {
                match m.iter().find(|(range, _)| {
                    (r.start > range.start && r.start < range.end)
                        || (r.end < range.end && r.end > range.start)
                }) {
                    Some(_) => return false,
                    None => {
                        m.insert(r.clone(), Slice::from_range(kind, r.clone()));
                    }
                }
            }
        }
        true
    }

    pub fn write(&mut self, id: K, ty: &Type, range: &Range, values: &[u16]) -> bool {
        if range.length() != values.len() || !self.writable(&id, ty, range) {
            return false;
        }

        let mut idx = 0;
        let mut range = range.clone();
        match self.slices.get_mut(&id) {
            Some(map) => {
                let mut cursor = map.upper_bound_mut(std::ops::Bound::Included(&range));
                if let Some((r, slice)) = cursor.peek_prev() {
                    let start = std::cmp::min(range.start, r.end);
                    let end = std::cmp::min(range.end, r.end);
                    let count = end - start;

                    if count != 0 {
                        slice.write(&Range::new(start, count), &values[idx..(idx + count)]);
                        range = Range::new(range.start + count, range.length() - count);
                        idx += count;
                    }
                }

                if range.length() != 0 {
                    while let Some((r, slice)) = cursor.peek_next() {
                        if r.start > range.start {
                            break;
                        }
                        let start = std::cmp::min(range.start, r.end);
                        let end = std::cmp::min(range.end, r.end);
                        let count = end - start;

                        if count != 0 {
                            slice.write(&Range::new(start, count), &values[idx..(idx + count)]);
                            range = Range::new(range.start + count, range.length() - count);
                            idx += count;

                            if range.length() == 0 {
                                break;
                            }
                        }
                    }
                }
                range.length() == 0
            }
            _ => false,
        }
    }

    pub fn writable(&mut self, id: &K, ty: &Type, range: &Range) -> bool {
        let mut range = range.clone();
        match self.slices.get_mut(id) {
            Some(map) => {
                let mut cursor = map.upper_bound_mut(std::ops::Bound::Included(&range));
                if let Some((r, slice)) = cursor.peek_prev() {
                    let start = std::cmp::min(range.start, r.end);
                    let end = std::cmp::min(range.end, r.end);
                    let count = end - start;

                    if count != 0 {
                        if !slice.writable(ty, &Range::new(start, count)) {
                            return false;
                        }
                        range = Range::new(range.start + count, range.length() - count);
                    }
                }

                if range.length() != 0 {
                    while let Some((r, slice)) = cursor.peek_next() {
                        if r.start > range.start {
                            break;
                        }
                        let start = std::cmp::min(range.start, r.end);
                        let end = std::cmp::min(range.end, r.end);
                        let count = end - start;

                        if count != 0 {
                            if !slice.writable(ty, &Range::new(start, count)) {
                                return false;
                            }
                            range = Range::new(range.start + count, range.length() - count);

                            if range.length() == 0 {
                                break;
                            }
                        }
                    }
                }
                range.length() == 0
            }
            _ => false,
        }
    }

    pub fn read(&self, id: K, ty: &Type, range: &Range) -> Option<Vec<u16>> {
        if !self.readable(&id, ty, range) {
            return None;
        }

        let mut range = range.clone();
        match self.slices.get(&id) {
            Some(map) => {
                let mut output: Vec<u16> = Vec::with_capacity(range.length());
                let cursor = map.upper_bound(std::ops::Bound::Included(&range));
                if let Some((r, slice)) = cursor.peek_prev() {
                    let start = std::cmp::min(range.start, r.end);
                    let end = std::cmp::min(range.end, r.end);
                    let count = end - start;

                    if count != 0 {
                        if let Some(mut v) = slice.read(&Range::new(start, count)) {
                            output.append(&mut v)
                        };
                        range = Range::new(range.start + count, range.length() - count);
                    }
                }

                if range.length() != 0 {
                    while let Some((r, slice)) = cursor.peek_next() {
                        if r.start > range.start {
                            break;
                        }
                        let start = std::cmp::min(range.start, r.end);
                        let end = std::cmp::min(range.end, r.end);
                        let count = end - start;

                        if count != 0 {
                            if let Some(mut v) = slice.read(&Range::new(start, count)) {
                                output.append(&mut v)
                            };
                            range = Range::new(range.start + count, range.length() - count);

                            if range.length() == 0 {
                                break;
                            }
                        }
                    }
                }

                if range.length() == 0 {
                    Some(output)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn readable(&self, id: &K, ty: &Type, range: &Range) -> bool {
        let mut range = range.clone();
        match self.slices.get(id) {
            Some(map) => {
                let cursor = map.upper_bound(std::ops::Bound::Included(&range));
                if let Some((r, slice)) = cursor.peek_prev() {
                    let start = std::cmp::min(range.start, r.end);
                    let end = std::cmp::min(range.end, r.end);
                    let count = end - start;

                    if count != 0 {
                        if !slice.readable(ty, &Range::new(start, count)) {
                            return false;
                        };
                        range = Range::new(range.start + count, range.length() - count);
                    }
                }

                if range.length() != 0 {
                    while let Some((r, slice)) = cursor.peek_next() {
                        if r.start > range.start {
                            break;
                        }
                        let start = std::cmp::min(range.start, r.end);
                        let end = std::cmp::min(range.end, r.end);
                        let count = end - start;

                        if count != 0 {
                            if !slice.readable(ty, &Range::new(start, count)) {
                                return false;
                            };
                            range = Range::new(range.start + count, range.length() - count);
                            if range.length() == 0 {
                                break;
                            }
                        }
                    }
                }

                range.length() == 0
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Kind, Memory, Type, Value, ValueRange};

    #[test]
    fn ut_memory() {
        assert_eq!(
            Value::default(&Kind::Read(Type::Coil)),
            Value::Read(Type::Coil, 0)
        );
        assert_eq!(
            Value::default(&Kind::Write(Type::Coil)),
            Value::Write(Type::Coil, 0)
        );
        assert_eq!(
            Value::default(&Kind::Combined(Type::Coil)),
            Value::Combined(Type::Coil, 0)
        );
        assert_eq!(
            Value::default(&Kind::Separated(Type::Coil)),
            Value::Separated(Type::Coil, (0, 0))
        );
    }
}
