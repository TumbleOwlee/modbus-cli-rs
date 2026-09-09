//! Selection-based register edit dialog: enum-like properties picked from
//! lists instead of typed.

use super::{
    AccessOption, AddNamedValueDialog, Alignment, ConfirmDeleteDialog, Endian, Format, KindOption,
    SubDialogs, ValueType, WordOrder, access_index, alignment_index, endian_index, format_index,
    is_integer_format, is_multi_register_format, kind_index, numeric_parts, parse_address,
    parse_bitmask, set_input, with_numeric_parts, word_order_index,
};
use crate::config::device::{NamedValue, Scalar};
use crate::dialog::EditedRegister;
use crate::dialog::close_confirm::{CloseConfirmDialog, CloseConfirmEvent};
use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use ferrowl_codec::format::{BitField, Format as RegisterFormat, Resolution, Width};
use ferrowl_codec::{Address, Register, RegisterBuilder};
use ferrowl_modbus::UnitId;
use ferrowl_ui::{
    state::{ButtonState, InputFieldState, SelectionState},
    traits::HandleEvents,
    traits::SetFocus,
    traits::ToLabel,
    widgets::{Button, GetValue, InputField, Selection, Text, Validate, ValidateResult, Widget},
};
use ferrowl_ui_derive::{Focus, focusable};
use ratatui::{buffer::Buffer, layout::Rect};
use std::fmt::Debug;

mod build;
mod render;

/// Parse a raw memory string like `[00a0 0001]` into an i64 (big-endian word combination).
pub fn parse_raw_value(raw: &str) -> Option<i64> {
    let inner = raw.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut result: u64 = 0;
    for word in inner.split_whitespace() {
        result = (result << 16) | u64::from_str_radix(word, 16).ok()?;
    }
    Some(result as i64)
}

#[cfg(test)]
mod parse_raw_value_tests {
    use super::parse_raw_value;

    #[test]
    fn ut_parses_single_and_multi_word_big_endian() {
        assert_eq!(parse_raw_value("[0001]"), Some(1));
        assert_eq!(parse_raw_value("[0001 0002]"), Some(0x0001_0002));
        assert_eq!(parse_raw_value("[00a0 0001]"), Some(0x00a0_0001));
        // Empty body folds to zero.
        assert_eq!(parse_raw_value("[]"), Some(0));
        // Surrounding whitespace is tolerated.
        assert_eq!(parse_raw_value("  [00ff]  "), Some(0xff));
    }

    #[test]
    fn ut_rejects_unbracketed_or_non_hex() {
        assert_eq!(parse_raw_value("nope"), None);
        assert_eq!(parse_raw_value("[zz]"), None);
        assert_eq!(parse_raw_value("0001"), None);
    }

    #[test]
    fn ut_inverts_raw_hex_layout() {
        // Mirrors `view::raw_hex`'s `[wwww wwww]` formatting.
        assert_eq!(parse_raw_value("[0000 0000]"), Some(0));
        assert_eq!(parse_raw_value("[ffff ffff]"), Some(0xffff_ffff));
    }
}

#[focusable]
#[derive(Builder, Debug, Focus)]
pub struct EditSelectionDialog<V>
where
    V: ToLabel + Clone,
{
    #[focus]
    pub label: Widget<InputFieldState, InputField<crate::dialog::NonEmpty>>,
    #[focus]
    pub description: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub slave_id: Widget<InputFieldState, InputField<u8>>,
    #[focus]
    pub address: Widget<InputFieldState, InputField<crate::dialog::Address>>,
    #[focus]
    pub kind: Widget<SelectionState<KindOption>, Selection<KindOption>>,
    #[focus]
    pub access: Widget<SelectionState<AccessOption>, Selection<AccessOption>>,
    #[focus]
    pub value_type: Widget<SelectionState<ValueType>, Selection<ValueType>>,
    #[focus(when = {self.value_type.get_value() == ValueType::Number})]
    pub number_format: Widget<SelectionState<Format>, Selection<Format>>,
    #[focus(when = {self.value_type.get_value() == ValueType::Number})]
    pub number_endian: Widget<SelectionState<Endian>, Selection<Endian>>,
    #[focus(when = {self.value_type.get_value() == ValueType::Number && is_multi_register_format(&self.number_format.get_value().0)})]
    pub number_word_order: Widget<SelectionState<WordOrder>, Selection<WordOrder>>,
    #[focus(when = {self.value_type.get_value() == ValueType::Number})]
    pub number_resolution: Widget<InputFieldState, InputField<f64>>,
    #[focus(when = {self.value_type.get_value() == ValueType::Number && is_integer_format(&self.number_format.get_value().0)})]
    pub number_bitmask: Widget<InputFieldState, InputField<crate::dialog::Bitmask>>,
    #[focus(when = {self.value_type.get_value() == ValueType::Text})]
    pub text_alignment: Widget<SelectionState<Alignment>, Selection<Alignment>>,
    #[focus(when = {self.value_type.get_value() == ValueType::Text})]
    pub text_width: Widget<InputFieldState, InputField<usize>>,
    #[focus(when = {!self.value.state.values().is_empty()})]
    pub value: Widget<SelectionState<V>, Selection<V>>,
    #[focus]
    pub add_button: Widget<ButtonState, Button>,
    #[focus(when = {!self.value.state.values().is_empty()})]
    pub delete_button: Widget<ButtonState, Button>,
    // Default value selection (same options as value, plus a leading "no default" sentinel)
    #[focus(when = {!self.value.state.values().is_empty() && (self.access.get_value().0 != ferrowl_codec::Access::ReadOnly || self.is_server) })]
    pub default_value: Widget<SelectionState<V>, Selection<V>>,
    #[focus]
    pub confirm_button: Widget<ButtonState, Button>,
    #[focus(when = { self.deletable })]
    pub delete_register_button: Widget<ButtonState, Button>,
    pub error: Widget<String, Text>,
    pub success: Widget<String, Text>,
    pub keybinds: [Widget<String, Text>; 2],
    #[builder(default)]
    pub add_dialog: Option<AddNamedValueDialog>,
    // Whether this dialog edits an existing register of server (enables the value input).
    #[builder(default)]
    pub is_server: bool,
    // Whether this dialog edits an existing register (enables the delete button).
    #[builder(default)]
    pub deletable: bool,
    #[builder(default)]
    pub confirm_delete: Option<ConfirmDeleteDialog>,
    // Name-conflict error set by the app at confirm time. Survives the per-frame `validate()`
    // refresh (which can't see other registers) until the user edits the dialog again.
    #[builder(default)]
    pub name_error: Option<String>,
    // Confirm-close popup, opened with Esc.
    #[builder(default)]
    pub close_confirm: Option<CloseConfirmDialog>,
}

impl<V: ToLabel + Clone> EditSelectionDialog<V> {
    fn validate(&self) -> Result<(), String> {
        if let ValidateResult::Error(e) = String::validate(self.label.state.input()) {
            return Err(format!("Label: {e}"));
        } else if let ValidateResult::Error(e) = u8::validate(self.slave_id.state.input()) {
            return Err(format!("Slave ID: {e}"));
        } else if let Err(e) = parse_address(self.address.state.input()) {
            return Err(format!("Address: {e}"));
        }

        match self.value_type.state.values()[self.value_type.state.selection()] {
            ValueType::Number => {
                if let ValidateResult::Error(e) =
                    f64::validate(self.number_resolution.state.input())
                {
                    return Err(format!("Resolution: {e}"));
                }
                let format =
                    &self.number_format.state.values()[self.number_format.state.selection()].0;
                if is_integer_format(format)
                    && let Err(e) = parse_bitmask(self.number_bitmask.state.input())
                {
                    return Err(format!("Bitmask: {e}"));
                }
            }
            ValueType::Text => {
                if let ValidateResult::Error(e) = usize::validate(self.text_width.state.input()) {
                    return Err(format!("Width: {e}"));
                }
            }
        }
        Ok(())
    }
}

impl EditSelectionDialog<NamedValue> {
    /// Build the dialog pre-filled from an existing register, its named values, and current value.
    /// `raw_value` is the hex memory string (e.g. `[000a]`) used for accurate integer matching.
    #[allow(clippy::too_many_arguments)]
    pub fn from_register(
        name: &str,
        description: &str,
        register: &Register,
        named_values: Vec<NamedValue>,
        current_value: &str,
        raw_value: &str,
        default: Option<&Scalar>,
        is_server: bool,
    ) -> Self {
        let mut dialog = Self::new(named_values.clone());
        dialog.deletable = true;
        dialog.is_server = is_server;
        set_input(&mut dialog.label, name);
        set_input(&mut dialog.description, description);
        // Populate default selection: sentinel at index 0, then all named values.
        let mut default_vals = vec![NamedValue {
            name: "(no default)".to_string(),
            value: Scalar::Text("".into()),
        }];
        default_vals.extend_from_slice(&named_values);
        *dialog.default_value.state.values_mut() = default_vals;
        if let Some(def) = default {
            let def_str = def.to_string();
            if let Some(idx) = named_values
                .iter()
                .position(|nv| nv.value.to_string() == def_str)
            {
                dialog.default_value.state.set_selection(idx + 1);
            }
        }
        dialog.label.state.set_focused(false);
        dialog.value.state.set_focused(true);
        dialog.focus = EditSelectionDialogFocus::Value;
        match register.address() {
            Address::Fixed(addr) => set_input(&mut dialog.address, &addr.to_string()),
            Address::Virtual => set_input(&mut dialog.address, "virtual"),
        }
        set_input(&mut dialog.slave_id, &register.slave_id().to_string());
        dialog
            .access
            .state
            .set_selection(access_index(register.access()));
        dialog.kind.state.set_selection(kind_index(register.kind()));

        match register.format() {
            RegisterFormat::Ascii(align, width) => {
                dialog.value_type.state.set_selection(1);
                dialog
                    .text_alignment
                    .state
                    .set_selection(alignment_index(align));
                set_input(&mut dialog.text_width, &width.0.to_string());
            }
            numeric => {
                let (endian, word_order, resolution, bitfield) = numeric_parts(numeric);
                dialog.value_type.state.set_selection(0);
                dialog
                    .number_format
                    .state
                    .set_selection(format_index(numeric));
                dialog
                    .number_endian
                    .state
                    .set_selection(endian_index(&endian));
                dialog
                    .number_word_order
                    .state
                    .set_selection(word_order_index(&word_order));
                set_input(&mut dialog.number_resolution, &resolution.0.to_string());
                // Show the mask only when it actually selects a sub-field.
                if !bitfield.is_full() {
                    set_input(
                        &mut dialog.number_bitmask,
                        &format!("0x{:X}", bitfield.mask),
                    );
                }
            }
        }

        // Pre-select the matching named value. Integer values match the raw memory words (reliable
        // across formats/resolutions); any value type also matches the decoded display string.
        let raw_int = parse_raw_value(raw_value);
        let current = current_value.trim();
        if let Some(idx) = named_values.iter().position(|nv| match &nv.value {
            Scalar::Int(v) => raw_int == Some(*v) || current == v.to_string(),
            other => current == other.to_string(),
        }) {
            dialog.value.state.set_selection(idx);
        }

        dialog
    }

    /// Validate and produce the edited register metadata + the selected named value to write.
    pub fn apply(&self) -> Result<EditedRegister, String> {
        self.validate()?;
        let name = self.label.state.input().trim().to_string();
        let description = self.description.state.input().trim().to_string();
        let address = parse_address(self.address.state.input())?;

        let format = match self.value_type.state.get_value() {
            ValueType::Number => {
                let selected = self.number_format.state.get_value();
                let endian = self.number_endian.state.get_value().0;
                let word_order = self.number_word_order.state.get_value().0;
                let resolution = Resolution(
                    self.number_resolution
                        .state
                        .input()
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| "Resolution must be a number.".to_string())?,
                );
                // Bitmask applies to integer formats only; floats ignore it.
                let bitfield = if is_integer_format(&selected.0) {
                    parse_bitmask(self.number_bitmask.state.input())
                        .map_err(|e| format!("Bitmask {e}."))?
                } else {
                    BitField::default()
                };
                with_numeric_parts(&selected.0, endian, word_order, resolution, bitfield)
            }
            ValueType::Text => {
                let alignment = self.text_alignment.state.get_value().0;
                let width = self
                    .text_width
                    .state
                    .input()
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "Width must be a number.".to_string())?;
                RegisterFormat::Ascii(alignment, Width(width))
            }
        };

        let slave_id = self
            .slave_id
            .state
            .input()
            .trim()
            .parse::<u8>()
            .map_err(|_| "Slave ID must be 0–255.".to_string())?;

        let register = RegisterBuilder::default()
            .slave_id(UnitId(slave_id))
            .access(self.access.state.get_value().0.clone())
            .kind(self.kind.state.get_value().0)
            .address(address)
            .format(format)
            .build()
            .expect("all register fields are set");

        let named_values = self.value.state.values().clone();
        let value = if named_values.is_empty() {
            None
        } else {
            Some(self.value.state.get_value().value.to_string())
        };
        let default = {
            let sel = self.default_value.state.selection();
            let vals = self.default_value.state.values();
            if sel == 0 || vals.len() <= 1 {
                None
            } else {
                Some(vals[sel].value.clone())
            }
        };

        Ok(EditedRegister {
            name,
            description,
            register,
            value,
            named_values: Some(named_values),
            default,
        })
    }

    pub fn handle_space(&mut self) {
        match self.focus {
            EditSelectionDialogFocus::AddButton => self.open_add_dialog(),
            EditSelectionDialogFocus::DeleteButton => self.delete_selected(),
            EditSelectionDialogFocus::DeleteRegisterButton => self.open_confirm_delete(),
            _ => {
                self.handle_events(KeyModifiers::NONE, KeyCode::Char(' '));
            }
        }
    }

    pub fn is_delete_register_button_focused(&self) -> bool {
        matches!(self.focus, EditSelectionDialogFocus::DeleteRegisterButton)
    }

    pub fn is_confirm_button_focused(&self) -> bool {
        matches!(self.focus, EditSelectionDialogFocus::ConfirmButton)
    }

    /// Convert this dialog into an EditInputDialog, preserving all shared field state.
    /// Called when all named values are removed and the dialog should switch to free-text mode.
    pub fn to_edit_input_dialog(&self) -> super::input::EditInputDialog {
        let mut d = super::input::EditInputDialog::new();
        d.deletable = self.deletable;
        d.is_server = self.is_server;
        d.label.state = self.label.state.clone();
        d.description.state = self.description.state.clone();
        d.slave_id.state = self.slave_id.state.clone();
        d.address.state = self.address.state.clone();
        d.kind.state = self.kind.state.clone();
        d.access.state = self.access.state.clone();
        d.value_type.state = self.value_type.state.clone();
        d.number_format.state = self.number_format.state.clone();
        d.number_endian.state = self.number_endian.state.clone();
        d.number_word_order.state = self.number_word_order.state.clone();
        d.number_resolution.state = self.number_resolution.state.clone();
        d.number_bitmask.state = self.number_bitmask.state.clone();
        d.text_alignment.state = self.text_alignment.state.clone();
        d.text_width.state = self.text_width.state.clone();
        // Index 0 is the "(no default)" sentinel; skip it.
        let sel = self.default_value.state.selection();
        if sel > 0
            && let Some(nv) = self.default_value.state.values().get(sel)
        {
            set_input(&mut d.default_value, &nv.value.to_string());
        }
        d
    }

    pub fn delete_selected(&mut self) {
        let idx = self.value.state.selection();
        let vals = self.value.state.values_mut();
        let mut is_empty = vals.is_empty();
        if !vals.is_empty() {
            vals.remove(idx);
            is_empty = vals.is_empty();
            if !vals.is_empty() {
                let new_idx = if idx >= vals.len() {
                    vals.len() - 1
                } else {
                    idx
                };
                self.value.state.set_selection(new_idx);
            } else {
                self.value.state.set_selection(0);
            }

            // Sync default selection: idx+1 because sentinel sits at position 0.
            let default_idx = idx + 1;
            let default_vals = self.default_value.state.values_mut();
            if default_idx < default_vals.len() {
                default_vals.remove(default_idx);
                let default_sel = self.default_value.state.selection();
                if default_sel >= default_idx {
                    // If exactly the deleted entry was selected, reset to "no default";
                    // otherwise shift the selection down to stay on the same item.
                    let new_sel = if default_sel == default_idx {
                        0
                    } else {
                        default_sel - 1
                    };
                    self.default_value.state.set_selection(new_sel);
                }
            }
        }

        if is_empty {
            self.focus_previous();
        }
    }
}

impl SubDialogs for EditSelectionDialog<NamedValue> {
    fn add_dialog_opt(&self) -> Option<&AddNamedValueDialog> {
        self.add_dialog.as_ref()
    }

    fn add_dialog_slot(&mut self) -> &mut Option<AddNamedValueDialog> {
        &mut self.add_dialog
    }

    fn confirm_delete_opt(&self) -> Option<&ConfirmDeleteDialog> {
        self.confirm_delete.as_ref()
    }

    fn confirm_delete_slot(&mut self) -> &mut Option<ConfirmDeleteDialog> {
        &mut self.confirm_delete
    }

    fn name_error_slot(&mut self) -> &mut Option<String> {
        &mut self.name_error
    }

    fn register_label(&self) -> String {
        self.label.state.input().trim().to_string()
    }

    fn accept_named_value(&mut self, nv: NamedValue) {
        self.value.state.values_mut().push(nv.clone());
        let idx = self.value.state.values().len() - 1;
        self.value.state.set_selection(idx);
        // Keep default selection in sync: append after the sentinel.
        self.default_value.state.values_mut().push(nv);
    }
}

#[cfg(test)]
mod apply_tests {
    //! Characterization tests for the `from_register` → `apply` round-trip and `delete_selected`,
    //! pinning current behavior before the mod.rs split / unwrap sweep.
    use super::EditSelectionDialog;
    use crate::config::device::{NamedValue, Scalar};
    use ferrowl_codec::format::{
        BitField, Endian as RegisterEndian, Format as RegisterFormat, Resolution,
        WordOrder as RegisterWordOrder,
    };
    use ferrowl_codec::{Access, Address, Kind, Register, RegisterBuilder};
    use ferrowl_modbus::UnitId;

    fn reg(kind: Kind, address: Address, format: RegisterFormat) -> Register {
        RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(kind)
            .address(address)
            .format(format)
            .build()
            .unwrap()
    }

    fn named_values() -> Vec<NamedValue> {
        vec![
            NamedValue {
                name: "on".into(),
                value: Scalar::Int(1),
            },
            NamedValue {
                name: "off".into(),
                value: Scalar::Int(0),
            },
        ]
    }

    #[test]
    fn ut_round_trips_through_apply() {
        let original = reg(
            Kind::HoldingRegister,
            Address::Fixed(10),
            RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ),
        );
        let edited = EditSelectionDialog::from_register(
            "state",
            "desc",
            &original,
            named_values(),
            "1",
            "[0001]",
            None,
            true,
        )
        .apply()
        .expect("valid dialog should apply");

        assert_eq!(edited.name, "state");
        assert_eq!(edited.description, "desc");
        assert_eq!(*edited.register.address(), Address::Fixed(10));
        assert_eq!(edited.value.as_deref(), Some("1"));
    }

    #[test]
    fn ut_from_register_preselects_matching_named_value() {
        let original = reg(
            Kind::HoldingRegister,
            Address::Fixed(0),
            RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ),
        );
        let dialog = EditSelectionDialog::from_register(
            "s",
            "",
            &original,
            named_values(),
            "0",
            "[0000]",
            None,
            true,
        );
        assert_eq!(dialog.value.state.selection(), 1); // "off" (value 0)
    }

    #[test]
    fn ut_delete_selected_removes_entry_and_syncs_default() {
        let original = reg(
            Kind::HoldingRegister,
            Address::Fixed(0),
            RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ),
        );
        let mut dialog = EditSelectionDialog::from_register(
            "s",
            "",
            &original,
            named_values(),
            "1",
            "[0001]",
            None,
            true,
        );
        assert_eq!(dialog.value.state.values().len(), 2);
        dialog.value.state.set_selection(0);
        dialog.delete_selected();
        assert_eq!(dialog.value.state.values().len(), 1);
        assert_eq!(dialog.value.state.values()[0].name, "off");
    }

    #[test]
    fn ut_empty_dialog_fails_validation() {
        assert!(
            EditSelectionDialog::<NamedValue>::new(vec![])
                .apply()
                .is_err()
        );
    }
}

#[cfg(test)]
mod focus_tests {
    //! Focus-cycle characterization, mirroring `EditInputDialog`'s coverage.
    use super::EditSelectionDialog;
    use crate::config::device::{NamedValue, Scalar};
    use ferrowl_codec::format::{
        BitField, Endian as RegisterEndian, Format as RegisterFormat, Resolution,
        WordOrder as RegisterWordOrder,
    };
    use ferrowl_codec::{Access, Address, Kind, Register, RegisterBuilder};
    use ferrowl_modbus::UnitId;

    fn dialog_with_values() -> EditSelectionDialog<NamedValue> {
        let original: Register = RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Fixed(0))
            .format(RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap();
        let values = vec![NamedValue {
            name: "on".into(),
            value: Scalar::Int(1),
        }];
        EditSelectionDialog::from_register("s", "", &original, values, "1", "[0001]", None, true)
    }

    #[test]
    fn ut_focus_cycle_wraps_back_to_start() {
        let mut d = dialog_with_values();
        let start = d.focus;
        for _ in 0..64 {
            d.focus_next();
            if d.focus == start {
                return;
            }
        }
        panic!("focus_next did not return to the starting pane within 64 steps");
    }

    #[test]
    fn ut_focus_previous_reverses_focus_next() {
        let mut d = dialog_with_values();
        let start = d.focus;
        d.focus_next();
        assert_ne!(d.focus, start);
        d.focus_previous();
        assert_eq!(d.focus, start);
    }
}

#[cfg(test)]
mod default_and_conversion_tests {
    //! Characterization of the default-value list sync, named-value append, mode conversion
    //! (`to_edit_input_dialog`), and gated focus panes on an empty-values dialog.
    use super::{EditSelectionDialog, EditSelectionDialogFocus};
    use crate::config::device::{NamedValue, Scalar};
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_codec::format::{
        BitField, Endian, Format, Resolution, WordOrder as RegisterWordOrder,
    };
    use ferrowl_codec::{Access, Address, Kind, Register, RegisterBuilder};
    use ferrowl_modbus::UnitId;
    use ferrowl_ui::traits::HandleEvents;

    fn u16_register() -> Register {
        RegisterBuilder::default()
            .slave_id(UnitId(3))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Fixed(7))
            .format(Format::u16(
                Endian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap()
    }

    fn named_values() -> Vec<NamedValue> {
        vec![
            NamedValue {
                name: "off".into(),
                value: Scalar::Int(0),
            },
            NamedValue {
                name: "on".into(),
                value: Scalar::Int(1),
            },
        ]
    }

    /// Dialog editing a register whose current raw value matches "on", with "on" as default.
    fn dialog() -> EditSelectionDialog<NamedValue> {
        EditSelectionDialog::from_register(
            "state",
            "power state",
            &u16_register(),
            named_values(),
            "1",
            "[0001]",
            Some(&Scalar::Int(1)),
            true,
        )
    }

    #[test]
    fn ut_from_register_preselects_default_after_sentinel() {
        let d = dialog();
        // Default list gains the "(no default)" sentinel at 0; Scalar::Int(1) sits at 1 + 1.
        assert_eq!(d.default_value.state.values().len(), 3);
        assert_eq!(d.default_value.state.values()[0].name, "(no default)");
        assert_eq!(d.default_value.state.selection(), 2);
    }

    #[test]
    fn ut_apply_carries_selected_default() {
        let edited = dialog().apply().expect("valid dialog should apply");
        assert!(matches!(edited.default, Some(Scalar::Int(1))));
        // And the named-value list survives unchanged.
        assert_eq!(edited.named_values.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn ut_delete_selected_shifts_default_selection_down() {
        let mut d = dialog();
        d.value.state.set_selection(0); // delete "off"; default "on" must stay selected
        d.delete_selected();
        assert_eq!(d.default_value.state.values().len(), 2);
        assert_eq!(d.default_value.state.selection(), 1);
    }

    #[test]
    fn ut_delete_selected_default_resets_to_sentinel_when_its_entry_dies() {
        let mut d = dialog();
        d.value.state.set_selection(1); // "on", which is also the default
        d.delete_selected();
        assert_eq!(d.value.state.values().len(), 1);
        // The deleted entry was the default -> back to "(no default)".
        assert_eq!(d.default_value.state.selection(), 0);
    }

    #[test]
    fn ut_accept_named_value_appends_selects_and_syncs_default_options() {
        use super::SubDialogs;
        let mut d = dialog();
        d.accept_named_value(NamedValue {
            name: "auto".into(),
            value: Scalar::Int(2),
        });
        assert_eq!(d.value.state.values().len(), 3);
        assert_eq!(d.value.state.selection(), 2);
        // Default options track the value list (sentinel + 3).
        assert_eq!(d.default_value.state.values().len(), 4);
    }

    #[test]
    fn ut_to_edit_input_dialog_carries_state_and_default_text() {
        let d = dialog();
        let input = d.to_edit_input_dialog();
        assert_eq!(input.label.state.input(), "state");
        assert_eq!(input.slave_id.state.input(), "3");
        assert_eq!(input.address.state.input(), "7");
        assert!(input.deletable);
        // The selected default (Int 1) becomes free text.
        assert_eq!(input.default_value.state.input(), "1");
    }

    #[test]
    fn ut_focus_cycle_skips_value_panes_when_no_named_values() {
        let mut d = EditSelectionDialog::<NamedValue>::new(vec![]);
        // `new()` always starts focus on `Value`, even though it's gated off (not focusable)
        // when there are no named values yet; one `focus_next()` lands on the first real pane.
        d.focus_next();
        let start = d.focus;
        let mut seen = vec![start];
        for _ in 0..64 {
            d.focus_next();
            if d.focus == start {
                break;
            }
            seen.push(d.focus);
        }
        assert_eq!(d.focus, start, "focus_next never wrapped: {seen:?}");
        for gated in [
            EditSelectionDialogFocus::Value,
            EditSelectionDialogFocus::DeleteButton,
            EditSelectionDialogFocus::DefaultValue,
        ] {
            assert!(
                !seen.contains(&gated),
                "empty-values cycle should skip {gated:?}: {seen:?}"
            );
        }
        assert!(seen.contains(&EditSelectionDialogFocus::ConfirmButton));
    }

    #[test]
    fn ut_handle_events_routes_key_to_focused_selection() {
        let mut d = dialog();
        assert!(matches!(d.focus, EditSelectionDialogFocus::Value));
        assert_eq!(d.value.state.selection(), 1);
        // Up/Down cycle the focused selection widget's item (Left/Right instead scroll a long
        // label horizontally, they don't change `selection`).
        let _ = HandleEvents::handle_events(&mut d, KeyModifiers::NONE, KeyCode::Up);
        assert_eq!(d.value.state.selection(), 0);
        let _ = HandleEvents::handle_events(&mut d, KeyModifiers::NONE, KeyCode::Down);
        assert_eq!(d.value.state.selection(), 1);
    }
}

impl super::RegisterDialog for EditSelectionDialog<NamedValue> {
    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.render(area, buf)
    }
    fn focus_next(&mut self) {
        self.focus_next()
    }
    fn focus_previous(&mut self) {
        self.focus_previous()
    }
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) {
        let _ = HandleEvents::handle_events(self, modifiers, code);
    }
    fn handle_space(&mut self) {
        self.handle_space()
    }
    fn is_confirm_button_focused(&self) -> bool {
        self.is_confirm_button_focused()
    }
    fn is_delete_register_button_focused(&self) -> bool {
        self.is_delete_register_button_focused()
    }
    fn apply(&self) -> Result<EditedRegister, String> {
        self.apply()
    }
    fn close_confirm_is_active(&self) -> bool {
        self.close_confirm.is_some()
    }
    fn close_confirm_open(&mut self) {
        self.close_confirm = Some(CloseConfirmDialog::new());
    }
    fn close_confirm_handle_key(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> CloseConfirmEvent {
        let Some(confirm) = self.close_confirm.as_mut() else {
            return CloseConfirmEvent::Dismiss;
        };
        let event = confirm.handle_key(modifiers, code);
        if !matches!(event, CloseConfirmEvent::Consumed) {
            self.close_confirm = None;
        }
        event
    }
}
