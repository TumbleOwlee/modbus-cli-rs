//! Free-text register edit dialog: every register property as an input field.

use super::{
    AccessOption, Alignment, Endian, Format, KindOption, ValueType, WordOrder, parse_address,
};
use crate::config::device::{NamedValue, Scalar};
use crate::dialog::NonEmpty;
use crate::dialog::close_confirm::{CloseConfirmDialog, CloseConfirmEvent};
use derive_builder::Builder;
use ferrowl_codec::format::{
    BitField, Endian as RegisterEndian, Format as RegisterFormat, Resolution, Width,
    WordOrder as RegisterWordOrder,
};
use ferrowl_codec::{Address, Kind, Register, RegisterBuilder, encode};
use ferrowl_modbus::UnitId;
use ferrowl_ui::{
    state::{ButtonState, InputFieldState, SelectionState},
    traits::SetFocus,
    widgets::{Button, GetValue, InputField, Selection, Text, Validate, ValidateResult, Widget},
};
use ferrowl_ui_derive::{Focus, focusable};
use ratatui::{buffer::Buffer, layout::Rect};
use std::fmt::Debug;

mod build;
mod render;

#[focusable]
#[derive(Builder, Debug, Focus)]
pub struct EditInputDialog {
    #[focus]
    pub label: Widget<InputFieldState, InputField<NonEmpty>>,
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
    #[focus(when = { !self.is_boolean_kind() })]
    pub value_type: Widget<SelectionState<ValueType>, Selection<ValueType>>,
    // Static "Boolean" label shown instead of Type selector for Coil/DiscreteInput
    pub boolean_type: Widget<String, Text>,
    #[focus(when = { !self.is_boolean_kind() && self.value_type.get_value() == ValueType::Number })]
    pub number_format: Widget<SelectionState<Format>, Selection<Format>>,
    #[focus(when = { !self.is_boolean_kind() && self.value_type.get_value() == ValueType::Number })]
    pub number_endian: Widget<SelectionState<Endian>, Selection<Endian>>,
    #[focus(when = { !self.is_boolean_kind() && self.value_type.get_value() == ValueType::Number && is_multi_register_format(&self.number_format.get_value().0) })]
    pub number_word_order: Widget<SelectionState<WordOrder>, Selection<WordOrder>>,
    #[focus(when = { !self.is_boolean_kind() && self.value_type.get_value() == ValueType::Number })]
    pub number_resolution: Widget<InputFieldState, InputField<f64>>,
    #[focus(when = { !self.is_boolean_kind() && self.value_type.get_value() == ValueType::Number && is_integer_format(&self.number_format.get_value().0) })]
    pub number_bitmask: Widget<InputFieldState, InputField<crate::dialog::Bitmask>>,
    #[focus(when = { !self.is_boolean_kind() && self.value_type.get_value() == ValueType::Text })]
    pub text_alignment: Widget<SelectionState<Alignment>, Selection<Alignment>>,
    #[focus(when = { !self.is_boolean_kind() && self.value_type.get_value() == ValueType::Text })]
    pub text_width: Widget<InputFieldState, InputField<usize>>,
    #[focus(when = {self.access.get_value().0 != ferrowl_codec::Access::ReadOnly || self.is_server })]
    pub value: Widget<InputFieldState, InputField<String>>,
    // Default value stored in the device config and applied on startup
    #[focus(when = {self.access.get_value().0 != ferrowl_codec::Access::ReadOnly || self.is_server })]
    pub default_value: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub add_button: Widget<ButtonState, Button>,
    #[focus]
    pub confirm_button: Widget<ButtonState, Button>,
    #[focus(when = { self.deletable })]
    pub delete_register_button: Widget<ButtonState, Button>,
    pub error: Widget<String, Text>,
    pub success: Widget<String, Text>,
    pub keybinds: [Widget<String, Text>; 2],
    #[builder(default)]
    pub add_dialog: Option<AddNamedValueDialog>,
    // Named values accumulated via the ADD button in this session.
    #[builder(default)]
    pub pending_named_values: Vec<NamedValue>,
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

/// The result of confirming the edit dialog: updated register metadata + an optional value to
/// write.
#[derive(Debug, Clone)]
pub struct EditedRegister {
    pub name: String,
    pub description: String,
    pub register: Register,
    pub value: Option<String>,
    /// Updated named-value list from EditSelectionDialog; None means unchanged.
    pub named_values: Option<Vec<crate::config::device::NamedValue>>,
    /// Default value to store in the device config (applied on startup). None = no default.
    pub default: Option<Scalar>,
}

impl EditInputDialog {
    fn is_boolean_kind(&self) -> bool {
        matches!(
            self.kind.state.get_value().0,
            Kind::Coil | Kind::DiscreteInput
        )
    }

    fn validate(&self) -> Result<(), String> {
        if let ValidateResult::Error(e) = String::validate(self.label.state.input()) {
            return Err(format!("Label: {e}"));
        } else if let ValidateResult::Error(e) = u8::validate(self.slave_id.state.input()) {
            return Err(format!("Slave ID: {e}"));
        } else if let Err(e) = parse_address(self.address.state.input()) {
            return Err(format!("Address: {e}"));
        }

        if !self.is_boolean_kind() {
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
                    let v = self.value.state.input();
                    let s = v.trim();
                    if let Err(e) = encode(format, s) {
                        return Err(format!("Value: cannot convert '{s}' to number [{e}]"));
                    }
                    let v = self.default_value.state.input();
                    let s = v.trim();
                    if !s.is_empty()
                        && let Err(e) = encode(format, s)
                    {
                        return Err(format!("Value: cannot convert '{s}' to number [{e}]"));
                    }
                }
                ValueType::Text => {
                    if let ValidateResult::Error(e) = usize::validate(self.text_width.state.input())
                    {
                        return Err(format!("Width: {e}"));
                    }
                }
            }
        }
        Ok(())
    }
    /// Build the dialog pre-filled from an existing register and its current value. Focus
    /// starts on the value field so editing the value (the common case) works immediately.
    pub fn from_register(
        name: &str,
        description: &str,
        register: &Register,
        value: &str,
        default: Option<&Scalar>,
        is_server: bool,
    ) -> Self {
        let mut dialog = Self::new();
        dialog.deletable = true;
        dialog.is_server = is_server;
        set_input(&mut dialog.label, name);
        set_input(&mut dialog.description, description);
        if let Some(def) = default {
            set_input(&mut dialog.default_value, &def.to_string());
        }
        // Pre-populate the value field so the user can edit or clear it directly.
        let is_ascii = matches!(register.format(), RegisterFormat::Ascii(_, _));
        if is_ascii {
            let value: String = if matches!(
                register.format(),
                RegisterFormat::Ascii(ferrowl_codec::Alignment::Left, _)
            ) {
                let value: String = value
                    .chars()
                    .rev()
                    .skip_while(|c| !c.is_ascii_graphic())
                    .map(|c| if !c.is_ascii_graphic() { ' ' } else { c })
                    .collect();
                value.chars().rev().collect()
            } else {
                value
                    .chars()
                    .skip_while(|c| !c.is_ascii_graphic())
                    .map(|c| if !c.is_ascii_graphic() { ' ' } else { c })
                    .collect()
            };
            set_input(&mut dialog.value, &value);
        } else {
            set_input(&mut dialog.value, value);
        }
        dialog.label.state.set_focused(false);
        dialog.value.state.set_focused(true);
        dialog.focus = EditInputDialogFocus::Value;
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
        dialog
    }

    /// Validate and produce the edited register metadata + optional value to write.
    pub fn apply(&self) -> Result<EditedRegister, String> {
        self.validate()?;
        let name = self.label.state.input().trim().to_string();
        let description = self.description.state.input().trim().to_string();
        let address = parse_address(self.address.state.input())?;

        let format = if self.is_boolean_kind() {
            RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            )
        } else {
            match self.value_type.state.get_value() {
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
            }
        };
        let is_ascii = matches!(format, RegisterFormat::Ascii(_, _));

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

        let input = self.value.state.input().to_string();
        let value = if is_ascii || !input.trim().is_empty() {
            Some(input)
        } else {
            None
        };
        let named_values = if self.pending_named_values.is_empty() {
            None
        } else {
            Some(self.pending_named_values.clone())
        };

        let default = {
            let s = self.default_value.state.input().trim();
            if s.is_empty() {
                None
            } else {
                Some(Scalar::from_input(s))
            }
        };

        Ok(EditedRegister {
            name,
            description,
            register,
            value,
            named_values,
            default,
        })
    }

    pub fn handle_space(&mut self) {
        match self.focus {
            EditInputDialogFocus::AddButton => self.open_add_dialog(),
            EditInputDialogFocus::DeleteRegisterButton => self.open_confirm_delete(),
            _ => {
                self.handle_events(KeyModifiers::NONE, KeyCode::Char(' '));
            }
        }
    }

    pub fn is_delete_register_button_focused(&self) -> bool {
        matches!(self.focus, EditInputDialogFocus::DeleteRegisterButton)
    }

    pub fn is_confirm_button_focused(&self) -> bool {
        matches!(self.focus, EditInputDialogFocus::ConfirmButton)
    }

    /// Convert this dialog into an EditSelectionDialog, preserving shared field state.
    /// Called when the first named value is added and the dialog should switch to selection mode.
    pub fn to_edit_selection_dialog(
        &self,
    ) -> super::selection::EditSelectionDialog<crate::config::device::NamedValue> {
        use crate::config::device::{NamedValue, Scalar};
        let values = self.pending_named_values.clone();
        let mut d = super::selection::EditSelectionDialog::new(values.clone());
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

        // Index 0 is the "(no default)" sentinel.
        let mut default_vals = vec![NamedValue {
            name: "(no default)".to_string(),
            value: Scalar::Text("".into()),
        }];
        default_vals.extend_from_slice(&values);
        *d.default_value.state.values_mut() = default_vals;
        let default_text = self.default_value.state.input().trim().to_string();
        if !default_text.is_empty()
            && let Some(idx) = values
                .iter()
                .position(|nv| nv.value.to_string() == default_text)
        {
            d.default_value.state.set_selection(idx + 1);
        }
        d
    }
}

impl SubDialogs for EditInputDialog {
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
        self.pending_named_values.push(nv);
    }
}

impl super::RegisterDialog for EditInputDialog {
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

use super::{
    AddNamedValueDialog, ConfirmDeleteDialog, SubDialogs, access_index, alignment_index,
    endian_index, format_index, is_integer_format, is_multi_register_format, kind_index,
    numeric_parts, parse_bitmask, set_input, with_numeric_parts, word_order_index,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::traits::HandleEvents;

#[cfg(test)]
mod apply_tests {
    //! Characterization tests for the `from_register` → `apply` round-trip: editing an existing
    //! register and confirming must reproduce its metadata.
    use super::EditInputDialog;
    use ferrowl_codec::format::{
        Alignment as TextAlignment, BitField, Endian as RegisterEndian, Format as RegisterFormat,
        Resolution, Width, WordOrder as RegisterWordOrder,
    };
    use ferrowl_codec::{Access, Address, Kind, Register, RegisterBuilder};
    use ferrowl_modbus::UnitId;

    fn reg(
        kind: Kind,
        access: Access,
        address: Address,
        slave: u8,
        format: RegisterFormat,
    ) -> Register {
        RegisterBuilder::default()
            .slave_id(UnitId(slave))
            .access(access)
            .kind(kind)
            .address(address)
            .format(format)
            .build()
            .unwrap()
    }

    #[test]
    fn ut_numeric_register_round_trips_through_apply() {
        let original = reg(
            Kind::HoldingRegister,
            Access::ReadWrite,
            Address::Fixed(100),
            7,
            RegisterFormat::u32(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ),
        );
        let edited =
            EditInputDialog::from_register("temp", "a sensor", &original, "42", None, true)
                .apply()
                .expect("valid register should apply");

        assert_eq!(edited.name, "temp");
        assert_eq!(edited.description, "a sensor");
        assert_eq!(*edited.register.slave_id(), UnitId(7));
        assert_eq!(*edited.register.kind(), Kind::HoldingRegister);
        assert_eq!(*edited.register.access(), Access::ReadWrite);
        assert_eq!(*edited.register.address(), Address::Fixed(100));
        assert_eq!(edited.register.format(), original.format());
        assert_eq!(edited.value.as_deref(), Some("42"));
    }

    #[test]
    /// MB-R-099 — a reversed register order is seeded on open and preserved through apply.
    fn ut_reversed_word_order_round_trips_through_apply() {
        let original = reg(
            Kind::HoldingRegister,
            Access::ReadWrite,
            Address::Fixed(10),
            1,
            RegisterFormat::u32(
                RegisterEndian::Big,
                RegisterWordOrder::Reversed,
                Resolution(1.0),
                BitField::default(),
            ),
        );
        let edited = EditInputDialog::from_register("w", "", &original, "1", None, true)
            .apply()
            .expect("valid register should apply");
        assert_eq!(edited.register.format(), original.format());
    }

    #[test]
    fn ut_virtual_address_and_read_only_round_trip() {
        let original = reg(
            Kind::InputRegister,
            Access::ReadOnly,
            Address::Virtual,
            1,
            RegisterFormat::u16(
                RegisterEndian::Little,
                RegisterWordOrder::Normal,
                Resolution(0.5),
                BitField::default(),
            ),
        );
        let edited = EditInputDialog::from_register("v", "", &original, "3", None, true)
            .apply()
            .expect("valid register should apply");

        assert_eq!(*edited.register.address(), Address::Virtual);
        assert_eq!(*edited.register.access(), Access::ReadOnly);
        assert_eq!(edited.register.format(), original.format());
    }

    #[test]
    fn ut_non_full_bitmask_round_trips() {
        let original = reg(
            Kind::HoldingRegister,
            Access::ReadWrite,
            Address::Fixed(5),
            1,
            RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField { mask: 0xFF00 },
            ),
        );
        let edited = EditInputDialog::from_register("masked", "", &original, "0", None, true)
            .apply()
            .expect("valid register should apply");
        assert_eq!(edited.register.format(), original.format());
    }

    #[test]
    fn ut_ascii_register_round_trips_format() {
        let original = reg(
            Kind::HoldingRegister,
            Access::ReadWrite,
            Address::Fixed(0),
            1,
            RegisterFormat::Ascii(TextAlignment::Left, Width(4)),
        );
        let edited = EditInputDialog::from_register("label", "", &original, "AB", None, true)
            .apply()
            .expect("valid register should apply");
        assert_eq!(edited.register.format(), original.format());
    }

    #[test]
    fn ut_boolean_kind_forces_default_u16_format() {
        let original = reg(
            Kind::Coil,
            Access::ReadWrite,
            Address::Fixed(1),
            1,
            RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ),
        );
        let edited = EditInputDialog::from_register("c", "", &original, "1", None, true)
            .apply()
            .expect("valid register should apply");
        assert_eq!(*edited.register.kind(), Kind::Coil);
        // Boolean kinds (Coil/DiscreteInput) always serialize as a default big-endian U16.
        assert_eq!(
            *edited.register.format(),
            RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default()
            )
        );
    }

    #[test]
    fn ut_empty_add_dialog_does_not_apply() {
        // A freshly opened "Add" dialog has empty fields (no slave id / value), so confirming it
        // must fail validation rather than produce a bogus register.
        assert!(EditInputDialog::new().apply().is_err());
    }
}

#[cfg(test)]
mod focus_tests {
    //! Characterization tests for the `#[derive(Focus)]`-generated event dispatch and focus cycle:
    //! `handle_events` routes a key to the focused pane, and `focus_next`/`focus_previous` cycle
    //! through the focusable panes while skipping `#[focus(when = …)]`-gated ones.
    use super::{EditInputDialog, EditInputDialogFocus};
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_codec::format::{
        BitField, Endian as RegisterEndian, Format as RegisterFormat, Resolution,
        WordOrder as RegisterWordOrder,
    };
    use ferrowl_codec::{Access, Address, Kind, Register, RegisterBuilder};
    use ferrowl_modbus::UnitId;
    use ferrowl_ui::traits::HandleEvents;

    fn numeric_dialog() -> EditInputDialog {
        let register = RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Fixed(0))
            .format(RegisterFormat::u32(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap();
        // `from_register` focuses the value field and sets the cursor at the end of "4".
        EditInputDialog::from_register("name", "", &register, "4", None, true)
    }

    fn coil_dialog() -> EditInputDialog {
        let register: Register = RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::Coil)
            .address(Address::Fixed(0))
            .format(RegisterFormat::u16(
                RegisterEndian::Big,
                RegisterWordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap();
        EditInputDialog::from_register("c", "", &register, "1", None, true)
    }

    /// Walk a full forward focus cycle, returning every focus state visited (starting state first).
    fn forward_cycle(dialog: &mut EditInputDialog) -> Vec<EditInputDialogFocus> {
        let start = dialog.focus;
        let mut seen = vec![start];
        for _ in 0..64 {
            dialog.focus_next();
            if dialog.focus == start {
                return seen;
            }
            seen.push(dialog.focus);
        }
        panic!("focus_next did not return to the starting pane within 64 steps");
    }

    #[test]
    fn ut_handle_events_types_into_focused_value_field() {
        let mut d = numeric_dialog();
        assert_eq!(d.focus, EditInputDialogFocus::Value);
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('2'));
        // The keystroke is routed to the focused value field (cursor was at the end of "4").
        assert_eq!(d.value.state.input(), "42");
        // Other fields are untouched.
        assert_eq!(d.label.state.input(), "name");
    }

    #[test]
    fn ut_handle_events_follows_focus_to_another_pane() {
        let mut d = numeric_dialog();
        d.focus = EditInputDialogFocus::Label;
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('x'));
        // Now the label receives the keystroke; the value field stays at "4".
        assert_eq!(d.label.state.input(), "namex");
        assert_eq!(d.value.state.input(), "4");
    }

    #[test]
    fn ut_focus_cycle_wraps_and_visits_core_panes() {
        let mut d = numeric_dialog();
        let seen = forward_cycle(&mut d);
        // Wrapped back to the starting pane.
        assert_eq!(d.focus, EditInputDialogFocus::Value);
        // Core always-present panes and the editing register's numeric + delete panes are reached.
        for expected in [
            EditInputDialogFocus::Label,
            EditInputDialogFocus::SlaveId,
            EditInputDialogFocus::Address,
            EditInputDialogFocus::Value,
            EditInputDialogFocus::NumberFormat,
            EditInputDialogFocus::ConfirmButton,
            EditInputDialogFocus::DeleteRegisterButton,
        ] {
            assert!(
                seen.contains(&expected),
                "cycle missing {expected:?}: {seen:?}"
            );
        }
    }

    #[test]
    /// MB-R-099 — the register-order pane is in the cycle for a multi-register format (U32)
    /// but gated off for a single-register one (U16).
    fn ut_focus_cycle_gates_word_order_on_register_width() {
        let mut multi = numeric_dialog(); // U32
        assert!(
            forward_cycle(&mut multi).contains(&EditInputDialogFocus::NumberWordOrder),
            "multi-register cycle should visit NumberWordOrder"
        );

        let single = RegisterBuilder::default()
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
        let mut single = EditInputDialog::from_register("name", "", &single, "4", None, true);
        assert!(
            !forward_cycle(&mut single).contains(&EditInputDialogFocus::NumberWordOrder),
            "single-register cycle should skip NumberWordOrder"
        );
    }

    #[test]
    fn ut_focus_previous_reverses_focus_next() {
        let mut d = numeric_dialog();
        let start = d.focus;
        d.focus_next();
        assert_ne!(d.focus, start);
        d.focus_previous();
        assert_eq!(d.focus, start);
    }

    #[test]
    fn ut_focus_cycle_skips_gated_number_panes_for_boolean_kind() {
        let mut d = coil_dialog();
        let seen = forward_cycle(&mut d);
        // Coil/DiscreteInput are boolean: the type selector and all numeric/text sub-panes are
        // gated off and must be skipped by the cycle.
        for gated in [
            EditInputDialogFocus::ValueType,
            EditInputDialogFocus::NumberFormat,
            EditInputDialogFocus::NumberEndian,
            EditInputDialogFocus::NumberResolution,
            EditInputDialogFocus::TextAlignment,
            EditInputDialogFocus::TextWidth,
        ] {
            assert!(
                !seen.contains(&gated),
                "boolean cycle should skip {gated:?}: {seen:?}"
            );
        }
        // The value field is still reachable.
        assert!(seen.contains(&EditInputDialogFocus::Value));
    }
}
