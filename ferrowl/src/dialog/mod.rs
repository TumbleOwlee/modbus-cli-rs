//! Modal dialogs: module setup and shared register-edit data types.

pub mod ca_file_list;
pub(crate) mod choices;
pub mod close_confirm;
pub mod help;
pub mod lua_help;
pub mod path_suggest;
pub mod rename;
pub mod script_keys;
mod script_manager;
pub mod scripts;
pub mod template_browser;
pub mod tls_section;
pub(crate) mod widgets;

pub use crate::module::modbus::dialog::{EditedRegister, parse_raw_value};
pub use crate::module::modbus::setup_dialog::SetupDialog;
use ferrowl_ui::widgets::{Validate, ValidateResult};

#[derive(Clone, Debug)]
pub struct NonEmpty();

impl Validate for NonEmpty {
    fn validate(input: &str) -> ValidateResult {
        if input.is_empty() {
            ValidateResult::Error("Non-empty input required".to_string())
        } else {
            String::validate(input)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Address();

impl Validate for Address {
    fn validate(input: &str) -> ValidateResult {
        if input == "virtual" {
            ValidateResult::Success
        } else if let ValidateResult::Error(e) = u16::validate(input) {
            ValidateResult::Error(e.to_string())
        } else {
            ValidateResult::None
        }
    }

    fn allowed_char(c: char) -> bool {
        c.is_ascii_digit() || c == '-' || "virtual".contains(c)
    }
}

#[derive(Clone, Debug)]
pub struct Bitmask();

impl Validate for Bitmask {
    fn validate(input: &str) -> ValidateResult {
        if input.is_empty() {
            ValidateResult::None
        } else if let Some(hex) = input
            .strip_prefix("0x")
            .or_else(|| input.strip_prefix("0X"))
        {
            if let Err(e) =
                u128::from_str_radix(hex, 16).map_err(|_| "must be a hex (0x…) or decimal number")
            {
                ValidateResult::Error(e.to_string())
            } else {
                ValidateResult::None
            }
        } else if let Err(e) = input
            .parse::<u128>()
            .map_err(|_| "must be a hex (0x…) or decimal number")
        {
            ValidateResult::Error(e.to_string())
        } else {
            ValidateResult::None
        }
    }

    fn allowed_char(c: char) -> bool {
        c.is_ascii_hexdigit() || matches!(c, 'x' | 'X')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_ui::widgets::ValidateResult;

    #[test]
    /// UI-R-048 — the non-empty field validator rejects empty input.
    fn non_empty_rejects_empty() {
        assert!(matches!(NonEmpty::validate(""), ValidateResult::Error(_)));
    }

    #[test]
    /// UI-R-048 — the non-empty field validator accepts text.
    fn non_empty_accepts_text() {
        assert!(matches!(NonEmpty::validate("hello"), ValidateResult::None));
        assert!(matches!(NonEmpty::validate(" "), ValidateResult::None));
    }

    #[test]
    /// UI-R-048 — the address field validator accepts the `virtual` keyword.
    fn address_virtual_keyword() {
        assert!(matches!(
            Address::validate("virtual"),
            ValidateResult::Success
        ));
    }

    #[test]
    /// MB-R-003 — the address field validator accepts an in-range value.
    fn address_valid_u16() {
        assert!(matches!(Address::validate("0"), ValidateResult::None));
        assert!(matches!(Address::validate("65535"), ValidateResult::None));
        assert!(matches!(Address::validate("32768"), ValidateResult::None));
        assert!(matches!(Address::validate("100"), ValidateResult::None));
    }

    #[test]
    /// MB-R-003 — the address field validator rejects an out-of-range value.
    fn address_overflow_u16() {
        assert!(matches!(
            Address::validate("65536"),
            ValidateResult::Error(_)
        ));
        assert!(matches!(Address::validate("-1"), ValidateResult::Error(_)));
        assert!(matches!(
            Address::validate("99999"),
            ValidateResult::Error(_)
        ));
    }

    #[test]
    /// UI-R-048 — the address field validator rejects non-numeric input.
    fn address_non_numeric() {
        assert!(matches!(Address::validate("abc"), ValidateResult::Error(_)));
        assert!(matches!(Address::validate(""), ValidateResult::Error(_)));
    }

    #[test]
    /// UI-R-048 — the address field's per-character filter admits only valid characters.
    fn address_allowed_char() {
        for c in "virtual1-".chars() {
            assert!(Address::allowed_char(c), "expected {c:?} to be allowed");
        }
        assert!(!Address::allowed_char('z'));
        assert!(!Address::allowed_char(' '));
    }

    #[test]
    /// UI-R-048 — an empty bitmask field validates as none.
    fn bitmask_empty_is_none() {
        assert!(matches!(Bitmask::validate(""), ValidateResult::None));
    }

    #[test]
    /// UI-R-048 — the bitmask validator accepts a lowercase-prefixed hex value.
    fn bitmask_valid_hex_lowercase_prefix() {
        assert!(matches!(Bitmask::validate("0xFF"), ValidateResult::None));
        assert!(matches!(Bitmask::validate("0x0"), ValidateResult::None));
        assert!(matches!(
            Bitmask::validate("0xDEADBEEF"),
            ValidateResult::None
        ));
    }

    #[test]
    /// UI-R-048 — the bitmask validator accepts an uppercase-prefixed hex value.
    fn bitmask_valid_hex_uppercase_prefix() {
        assert!(matches!(Bitmask::validate("0XFF"), ValidateResult::None));
        assert!(matches!(Bitmask::validate("0X0"), ValidateResult::None));
    }

    #[test]
    /// UI-R-048 — the bitmask validator rejects malformed hex.
    fn bitmask_invalid_hex() {
        assert!(matches!(
            Bitmask::validate("0xGG"),
            ValidateResult::Error(_)
        ));
        assert!(matches!(Bitmask::validate("0x"), ValidateResult::Error(_)));
    }

    #[test]
    /// UI-R-048 — the bitmask validator accepts a decimal value.
    fn bitmask_valid_decimal() {
        assert!(matches!(Bitmask::validate("0"), ValidateResult::None));
        assert!(matches!(Bitmask::validate("255"), ValidateResult::None));
        assert!(matches!(
            Bitmask::validate("340282366920938463463374607431768211455"),
            ValidateResult::None
        )); // u128::MAX
    }

    #[test]
    /// UI-R-048 — the bitmask validator rejects malformed decimal.
    fn bitmask_invalid_decimal() {
        assert!(matches!(Bitmask::validate("abc"), ValidateResult::Error(_)));
        assert!(matches!(Bitmask::validate("-1"), ValidateResult::Error(_)));
    }

    #[test]
    /// UI-R-048 — the bitmask field's per-character filter admits only valid characters.
    fn bitmask_allowed_char() {
        for c in ['F', 'x', 'X', '9', 'a'] {
            assert!(Bitmask::allowed_char(c), "expected {c:?} to be allowed");
        }
        assert!(!Bitmask::allowed_char('g'));
        assert!(!Bitmask::allowed_char(' '));
    }
}
