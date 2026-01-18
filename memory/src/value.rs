use crate::range::Range;
use std::fmt::Debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Read(u16),
    Write(u16),
    Combined(u16),
    Separated((u16, u16)),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Read,
    Write,
    Combined,
    Separated,
}

impl Value {
    pub fn default(kind: &Kind) -> Self {
        Self::from_u16(kind, 0)
    }

    pub fn from_u16(kind: &Kind, init: u16) -> Self {
        match kind {
            Kind::Read => Value::Read(init),
            Kind::Write => Value::Write(init),
            Kind::Combined => Value::Combined(init),
            Kind::Separated => Value::Separated((init, init)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValueRange<'a> {
    range: Range,
    values: &'a [u16],
}

impl<'a> ValueRange<'a> {
    pub fn new(start: usize, values: &'a [u16]) -> Self {
        Self {
            range: Range::new(start, values.len()),
            values,
        }
    }

    pub fn get_values(&self) -> &'a [u16] {
        self.values
    }

    pub fn get_range(&self) -> Range {
        self.range.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Value, ValueRange};

    #[test]
    fn ut_value_default() {
        assert_eq!(Value::default(&Kind::Read), Value::Read(0));
        assert_eq!(Value::default(&Kind::Write), Value::Write(0));
        assert_eq!(Value::default(&Kind::Combined), Value::Combined(0));
        assert_eq!(Value::default(&Kind::Separated), Value::Separated((0, 0)));
    }

    #[test]
    fn ut_value_from_u16() {
        assert_eq!(Value::from_u16(&Kind::Read, 1), Value::Read(1));
        assert_eq!(Value::from_u16(&Kind::Write, 2), Value::Write(2));
        assert_eq!(Value::from_u16(&Kind::Combined, 3), Value::Combined(3));
        assert_eq!(
            Value::from_u16(&Kind::Separated, 4),
            Value::Separated((4, 4))
        );
        assert_ne!(
            Value::from_u16(&Kind::Separated, 4),
            Value::Separated((4, 5))
        );
    }

    #[test]
    fn ut_value_range_new() {
        let values: Vec<u16> = (1..21).collect();
        let range = ValueRange::new(100, &values);

        assert_eq!(range.range.start, 100);
        assert_eq!(range.range.end, 120);
    }
}
