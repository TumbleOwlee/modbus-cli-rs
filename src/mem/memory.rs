use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use tokio_modbus::prelude::SlaveId;

const SLICE_SIZE: usize = 1024;

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Range<Key: Into<usize> + Clone> {
    start: Key,
    end: Key,
}

impl<Key: Into<usize> + Clone + Debug> Range<Key> {
    pub fn new(start: Key, end: Key) -> Self {
        if start.clone().into() > end.clone().into() {
            panic!("Range invalid: end is lower than start.");
        }
        Self { start, end }
    }

    pub fn length(&self) -> usize {
        self.end.clone().into() - self.start.clone().into()
    }

    pub fn start(&self) -> usize {
        self.start.clone().into()
    }

    pub fn end(&self) -> usize {
        self.end.clone().into()
    }
}

pub struct Memory {
    slices: HashMap<(SlaveId, usize), [u16; SLICE_SIZE]>,
    bounds: HashMap<SlaveId, Range<usize>>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            slices: HashMap::new(),
            bounds: HashMap::new(), //Range::new(usize::max_value(), usize::max_value()),
        }
    }

    pub fn init<Key: Into<usize> + Clone + Debug>(
        &mut self,
        slave: SlaveId,
        ranges: &[Range<Key>],
    ) {
        let bounds = self
            .bounds
            .entry(slave)
            .or_insert(Range::new(usize::max_value(), usize::max_value()));
        for range in ranges.iter() {
            let upper = if bounds.end() == usize::max_value() {
                0
            } else {
                bounds.end()
            };
            bounds.start = std::cmp::min(range.start(), bounds.start());
            bounds.end = std::cmp::max(range.end(), upper);
            let range = (range.start() / SLICE_SIZE, range.end() / SLICE_SIZE + 1);
            for i in range.0..range.1 {
                self.slices
                    .entry((slave, i))
                    .or_insert_with(|| [0; SLICE_SIZE]);
            }
        }
    }

    pub fn write<'a, Key: Into<usize> + Clone + Debug>(
        &mut self,
        slave: SlaveId,
        range: Range<Key>,
        mut values: &'a [u16],
    ) -> anyhow::Result<&'a [u16]> {
        let range = (range.start(), range.end());
        let bounds = self.bounds.get(&slave);
        if bounds.is_none() {
            return Err(anyhow!("Invalid memory address ({slave}, {range:?})"));
        }
        let bounds = bounds.unwrap();
        if bounds.start() > range.0 || bounds.end() < range.1 {
            return Err(anyhow!(
                "Range not available in memory ({slave}, {range:?})"
            ));
        }
        let mut len = range.1 - range.0;
        if len != values.len() {
            return Err(anyhow!("Range too large/small for given value slice."));
        } else if !((range.0 / SLICE_SIZE)..(range.1 / SLICE_SIZE + 1))
            .all(|v| self.slices.contains_key(&(slave, v)))
        {
            return Err(anyhow!(
                "Range not available in memory ({slave}, {range:?})"
            ));
        }

        let mut start = std::cmp::min(range.0 % SLICE_SIZE, SLICE_SIZE);
        for idx in (range.0 / SLICE_SIZE)..(range.1 / SLICE_SIZE + 1) {
            let slice = self
                .slices
                .get_mut(&(slave, idx))
                .expect("Slice does not exist.");
            let bound = std::cmp::min(len, SLICE_SIZE - start);
            slice[start..(start + bound)].copy_from_slice(&values[..bound]);
            values = &values[bound..];
            len -= bound;
            start = 0;
        }

        Ok(values)
    }

    pub fn read<Key: Into<usize> + Clone + Debug>(
        &mut self,
        slave: SlaveId,
        range: &Range<Key>,
    ) -> anyhow::Result<Vec<&u16>> {
        let bounds = self.bounds.get(&slave);
        if bounds.is_none() {
            return Err(anyhow!("Invalid memory address ({slave}, {range:?})"));
        }
        let bounds = bounds.unwrap();
        if bounds.start() > range.end()
            || bounds.end() < range.end()
            || !((range.start() / SLICE_SIZE)..(range.end() / SLICE_SIZE + 1))
                .all(|v| self.slices.contains_key(&(slave, v)))
        {
            let r = range.clone();
            self.init(slave, &[r]);
        }

        let mut len = range.end() - range.start();
        let mut vec = Vec::with_capacity(len);
        let mut start = std::cmp::min(range.start() % SLICE_SIZE, SLICE_SIZE);
        for idx in (range.start() / SLICE_SIZE)..(range.end() / SLICE_SIZE + 1) {
            let slice = self
                .slices
                .get(&(slave, idx))
                .expect("Slice does not exist.");
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
