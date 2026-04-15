#![feature(btree_cursors)]

mod memory;
mod range;
mod value;

pub mod slice;

pub use memory::Memory;
pub use range::Range;
pub use value::{Kind, Type, Value, ValueRange};
