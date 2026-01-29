use crate::{range::Range, value::Type};
use itertools::Itertools;
use std::fmt::Debug;

use crate::value::{Kind, Value, ValueRange};

#[derive(Debug)]
pub struct Slice {
    range: Range,
    buffer: Vec<Value>,
}

impl Slice {
    pub fn from_range(kind: &Kind, range: Range) -> Self {
        Self {
            buffer: vec![Value::default(kind); range.length()],
            range,
        }
    }

    pub fn from_value_range<'a>(kind: &Kind, range: ValueRange<'a>) -> Self {
        Self {
            buffer: range
                .get_values()
                .iter()
                .map(|v| Value::from_u16(kind, *v))
                .collect(),
            range: range.get_range(),
        }
    }

    pub fn extend(&mut self, kind: &Kind, range: &Range) -> bool {
        // Extend slice while maintaining data consistency
        if range.end == self.range.start {
            let mut buffer: Vec<Value> = vec![];
            std::mem::swap(&mut buffer, &mut self.buffer);
            self.buffer = itertools::repeat_n(Value::default(kind), range.length())
                .chain(buffer)
                .collect();
            self.range = Range::new(range.start, range.length() + self.range.length());
            true
        } else if range.start == self.range.end {
            let mut buffer: Vec<Value> = vec![];
            std::mem::swap(&mut buffer, &mut self.buffer);
            self.buffer = buffer
                .into_iter()
                .chain(itertools::repeat_n(Value::default(kind), range.length()))
                .collect();
            self.range = Range::new(self.range.start, range.length() + self.range.length());
            true
        } else {
            false
        }
    }

    pub fn writable(&mut self, ty: &Type, range: &Range) -> bool {
        let in_range = range.start >= self.range.start && range.end <= self.range.end;
        if in_range {
            self.buffer
                .iter()
                .skip(range.start - self.range.start)
                .take(range.length())
                .fold_while(true, |_, mem| match mem {
                    Value::Write(t, _) | Value::Combined(t, _) | Value::Separated(t, _)
                        if t == ty =>
                    {
                        itertools::FoldWhile::Continue(true)
                    }
                    _ => itertools::FoldWhile::Done(false),
                })
                .into_inner()
        } else {
            in_range
        }
    }

    pub fn write(&mut self, range: &Range, values: &[u16]) -> bool {
        let in_range = range.start >= self.range.start && range.end <= self.range.end;
        let writable = range.length() == values.len() && in_range;
        if writable {
            for (mem, val) in self
                .buffer
                .iter_mut()
                .skip(range.start - self.range.start)
                .take(range.length())
                .zip(values.iter())
            {
                match mem {
                    Value::Write(_, w) => *w = *val,
                    Value::Combined(_, rw) => *rw = *val,
                    Value::Separated(_, (_, w)) => *w = *val,
                    Value::Read(_, _) => {}
                };
            }
        }
        writable
    }

    pub fn read(&self, range: &Range) -> Option<Vec<u16>> {
        let readable = range.start >= self.range.start && range.end <= self.range.end;
        if readable {
            self.buffer
                .iter()
                .skip(range.start - self.range.start)
                .take(range.length())
                .fold_while(Some(Vec::with_capacity(range.length())), |init, val| {
                    if let Some(mut values) = init {
                        match val {
                            Value::Read(_, r) => values.push(*r),
                            Value::Combined(_, rw) => values.push(*rw),
                            Value::Separated(_, (r, _)) => values.push(*r),
                            Value::Write(_, _) => return itertools::FoldWhile::Done(None),
                        };
                        itertools::FoldWhile::Continue(Some(values))
                    } else {
                        itertools::FoldWhile::Done(None)
                    }
                })
                .into_inner()
        } else {
            None
        }
    }

    pub fn readable(&self, ty: &Type, range: &Range) -> bool {
        let in_range = range.start >= self.range.start && range.end <= self.range.end;
        if in_range {
            self.buffer
                .iter()
                .skip(range.start - self.range.start)
                .take(range.length())
                .fold_while(true, |_, mem| match mem {
                    Value::Read(t, _) | Value::Combined(t, _) | Value::Separated(t, _)
                        if t == ty =>
                    {
                        itertools::FoldWhile::Continue(true)
                    }
                    _ => itertools::FoldWhile::Done(false),
                })
                .into_inner()
        } else {
            in_range
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Range, Slice, Type, Value, ValueRange};

    #[test]
    fn ut_slice_from_range() {
        let slice = Slice::from_range(&Kind::Read(Type::Coil), Range::new(123, 45));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for value in slice.buffer.iter() {
            assert_eq!(*value, Value::Read(Type::Coil, 0));
        }

        let slice = Slice::from_range(&Kind::Write(Type::Coil), Range::new(123, 45));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for value in slice.buffer.iter() {
            assert_eq!(*value, Value::Write(Type::Coil, 0));
        }

        let slice = Slice::from_range(&Kind::Combined(Type::Coil), Range::new(123, 45));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for value in slice.buffer.iter() {
            assert_eq!(*value, Value::Combined(Type::Coil, 0));
        }

        let slice = Slice::from_range(&Kind::Separated(Type::Coil), Range::new(123, 45));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for value in slice.buffer.iter() {
            assert_eq!(*value, Value::Separated(Type::Coil, (0, 0)));
        }
    }

    #[test]
    fn ut_slice_from_value_range() {
        let values: Vec<u16> = (1..46).collect();
        let slice = Slice::from_value_range(&Kind::Read(Type::Coil), ValueRange::new(123, &values));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for (v1, v2) in slice.buffer.iter().zip(values) {
            assert_eq!(*v1, Value::Read(Type::Coil, v2));
        }

        let values: Vec<u16> = (1..46).collect();
        let slice =
            Slice::from_value_range(&Kind::Write(Type::Coil), ValueRange::new(123, &values));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for (v1, v2) in slice.buffer.iter().zip(values) {
            assert_eq!(*v1, Value::Write(Type::Coil, v2));
        }

        let values: Vec<u16> = (1..46).collect();
        let slice =
            Slice::from_value_range(&Kind::Combined(Type::Coil), ValueRange::new(123, &values));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for (v1, v2) in slice.buffer.iter().zip(values) {
            assert_eq!(*v1, Value::Combined(Type::Coil, v2));
        }

        let values: Vec<u16> = (1..46).collect();
        let slice =
            Slice::from_value_range(&Kind::Separated(Type::Coil), ValueRange::new(123, &values));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        for (v1, v2) in slice.buffer.iter().zip(values) {
            assert_eq!(*v1, Value::Separated(Type::Coil, (v2, v2)));
        }
    }

    #[test]
    fn ut_slice_extend() {
        let mut slice = Slice::from_range(&Kind::Read(Type::Coil), Range::new(123, 45));
        assert_eq!(slice.buffer.len(), 45);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 168);
        assert!(slice.extend(&Kind::Write(Type::Coil), &Range::new(168, 32)));
        assert_eq!(slice.buffer.len(), 77);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 200);

        for (idx, value) in slice.buffer.iter().enumerate() {
            if idx < 45 {
                assert_eq!(*value, Value::Read(Type::Coil, 0));
            } else {
                assert_eq!(*value, Value::Write(Type::Coil, 0));
            }
        }

        assert!(!slice.extend(&Kind::Separated(Type::Coil), &Range::new(240, 10)));
        assert_eq!(slice.buffer.len(), 77);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 200);

        assert!(!slice.extend(&Kind::Separated(Type::Coil), &Range::new(0, 10)));
        assert_eq!(slice.buffer.len(), 77);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 200);

        assert!(slice.extend(&Kind::Separated(Type::Coil), &Range::new(200, 50)));
        assert_eq!(slice.buffer.len(), 127);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 250);

        assert!(slice.extend(&Kind::Combined(Type::Coil), &Range::new(250, 50)));
        assert_eq!(slice.buffer.len(), 177);
        assert_eq!(slice.range.start, 123);
        assert_eq!(slice.range.end, 300);

        assert!(slice.extend(&Kind::Combined(Type::Coil), &Range::new(0, 123)));
        assert_eq!(slice.buffer.len(), 300);
        assert_eq!(slice.range.start, 0);
        assert_eq!(slice.range.end, 300);

        for (idx, value) in slice.buffer.iter().enumerate() {
            if idx < 123 {
                assert_eq!(*value, Value::Combined(Type::Coil, 0));
            } else if idx < 168 {
                assert_eq!(*value, Value::Read(Type::Coil, 0));
            } else if idx < 200 {
                assert_eq!(*value, Value::Write(Type::Coil, 0));
            } else if idx < 250 {
                assert_eq!(*value, Value::Separated(Type::Coil, (0, 0)));
            } else if idx < 300 {
                assert_eq!(*value, Value::Combined(Type::Coil, 0));
            } else {
                unreachable!();
            }
        }
    }

    #[test]
    fn ut_slice_writable() {
        let mut slice = Slice::from_range(&Kind::Read(Type::Coil), Range::new(123, 45));
        assert!(slice.extend(&Kind::Write(Type::Coil), &Range::new(168, 32)));
        assert!(slice.extend(&Kind::Separated(Type::Coil), &Range::new(200, 50)));
        assert!(slice.extend(&Kind::Combined(Type::Coil), &Range::new(250, 50)));

        assert!(!slice.writable(&Type::Coil, &Range::new(130, 10)));
        assert!(slice.writable(&Type::Coil, &Range::new(175, 10)));
        assert!(slice.writable(&Type::Coil, &Range::new(210, 10)));
        assert!(slice.writable(&Type::Coil, &Range::new(270, 10)));
    }

    #[test]
    fn ut_slice_write() {
        let mut slice = Slice::from_range(&Kind::Read(Type::Coil), Range::new(123, 45));
        assert!(slice.extend(&Kind::Write(Type::Coil), &Range::new(168, 32)));
        assert!(slice.extend(&Kind::Separated(Type::Coil), &Range::new(200, 50)));
        assert!(slice.extend(&Kind::Combined(Type::Coil), &Range::new(250, 50)));

        let values: Vec<u16> = (1..21).collect();
        assert!(slice.write(&Range::new(175, 20), &values));
        for (v1, v2) in slice.buffer[175 - slice.range.start..]
            .iter()
            .take(20)
            .zip(values.iter())
        {
            match v1 {
                Value::Write(_, w) => assert_eq!(w, v2),
                Value::Separated(_, (_, w)) => assert_eq!(w, v2),
                Value::Read(_, _) => unreachable!(),
                Value::Combined(_, rw) => assert_eq!(rw, v2),
            };
        }

        let values: Vec<u16> = (1..21).map(|c| 2 * c).collect();
        assert!(slice.write(&Range::new(190, 20), &values));
        for (v1, v2) in slice.buffer[190 - slice.range.start..]
            .iter()
            .take(20)
            .zip(values.iter())
        {
            match v1 {
                Value::Write(_, w) => assert_eq!(w, v2),
                Value::Separated(_, (_, w)) => assert_eq!(w, v2),
                Value::Read(_, _) => unreachable!(),
                Value::Combined(_, rw) => assert_eq!(rw, v2),
            };
        }

        assert!(!slice.writable(&Type::Coil, &Range::new(0, 20)));
        assert!(!slice.writable(&Type::Coil, &Range::new(160, 20)));
        assert!(!slice.writable(&Type::Coil, &Range::new(500, 20)));

        let values: Vec<u16> = (1..21).map(|c| 3 * c).collect();
        assert!(!slice.write(&Range::new(0, 20), &values));

        let values: Vec<u16> = (1..21).map(|c| 4 * c).collect();
        assert!(slice.write(&Range::new(160, 20), &values));

        let values: Vec<u16> = (1..21).map(|c| 5 * c).collect();
        assert!(!slice.write(&Range::new(500, 20), &values));

        assert!(slice.writable(&Type::Coil, &Range::new(190, 20)));
        assert!(!slice.writable(&Type::Register, &Range::new(190, 20)));
    }

    #[test]
    fn ut_slice_readable() {
        let mut slice = Slice::from_range(&Kind::Read(Type::Coil), Range::new(123, 45));
        assert!(slice.extend(&Kind::Write(Type::Coil), &Range::new(168, 32)));
        assert!(slice.extend(&Kind::Separated(Type::Coil), &Range::new(200, 50)));
        assert!(slice.extend(&Kind::Combined(Type::Coil), &Range::new(250, 50)));

        assert!(slice.readable(&Type::Coil, &Range::new(130, 10)));
        assert!(!slice.readable(&Type::Coil, &Range::new(175, 10)));
        assert!(slice.readable(&Type::Coil, &Range::new(210, 10)));
        assert!(slice.readable(&Type::Coil, &Range::new(270, 10)));
    }

    #[test]
    fn ut_slice_read() {
        let mut slice = Slice::from_range(&Kind::Read(Type::Coil), Range::new(123, 45));
        assert!(slice.extend(&Kind::Write(Type::Coil), &Range::new(168, 32)));
        assert!(slice.extend(&Kind::Separated(Type::Coil), &Range::new(200, 50)));
        assert!(slice.extend(&Kind::Combined(Type::Coil), &Range::new(250, 50)));

        let values: Vec<u16> = (1..21).collect();
        for (v1, v2) in slice.buffer[130 - slice.range.start..]
            .iter_mut()
            .zip(values)
        {
            match v1 {
                Value::Write(_, _) => unreachable!(),
                Value::Separated(_, (r, _)) => *r = v2,
                Value::Read(_, r) => *r = v2,
                Value::Combined(_, rw) => *rw = v2,
            };
        }

        let result = slice.read(&Range::new(130, 20));
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.len(), 20);

        for (v1, v2) in slice.buffer[130 - slice.range.start..]
            .iter()
            .take(20)
            .zip(result.iter())
        {
            match v1 {
                Value::Write(_, _) => unreachable!(),
                Value::Separated(_, (r, _)) => assert_eq!(r, v2),
                Value::Read(_, r) => assert_eq!(r, v2),
                Value::Combined(_, rw) => assert_eq!(rw, v2),
            };
        }

        let values: Vec<u16> = (1..21).map(|c| 2 * c).collect();
        for (v1, v2) in slice.buffer[240 - slice.range.start..]
            .iter_mut()
            .zip(values)
        {
            match v1 {
                Value::Write(_, _) => unreachable!(),
                Value::Separated(_, (r, _)) => *r = v2,
                Value::Read(_, r) => *r = v2,
                Value::Combined(_, rw) => *rw = v2,
            };
        }

        let result = slice.read(&Range::new(240, 20));
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.len(), 20);

        for (v1, v2) in slice.buffer[240 - slice.range.start..]
            .iter()
            .take(20)
            .zip(result.iter())
        {
            match v1 {
                Value::Write(_, _) => unreachable!(),
                Value::Separated(_, (r, _)) => assert_eq!(r, v2),
                Value::Read(_, r) => assert_eq!(r, v2),
                Value::Combined(_, rw) => assert_eq!(rw, v2),
            };
        }

        assert!(slice.read(&Range::new(0, 20)).is_none());
        assert!(slice.read(&Range::new(190, 20)).is_none());
        assert!(slice.read(&Range::new(500, 20)).is_none());
    }
}
