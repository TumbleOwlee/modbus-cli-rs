use crate::range::Range;
use std::fmt::Debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Coil,
    Register,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Read(Type, u16),
    Write(Type, u16),
    Combined(Type, u16),
    Separated(Type, (u16, u16)),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Read(Type),
    Write(Type),
    Combined(Type),
    Separated(Type),
}

impl Value {
    pub fn default(kind: &Kind) -> Self {
        Self::from_u16(kind, 0)
    }

    pub fn from_u16(kind: &Kind, init: u16) -> Self {
        match kind {
            Kind::Read(t) => Value::Read(t.clone(), init),
            Kind::Write(t) => Value::Write(t.clone(), init),
            Kind::Combined(t) => Value::Combined(t.clone(), init),
            Kind::Separated(t) => Value::Separated(t.clone(), (init, init)),
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
    use super::{Kind, Type, Value, ValueRange};

    #[test]
    fn ut_value_default() {
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

    #[test]
    fn ut_value_from_u16() {
        assert_eq!(
            Value::from_u16(&Kind::Read(Type::Coil), 1),
            Value::Read(Type::Coil, 1)
        );
        assert_eq!(
            Value::from_u16(&Kind::Write(Type::Coil), 2),
            Value::Write(Type::Coil, 2)
        );
        assert_eq!(
            Value::from_u16(&Kind::Combined(Type::Coil), 3),
            Value::Combined(Type::Coil, 3)
        );
        assert_eq!(
            Value::from_u16(&Kind::Separated(Type::Coil), 4),
            Value::Separated(Type::Coil, (4, 4))
        );
        assert_ne!(
            Value::from_u16(&Kind::Separated(Type::Coil), 4),
            Value::Separated(Type::Coil, (4, 5))
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
