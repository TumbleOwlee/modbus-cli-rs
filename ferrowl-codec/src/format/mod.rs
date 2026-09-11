//! Data formats describing how raw register words are interpreted.

/// Implements `Display` for an enum whose variants each map to one fixed literal string. The
/// literal is written as an argument, never as the format string, so text containing `{` or `}`
/// is emitted verbatim rather than parsed as a substitution. Passing it as an argument also means
/// any literal is accepted, not only a string one; every current caller passes a string, and a
/// non-string literal would simply render through its own `Display`.
macro_rules! display_by_variant {
    ($ty:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl std::fmt::Display for $ty {
            fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    $($ty::$variant => write!(fmt, "{}", $text)),+
                }
            }
        }
    };
}
pub(crate) use display_by_variant;

/// Declares a fieldless "kind" enum together with `ALL`, an array listing every
/// variant in declaration order.
macro_rules! kind_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            /// Every variant, in declaration order. Generated together with the
            /// enum above, so a variant cannot exist without appearing here.
            /// The order is load-bearing wherever a consumer builds an ordered
            /// list from it (e.g. a format-picker dropdown), not incidental.
            pub const ALL: [$name; kind_enum!(@count $($variant),+)] = [
                $($name::$variant),+
            ];
        }
    };
    (@count $($variant:ident),+) => {
        [$(kind_enum!(@unit $variant)),+].len()
    };
    (@unit $variant:ident) => { () };
}
pub(crate) use kind_enum;

mod alignment;
mod bitfield;
mod endian;
mod float_kind;
mod int_kind;
mod scalar;
mod word_order;

pub use alignment::Alignment;
pub use bitfield::BitField;
pub use endian::Endian;
pub use float_kind::FloatKind;
pub use int_kind::IntKind;
pub use scalar::{Resolution, Width};
pub use word_order::WordOrder;

/// Numeric format carrying an integer primitive: byte order, register order,
/// display resolution, and the [`BitField`] selecting a sub-field.
#[derive(Debug, Clone, PartialEq)]
pub struct NumericFormat {
    pub kind: IntKind,
    pub endian: Endian,
    pub word_order: WordOrder,
    pub resolution: Resolution,
    pub bit_field: BitField,
}

/// Numeric format carrying a float primitive: byte order, register order and
/// display resolution. Floats carry no [`BitField`].
#[derive(Debug, Clone, PartialEq)]
pub struct FloatFormat {
    pub kind: FloatKind,
    pub endian: Endian,
    pub word_order: WordOrder,
    pub resolution: Resolution,
}

/// How the raw register words of a value are interpreted.
#[derive(Debug, Clone, PartialEq)]
pub enum Format {
    Numeric(NumericFormat),
    Float(FloatFormat),
    Ascii(Alignment, Width),
}

impl std::fmt::Display for Format {
    /// MB-R-154 — name plus a parenthesized qualifier: byte order for a numeric
    /// format, alignment for `Ascii`. Register order, resolution and the
    /// bit-field selector never appear.
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Numeric(nf) => write!(fmt, "{} ({})", nf.kind, nf.endian),
            Self::Float(ff) => write!(fmt, "{} ({})", ff.kind, ff.endian),
            Self::Ascii(alignment, _) => write!(fmt, "ASCII ({alignment})"),
        }
    }
}

impl Format {
    /// The width of the format in Modbus registers (`u16` words).
    pub fn width(&self) -> usize {
        match self {
            Self::Numeric(nf) => nf.kind.register_width(),
            Self::Float(ff) => ff.kind.register_width(),
            Self::Ascii(_, w) => w.0,
        }
    }

    /// The display scale factor, or `None` for ASCII formats.
    pub fn resolution(&self) -> Option<Resolution> {
        match self {
            Self::Numeric(nf) => Some(nf.resolution.clone()),
            Self::Float(ff) => Some(ff.resolution.clone()),
            Self::Ascii(_, _) => None,
        }
    }

    /// The [`BitField`] selector for integer formats, or the no-op default
    /// (full mask, shift 0) for float and ASCII formats.
    pub fn bitfield(&self) -> BitField {
        match self {
            Self::Numeric(nf) => nf.bit_field.clone(),
            Self::Float(_) | Self::Ascii(_, _) => BitField::default(),
        }
    }

    /// The register (word) order for numeric formats, or the `Normal` no-op
    /// default for ASCII.
    pub fn word_order(&self) -> WordOrder {
        match self {
            Self::Numeric(nf) => nf.word_order,
            Self::Float(ff) => ff.word_order,
            Self::Ascii(_, _) => WordOrder::Normal,
        }
    }

    /// The length of the format in bytes (two per register).
    pub fn length(&self) -> usize {
        self.width() * 2
    }

    pub fn u8(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::U8,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn u16(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::U16,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn u32(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::U32,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn u64(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::U64,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn u128(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::U128,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn i8(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::I8,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn i16(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::I16,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn i32(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::I32,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn i64(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::I64,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn i128(
        endian: Endian,
        word_order: WordOrder,
        resolution: Resolution,
        bit_field: BitField,
    ) -> Self {
        Self::Numeric(NumericFormat {
            kind: IntKind::I128,
            endian,
            word_order,
            resolution,
            bit_field,
        })
    }
    pub fn f32(endian: Endian, word_order: WordOrder, resolution: Resolution) -> Self {
        Self::Float(FloatFormat {
            kind: FloatKind::F32,
            endian,
            word_order,
            resolution,
        })
    }
    pub fn f64(endian: Endian, word_order: WordOrder, resolution: Resolution) -> Self {
        Self::Float(FloatFormat {
            kind: FloatKind::F64,
            endian,
            word_order,
            resolution,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Alignment, BitField, Endian, FloatKind, Format, IntKind, Resolution, Width, WordOrder,
    };

    /// A display literal is emitted verbatim, braces included — the macro passes it as an
    /// argument, so it is never parsed as a format string. No current variant carries a brace;
    /// this pins the property before one does.
    #[test]
    fn ut_display_by_variant_emits_braces_verbatim() {
        #[derive(Debug)]
        enum Braced {
            Curly,
        }
        super::display_by_variant!(Braced { Curly => "{not a placeholder}" });

        assert_eq!(Braced::Curly.to_string(), "{not a placeholder}");
    }

    fn res() -> Resolution {
        Resolution(1.0)
    }

    fn bf() -> BitField {
        BitField::default()
    }

    #[test]
    /// MB-R-011 — per-format register widths (1/1/1/1, 2/2/2, 4/4/4, 8/8, Ascii = width).
    fn ut_format_width() {
        assert_eq!(Format::Ascii(Alignment::Left, Width(4)).width(), 4);
        assert_eq!(
            Format::u8(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            1
        );
        assert_eq!(
            Format::u16(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            1
        );
        assert_eq!(
            Format::i8(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            1
        );
        assert_eq!(
            Format::i16(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            1
        );
        assert_eq!(
            Format::u32(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            2
        );
        assert_eq!(
            Format::i32(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            2
        );
        assert_eq!(
            Format::f32(Endian::Big, WordOrder::Normal, res()).width(),
            2
        );
        assert_eq!(
            Format::u64(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            4
        );
        assert_eq!(
            Format::i64(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            4
        );
        assert_eq!(
            Format::f64(Endian::Big, WordOrder::Normal, res()).width(),
            4
        );
        assert_eq!(
            Format::u128(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            8
        );
        assert_eq!(
            Format::i128(Endian::Big, WordOrder::Normal, res(), bf()).width(),
            8
        );
    }

    #[test]
    /// MB-R-011 — byte length is twice the register width.
    fn ut_format_length() {
        assert_eq!(
            Format::u8(Endian::Big, WordOrder::Normal, res(), bf()).length(),
            2
        );
        assert_eq!(
            Format::u32(Endian::Big, WordOrder::Normal, res(), bf()).length(),
            4
        );
        assert_eq!(
            Format::u64(Endian::Big, WordOrder::Normal, res(), bf()).length(),
            8
        );
        assert_eq!(
            Format::u128(Endian::Big, WordOrder::Normal, res(), bf()).length(),
            16
        );
        assert_eq!(Format::Ascii(Alignment::Left, Width(3)).length(), 6);
    }

    #[test]
    /// MB-R-017 — float (and ASCII) formats carry no bit-field; their bit-field is the no-op full mask.
    fn ut_format_bitfield() {
        // Integer carries its BitField; float/ASCII report the no-op default.
        let bitfield = BitField { mask: 0xFF00 };
        assert_eq!(
            Format::u16(Endian::Big, WordOrder::Normal, res(), bitfield.clone()).bitfield(),
            bitfield
        );
        assert_eq!(
            Format::u16(Endian::Big, WordOrder::Normal, res(), bitfield)
                .bitfield()
                .shift(),
            8
        );
        assert!(
            Format::f32(Endian::Big, WordOrder::Normal, res())
                .bitfield()
                .is_full()
        );
        assert!(
            Format::Ascii(Alignment::Left, Width(1))
                .bitfield()
                .is_full()
        );
    }

    #[test]
    /// MB-R-021 — every numeric format carries a display resolution; ASCII carries none.
    fn ut_format_resolution() {
        let r = Resolution(0.5);
        assert!(
            Format::Ascii(Alignment::Left, Width(1))
                .resolution()
                .is_none()
        );
        assert_eq!(
            Format::u8(Endian::Big, WordOrder::Normal, r.clone(), bf())
                .resolution()
                .unwrap()
                .0,
            0.5
        );
        assert_eq!(
            Format::i16(Endian::Little, WordOrder::Normal, r.clone(), bf())
                .resolution()
                .unwrap()
                .0,
            0.5
        );
        assert_eq!(
            Format::f32(Endian::Big, WordOrder::Normal, r.clone())
                .resolution()
                .unwrap()
                .0,
            0.5
        );
    }

    #[test]
    /// MB-R-021 — every numeric format variant carries a display resolution.
    fn ut_format_resolution_all_variants() {
        let r = Resolution(0.25);
        let e = Endian::Big;
        for f in [
            Format::u16(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::i8(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::u32(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::i32(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::u64(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::i64(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::u128(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::i128(e.clone(), WordOrder::Normal, r.clone(), bf()),
            Format::f64(e.clone(), WordOrder::Normal, r.clone()),
        ] {
            assert_eq!(f.resolution().unwrap().0, 0.25);
        }
    }

    #[test]
    /// MB-R-017 — every integer variant carries its bit-field; the float variant reports the no-op default.
    fn ut_format_bitfield_all_variants() {
        let m = BitField { mask: 0x0FF0 };
        let e = Endian::Big;
        for f in [
            Format::u8(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::u32(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::u64(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::u128(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::i8(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::i16(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::i32(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::i64(e.clone(), WordOrder::Normal, res(), m.clone()),
            Format::i128(e.clone(), WordOrder::Normal, res(), m.clone()),
        ] {
            assert_eq!(f.bitfield(), m);
        }
        // Float variant reports the no-op default.
        assert!(
            Format::f64(e, WordOrder::Normal, res())
                .bitfield()
                .is_full()
        );
    }

    #[test]
    /// MB-R-154, MB-R-218 — a format's display text is its name plus a parenthesized qualifier:
    /// byte order for numeric formats (both `Big`/`Little`), alignment for `Ascii`
    /// (both `Left`/`Right`); `Ascii` itself renders as `ASCII`.
    fn ut_format_display_all_variants() {
        assert_eq!(
            Format::Ascii(Alignment::Left, Width(2)).to_string(),
            "ASCII (Left)"
        );
        assert_eq!(
            Format::Ascii(Alignment::Right, Width(2)).to_string(),
            "ASCII (Right)"
        );
        let e = Endian::Big;
        assert_eq!(
            Format::u8(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U8 (Big Endian)"
        );
        assert_eq!(
            Format::u16(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U16 (Big Endian)"
        );
        assert_eq!(
            Format::u32(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U32 (Big Endian)"
        );
        assert_eq!(
            Format::u64(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U64 (Big Endian)"
        );
        assert_eq!(
            Format::u128(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U128 (Big Endian)"
        );
        assert_eq!(
            Format::i8(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I8 (Big Endian)"
        );
        assert_eq!(
            Format::i16(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I16 (Big Endian)"
        );
        assert_eq!(
            Format::i32(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I32 (Big Endian)"
        );
        assert_eq!(
            Format::i64(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I64 (Big Endian)"
        );
        assert_eq!(
            Format::i128(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I128 (Big Endian)"
        );
        assert_eq!(
            Format::f32(e.clone(), WordOrder::Normal, res()).to_string(),
            "F32 (Big Endian)"
        );
        assert_eq!(
            Format::f64(e, WordOrder::Normal, res()).to_string(),
            "F64 (Big Endian)"
        );

        let e = Endian::Little;
        assert_eq!(
            Format::u8(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U8 (Little Endian)"
        );
        assert_eq!(
            Format::u16(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U16 (Little Endian)"
        );
        assert_eq!(
            Format::u32(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U32 (Little Endian)"
        );
        assert_eq!(
            Format::u64(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U64 (Little Endian)"
        );
        assert_eq!(
            Format::u128(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "U128 (Little Endian)"
        );
        assert_eq!(
            Format::i8(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I8 (Little Endian)"
        );
        assert_eq!(
            Format::i16(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I16 (Little Endian)"
        );
        assert_eq!(
            Format::i32(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I32 (Little Endian)"
        );
        assert_eq!(
            Format::i64(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I64 (Little Endian)"
        );
        assert_eq!(
            Format::i128(e.clone(), WordOrder::Normal, res(), bf()).to_string(),
            "I128 (Little Endian)"
        );
        assert_eq!(
            Format::f32(e.clone(), WordOrder::Normal, res()).to_string(),
            "F32 (Little Endian)"
        );
        assert_eq!(
            Format::f64(e, WordOrder::Normal, res()).to_string(),
            "F64 (Little Endian)"
        );
    }

    #[test]
    /// MB-R-219 — a format's display text never shows register order, resolution, or bit-field
    /// selector, even when they are non-default.
    fn ut_format_display_never_shows_order_resolution_bitfield() {
        let scaled = Format::u16(
            Endian::Big,
            WordOrder::Reversed,
            Resolution(0.5),
            BitField { mask: 0x0FF0 },
        );
        assert_eq!(scaled.to_string(), "U16 (Big Endian)");
    }

    #[test]
    /// MB-R-010 — the codec supports exactly thirteen data formats: `Ascii`, `U8`..`U128`,
    /// `I8`..`I128`, `F32`, `F64` — ten integer kinds, two float kinds, and `Ascii`.
    fn ut_format_exactly_thirteen_formats() {
        // Exhaustive, no wildcard: an eleventh `IntKind` variant fails to compile here.
        fn int_kind_ordinal(k: IntKind) -> usize {
            match k {
                IntKind::U8 => 0,
                IntKind::U16 => 1,
                IntKind::U32 => 2,
                IntKind::U64 => 3,
                IntKind::U128 => 4,
                IntKind::I8 => 5,
                IntKind::I16 => 6,
                IntKind::I32 => 7,
                IntKind::I64 => 8,
                IntKind::I128 => 9,
            }
        }
        let mut present = [false; 10];
        for k in IntKind::ALL {
            present[int_kind_ordinal(k)] = true;
        }
        assert_eq!(
            present, [true; 10],
            "IntKind::ALL is missing a kind the exhaustive match above knows about"
        );

        // Exhaustive, no wildcard: a third `FloatKind` variant fails to compile here.
        fn float_kind_ordinal(k: FloatKind) -> usize {
            match k {
                FloatKind::F32 => 0,
                FloatKind::F64 => 1,
            }
        }
        let mut present = [false; 2];
        for k in FloatKind::ALL {
            present[float_kind_ordinal(k)] = true;
        }
        assert_eq!(
            present, [true; 2],
            "FloatKind::ALL is missing a kind the exhaustive match above knows about"
        );

        // Exhaustive, no wildcard: a fourth top-level `Format` variant (neither `Numeric`,
        // `Float`, nor `Ascii`) fails to compile here.
        fn format_tag(f: &Format) -> u8 {
            match f {
                Format::Numeric(_) => 0,
                Format::Float(_) => 1,
                Format::Ascii(_, _) => 2,
            }
        }
        let samples = [
            Format::u8(Endian::Big, WordOrder::Normal, res(), bf()),
            Format::f32(Endian::Big, WordOrder::Normal, res()),
            Format::Ascii(Alignment::Left, Width(1)),
        ];
        assert_eq!(
            samples.iter().map(format_tag).collect::<Vec<_>>(),
            [0, 1, 2]
        );

        assert_eq!(IntKind::ALL.len() + FloatKind::ALL.len() + 1, 13);
    }
}
