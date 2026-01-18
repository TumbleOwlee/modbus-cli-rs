#![feature(btree_cursors)]

mod range;
mod value;

pub mod memory;
pub mod slice;

pub use range::Range;
pub use value::{Kind, Value, ValueRange};
