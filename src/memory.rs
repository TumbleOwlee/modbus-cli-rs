use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::default::Default;

#[derive(Serialize, Deserialize)]
pub struct Range<Key: Into<usize> + Clone>(Key, Key);

impl<Key: Into<usize> + Clone> Range<Key> {
    pub fn new(from: Key, to: Key) -> Self {
        Self(from, to)
    }

    pub fn length(&self) -> usize {
        self.1.clone().into() - self.0.clone().into()
    }
}

pub struct Memory<const SLICE_SIZE: usize, Value: Default + Copy> {
    slices: HashMap<usize, [Value; SLICE_SIZE]>,
}

impl<const SLICE_SIZE: usize, Value: Default + Copy> Memory<SLICE_SIZE, Value> {
    pub fn new<Key: Into<usize> + Clone>(range: Range<Key>) -> Self {
        let range = (range.0.into() / SLICE_SIZE, range.1.into() / SLICE_SIZE + 1);
        let mut slices = HashMap::with_capacity(range.1 - range.0);
        for i in range.0..range.1 {
            slices.insert(i, [Value::default(); SLICE_SIZE]);
        }
        Self { slices }
    }

    pub fn write<'a, Key: Into<usize> + Clone>(
        &mut self,
        range: Range<Key>,
        mut values: &'a [Value],
    ) -> anyhow::Result<&'a [Value]> {
        let range = (range.0.into(), range.1.into());
        let mut len = range.1 - range.0;
        if len != values.len() {
            return Err(anyhow!("Range too large/small for given value slice."));
        } else if !((range.0 / SLICE_SIZE)..(range.1 / SLICE_SIZE + 1))
            .all(|v| self.slices.contains_key(&v))
        {
            return Err(anyhow!("Range not available in memory."));
        }

        let mut start = std::cmp::min(range.0 % SLICE_SIZE, SLICE_SIZE);
        for idx in (range.0 / SLICE_SIZE)..(range.1 / SLICE_SIZE + 1) {
            let slice = self.slices.get_mut(&idx).expect("Slice does not exist.");
            let bound = std::cmp::min(len, SLICE_SIZE - start);
            slice[start..(start + bound)].copy_from_slice(&values[..bound]);
            values = &values[bound..];
            len -= bound;
            start = 0;
        }

        Ok(values)
    }

    pub fn read<Key: Into<usize> + Clone>(
        &mut self,
        range: &Range<Key>,
    ) -> anyhow::Result<Vec<&Value>> {
        let range = (range.0.clone().into(), range.1.clone().into());
        if !((range.0 / SLICE_SIZE)..(range.1 / SLICE_SIZE + 1))
            .all(|v| self.slices.contains_key(&v))
        {
            return Err(anyhow!("Range not available in memory."));
        }

        let mut len = range.1 - range.0;
        let mut vec = Vec::with_capacity(len);
        let mut start = std::cmp::min(range.0 % SLICE_SIZE, SLICE_SIZE);
        for idx in (range.0 / SLICE_SIZE)..(range.1 / SLICE_SIZE + 1) {
            let slice = self.slices.get(&idx).expect("Slice does not exist.");
            let bound = std::cmp::min(len, SLICE_SIZE - start);
            slice[start..(start + bound)]
                .iter()
                .for_each(|v| vec.push(v));
            len -= bound;
            start = 0;
        }

        Ok(vec)
    }
}