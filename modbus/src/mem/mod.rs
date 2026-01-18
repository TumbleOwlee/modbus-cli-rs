pub mod format;
pub mod handler;
pub mod memory;
pub mod range;
pub mod slice;

use range::Range;
use serde::{Deserialize, Serialize};
use std::hash::Hash;

pub enum Request<K> {
    NoOperation,
    Shutdown,
    Read((K, Range)),
    Write((K, Range, Vec<u16>)),
}

pub enum Response {
    Values(Vec<u16>),
    Confirm,
    Error(anyhow::Error),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Layout<K>
where
    K: Hash + Eq + Clone + Default,
{
    id: K,
    range: Range,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config<K>(Vec<Layout<K>>)
where
    K: Hash + Eq + Clone + Default;

impl<K> Default for Config<K>
where
    K: Hash + Eq + Clone + Default,
{
    fn default() -> Self {
        Self(vec![])
    }
}
