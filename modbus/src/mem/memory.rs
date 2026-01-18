use crate::mem::range::Range;
use crate::mem::slice::Slice;
use anyhow::{anyhow, Error};
use std::{collections::HashMap, fmt::Debug, hash::Hash};

#[derive(Debug, Default)]
pub struct Memory<K>
where
    K: Hash + Eq + Clone + Default,
{
    slices: HashMap<K, Slice>,
}

impl<K> Memory<K>
where
    K: Hash + Eq + Clone + Default,
{
    pub fn add_ranges(&mut self, id: K, ranges: &[Range]) -> Result<(), Error> {
        if let std::collections::hash_map::Entry::Vacant(e) = self.slices.entry(id.clone()) {
            e.insert(Slice::from_ranges(ranges));
            Ok(())
        } else {
            self.slices.get_mut(&id).unwrap().add_ranges(ranges)
        }
    }

    pub fn write(&mut self, id: K, range: &Range, values: &[u16]) -> Result<(), Error> {
        match self.slices.get_mut(&id) {
            Some(slice) => slice.write(range, values)
            _ => Err(anyhow!("Requested to write invalid memory range")),
        }
    }

    pub fn read(&mut self, id: K, range: &Range) -> Result<&[u16], Error> {
        match self.slices.get_mut(&id) {
            Some(slice) => slice.read(range),
            _ => Err(anyhow!("Requested to read invalid memory range")),
        }
    }
}
