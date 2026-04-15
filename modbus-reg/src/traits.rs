use std::borrow::Borrow;

pub trait ParseFromU8<V> {
    fn parse(self) -> V;
}

impl<I, V> ParseFromU8<V> for I
where
    I: Iterator<Item = u8>,
    V: Default + std::ops::Shl<usize, Output = V> + std::ops::AddAssign<V> + std::convert::From<u8>,
{
    fn parse(self) -> V {
        let mut output = V::default();
        for v in self {
            output = output << 8;
            output += v.into();
        }
        output
    }
}

pub trait IntoVec<T> {
    fn into_vec(self) -> anyhow::Result<Vec<T>>;
}

impl<I, T> IntoVec<u16> for I
where
    I: Iterator<Item = T>,
    T: Borrow<u8>,
{
    fn into_vec(self) -> anyhow::Result<Vec<u16>> {
        let mut values = vec![];
        let mut idx: usize = 0;
        let mut val: u16 = 0;
        for v in self {
            val &= *v.borrow() as u16;
            idx += 1;
            if idx.is_multiple_of(2) {
                values.push(val);
                val = 0;
            } else {
                val <<= 8;
            }
        }
        if idx != 0 {
            values.push(val << 8);
        }
        Ok(vec![])
    }
}
