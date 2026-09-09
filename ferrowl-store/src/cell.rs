use crate::range::Range;
use std::fmt::Debug;

/// The Modbus data type a memory cell holds.
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum CellType {
    /// Single-bit value (Modbus coil / discrete input).
    Coil,
    /// 16-bit value (Modbus holding / input register).
    Register,
}

/// Access direction permitted on a memory cell or a declared region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}

/// Access kind of a memory region: cell type plus allowed direction, without
/// a value. Used to declare ranges via [`Memory::add_ranges`](crate::Memory::add_ranges).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellKind {
    pub ty: CellType,
    pub access: Access,
}

impl CellKind {
    /// `ty` with a fixed [`Access::Read`] direction.
    pub fn read(ty: CellType) -> Self {
        Self {
            ty,
            access: Access::Read,
        }
    }

    /// `ty` with a fixed [`Access::Write`] direction.
    pub fn write(ty: CellType) -> Self {
        Self {
            ty,
            access: Access::Write,
        }
    }

    /// `ty` with a fixed [`Access::ReadWrite`] direction.
    pub fn read_write(ty: CellType) -> Self {
        Self {
            ty,
            access: Access::ReadWrite,
        }
    }
}

/// A single memory cell: its kind and current value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub(crate) kind: CellKind,
    pub(crate) value: u16,
}

impl Cell {
    /// Creates a zero-initialized cell with the access rights of `kind`.
    pub fn zeroed(kind: &CellKind) -> Self {
        Self::from_u16(kind, 0)
    }

    /// Creates a cell with the access rights of `kind`, initialized to `init`.
    pub fn from_u16(kind: &CellKind, init: u16) -> Self {
        Self {
            kind: *kind,
            value: init,
        }
    }

    /// Returns `true` if this cell accepts checked writes of type `ty`.
    pub fn accepts_write(&self, ty: &CellType) -> bool {
        self.kind.ty == *ty && matches!(self.kind.access, Access::Write | Access::ReadWrite)
    }

    /// Returns `true` if this cell accepts checked reads of type `ty`.
    pub fn accepts_read(&self, ty: &CellType) -> bool {
        self.kind.ty == *ty && matches!(self.kind.access, Access::Read | Access::ReadWrite)
    }

    /// Sets the stored value if this cell accepts writes; leaves read-only cells untouched.
    pub fn try_set_value(&mut self, val: u16) {
        if self.kind.access != Access::Read {
            self.value = val;
        }
    }

    /// Returns the stored value if this cell accepts reads, `None` for write-only cells.
    pub fn try_value(&self) -> Option<u16> {
        (self.kind.access != Access::Write).then_some(self.value)
    }
}

/// A borrowed run of raw `u16` values paired with the address [`Range`]
/// they occupy. The range length always equals the number of values.
#[derive(Debug, Clone)]
pub struct ValueRange<'a> {
    range: Range,
    values: &'a [u16],
}

impl<'a> ValueRange<'a> {
    /// Creates a value range starting at address `start`; the range end is
    /// derived from the slice length.
    pub fn new(start: usize, values: &'a [u16]) -> Self {
        Self {
            range: Range::new(start, values.len()),
            values,
        }
    }

    pub fn values(&self) -> &'a [u16] {
        self.values
    }

    /// Returns the address range covered by the values.
    pub fn range(&self) -> Range {
        self.range.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cell, CellKind, CellType, ValueRange};

    #[test]
    /// MB-R-030 — a cell carries both a cell type and an access direction (read/write/read-write).
    fn ut_value_zeroed() {
        assert_eq!(
            Cell::zeroed(&CellKind::read(CellType::Coil)),
            Cell {
                kind: CellKind::read(CellType::Coil),
                value: 0
            }
        );
        assert_eq!(
            Cell::zeroed(&CellKind::write(CellType::Coil)),
            Cell {
                kind: CellKind::write(CellType::Coil),
                value: 0
            }
        );
        assert_eq!(
            Cell::zeroed(&CellKind::read_write(CellType::Coil)),
            Cell {
                kind: CellKind::read_write(CellType::Coil),
                value: 0
            }
        );
    }

    #[test]
    /// MB-R-030 — a value-initialized cell preserves its cell type and access direction.
    fn ut_value_from_u16() {
        assert_eq!(
            Cell::from_u16(&CellKind::read(CellType::Coil), 1),
            Cell {
                kind: CellKind::read(CellType::Coil),
                value: 1
            }
        );
        assert_eq!(
            Cell::from_u16(&CellKind::write(CellType::Coil), 2),
            Cell {
                kind: CellKind::write(CellType::Coil),
                value: 2
            }
        );
        assert_eq!(
            Cell::from_u16(&CellKind::read_write(CellType::Coil), 3),
            Cell {
                kind: CellKind::read_write(CellType::Coil),
                value: 3
            }
        );
    }

    #[test]
    fn ut_value_range_new() {
        let values: Vec<u16> = (1..21).collect();
        let range = ValueRange::new(100, &values);

        assert_eq!(range.range.start, 100);
        assert_eq!(range.range.end, 120);
    }

    #[test]
    /// MB-R-030 — a region's declared kind exposes its underlying cell type independent of access direction.
    fn ut_kind_get_type() {
        assert_eq!(CellKind::read(CellType::Coil).ty, CellType::Coil);
        assert_eq!(CellKind::write(CellType::Register).ty, CellType::Register);
        assert_eq!(CellKind::read_write(CellType::Coil).ty, CellType::Coil);
    }

    #[test]
    fn ut_value_range_accessors() {
        let values: Vec<u16> = vec![7, 8, 9];
        let range = ValueRange::new(50, &values);
        assert_eq!(range.values(), &[7, 8, 9]);
        assert_eq!(range.range(), crate::range::Range::new(50, 3));
    }
}
