use serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub fn new(start: usize, size: usize) -> Self {
        Self {
            start,
            end: start + size,
        }
    }
    pub fn length(&self) -> usize {
        return self.end - self.start;
    }
}
