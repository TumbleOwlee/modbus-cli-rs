use crate::mem::range::Range;
use anyhow::{anyhow, Error};
use std::fmt::Debug;

#[derive(Debug)]
pub struct Slice {
    range: Range,
    buffer: Vec<u16>,
}

impl Slice {
    pub fn from_range(range: Range) -> Self {
        Self {
            buffer: vec![0; range.length()],
            range,
        }
    }

    pub fn from_ranges(ranges: &[Range]) -> Self {
        let range = ranges
            .iter()
            .fold(None, |init, range| match init {
                None => Some(range.clone()),
                Some(r) => Some(Range::new(
                    std::cmp::min(r.start, range.start),
                    std::cmp::max(r.end, range.end),
                )),
            })
            .expect("Slice of ranges has to contain at least a single range");
        Self::from_range(range)
    }

    fn extend(&mut self, range: Range) -> Result<(), Error> {
        // Extend slice while maintaining data consistency
        if range.start < self.range.start || range.end > self.range.end {
            let b =
                vec![0; std::cmp::max(self.range.start - range.start, range.end - self.range.end)];

            let s1 = &b[0..(self.range.start - range.start)];
            let s2 = &self.buffer[0..];
            let s3 = &b[0..(self.range.end - range.end)];

            let buffer: Vec<u16> = itertools::merge(itertools::merge(s1, s2), s3)
                .cloned()
                .collect();
            self.buffer = buffer;
            Ok(())
        } else {
            Err(anyhow!("Tried to extend with smaller range"))
        }
    }

    pub fn add_range(&mut self, range: Range) -> Result<(), Error> {
        let r = Range::new(
            std::cmp::min(range.start, self.range.start),
            std::cmp::max(range.end, self.range.end),
        );
        self.extend(r)
    }

    pub fn add_ranges(&mut self, ranges: &[Range]) -> Result<(), Error> {
        let r = ranges.iter().fold(self.range.clone(), |init, range| {
            Range::new(
                std::cmp::min(range.start, init.start),
                std::cmp::max(range.end, init.end),
            )
        });
        self.extend(r)
    }

    pub fn write(&mut self, range: &Range, values: &[u16]) -> Result<(), Error> {
        if range.start >= self.range.start && range.end < self.range.end {
            let idx = range.start - self.range.start;
            for (offset, value) in values.iter().enumerate() {
                self.buffer[idx + offset] = *value;
            }
            Ok(())
        } else {
            Err(anyhow!("Requested to write invalid memory range"))
        }
    }

    pub fn read(&mut self, range: &Range) -> Result<&[u16], Error> {
        if range.start >= self.range.start && range.end < self.range.end {
            let idx = range.start - self.range.start;
            Ok(&self.buffer[idx..(idx + range.length())])
        } else {
            Err(anyhow!("Requested to read invalid memory range"))
        }
    }
}
