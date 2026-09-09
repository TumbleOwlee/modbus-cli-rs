//! MB-R-140 — the monitor's setup dialog: name, an optional device-config path, and the
//! `Rtu`/`Ascii` transport field set only (no TLS, no role switch — a monitor tab is created and
//! stays a monitor). Mirrors `module/modbus/setup_dialog.rs::SetupDialog` at roughly 1/20th the
//! size, since a monitor has no TCP/UDP/RtuOverTcp/AsciiOverTcp fields and no role selector.

use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use ferrowl_ui::{
    Border, COLOR_SCHEME, EventResult, render_field, render_row,
    state::{InputFieldState, SelectionState, SuggestInputState},
    style::{InputFieldStyle, SelectionStyle, TextStyle},
    traits::{HandleEvents, ToLabel},
    widgets::{
        GetValue, InputField, Selection, SuggestInput, Text, TextBuilder, Validate, ValidateResult,
        Widget,
    },
};
use ferrowl_ui_derive::{Focus, focusable};
use ferrowl_util::convert::FileType;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect},
    widgets::{Block, Clear, Widget as UiWidget},
};

use crate::config::{Endpoint, ModuleSpec, MonitorDeviceConfig};
use crate::dialog::NonEmpty;
use crate::dialog::choices::{DialogMode, Parity, ReconnectChoice, U8Choice, select_u8};
use crate::dialog::close_confirm::{CloseConfirmDialog, CloseConfirmOutcome, route_close_confirm};
use crate::dialog::path_suggest::FsPathProvider;
use crate::dialog::widgets::{input, selection, set_input, set_suggest_input, suggest_input};

use super::build::endpoint_to_monitor_config;

/// The validated per-instance settings.
pub struct MonitorSetupValues {
    pub name: String,
    pub endpoint: Endpoint,
    pub reconnect: bool,
    /// The device-config-path field's current value, always populated regardless of
    /// `New`/`Edit` mode (unlike `MonitorSetupOutcome::device`, which only re-loads the device
    /// config file in `New` mode): an edit-confirm must apply this field even
    /// though it never re-loads a device config from it, mirroring `SetupValues::config_path`'s
    /// own always-populated shape in the full client/server module's setup dialog.
    pub config_path: String,
}

/// The full validated dialog result. `device` is set in New mode: the config path (or `""`)
/// and the loaded (or empty) device config.
pub struct MonitorSetupOutcome {
    pub values: MonitorSetupValues,
    pub device: Option<(String, MonitorDeviceConfig)>,
}

/// Transport selection value — `Rtu`/`Ascii` only (MB-R-140).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Transport {
    Rtu,
    Ascii,
}

impl ToLabel for Transport {
    fn to_label(&self) -> String {
        match self {
            Transport::Rtu => "RTU",
            Transport::Ascii => "ASCII",
        }
        .to_string()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ConfigPath;

impl Validate for ConfigPath {
    fn validate(input: &str) -> ValidateResult {
        let input = input.trim();
        let resolved = ferrowl_util::path::expand(input);

        if input.is_empty() {
            ValidateResult::None
        } else if FileType::from_path(input).is_some() {
            if resolved.exists() {
                match crate::config::load_monitor_device(input) {
                    Ok(_) => ValidateResult::Success,
                    Err(e) => ValidateResult::Error(format!("Config: {e}")),
                }
            } else {
                ValidateResult::None
            }
        } else {
            ValidateResult::Error("Invalid filetype, TOML or JSON expected.".to_string())
        }
    }
}

#[focusable]
#[derive(Builder, Focus)]
pub struct MonitorSetupDialog {
    #[focus]
    pub(crate) name: Widget<InputFieldState, InputField<NonEmpty>>,
    #[focus]
    pub(crate) config_path:
        Widget<SuggestInputState<FsPathProvider>, SuggestInput<ConfigPath, FsPathProvider>>,
    #[focus]
    pub(crate) transport: Widget<SelectionState<Transport>, Selection<Transport>>,
    #[focus]
    pub(crate) path:
        Widget<SuggestInputState<FsPathProvider>, SuggestInput<String, FsPathProvider>>,
    #[focus]
    pub(crate) baud: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub(crate) reconnect: Widget<SelectionState<ReconnectChoice>, Selection<ReconnectChoice>>,
    #[focus]
    pub(crate) parity: Widget<SelectionState<Parity>, Selection<Parity>>,
    #[focus]
    pub(crate) data_bits: Widget<SelectionState<U8Choice>, Selection<U8Choice>>,
    #[focus]
    pub(crate) stop_bits: Widget<SelectionState<U8Choice>, Selection<U8Choice>>,
    error: Widget<String, Text>,
    keybinds: Widget<String, Text>,
    mode: DialogMode,
    /// Confirm-close popup, opened with Esc.
    #[builder(default)]
    close_confirm: Option<CloseConfirmDialog>,
    /// Set once the close-confirm popup is confirmed; the host checks this via
    /// `take_close_request` and closes the dialog.
    #[builder(default)]
    close_requested: bool,
}

impl MonitorSetupDialog {
    /// Create a new monitor module (`:n`/`:new`), with an optional device-config path.
    pub fn create() -> Self {
        Self::build("", "", DialogMode::New)
    }

    /// Edit an existing monitor instance (`:e`).
    pub fn edit(name: &str, spec: &ModuleSpec, device: &MonitorDeviceConfig) -> Self {
        let mut dialog = Self::build(name, &spec.device, DialogMode::Edit);
        match &spec.endpoint {
            Endpoint::Rtu {
                path,
                baud_rate,
                parity,
                data_bits,
                stop_bits,
            } => {
                dialog.transport.state.set_selection(0);
                set_suggest_input(&mut dialog.path, path);
                set_input(&mut dialog.baud, &baud_rate.to_string());
                dialog
                    .parity
                    .state
                    .set_selection(Parity::from_config(parity.as_deref()).index());
                select_u8(&mut dialog.data_bits.state, *data_bits);
                select_u8(&mut dialog.stop_bits.state, *stop_bits);
            }
            Endpoint::Ascii {
                path,
                baud_rate,
                parity,
                data_bits,
                stop_bits,
            } => {
                dialog.transport.state.set_selection(1);
                set_suggest_input(&mut dialog.path, path);
                set_input(&mut dialog.baud, &baud_rate.to_string());
                dialog
                    .parity
                    .state
                    .set_selection(Parity::from_config(parity.as_deref()).index());
                select_u8(&mut dialog.data_bits.state, *data_bits);
                select_u8(&mut dialog.stop_bits.state, *stop_bits);
            }
            // MB-R-140 is only enforced at `ModbusMonitorModule::start()`, never at
            // construction — a hand-edited session file can carry `role = "monitor"` with a
            // non-serial transport (e.g. `transport = "tcp"`). Such a tab loads fine and fails
            // `:start` gracefully (logged); `:edit` on it must not panic either. Degrade to the
            // dialog's own Rtu default instead (leaves path/baud/parity/data_bits/stop_bits at
            // their placeholder defaults, since a Tcp/Udp/RtuOverTcp/AsciiOverTcp endpoint
            // carries no equivalent serial fields to prefill from).
            Endpoint::Tcp { .. }
            | Endpoint::RtuOverTcp { .. }
            | Endpoint::Udp { .. }
            | Endpoint::AsciiOverTcp { .. } => {
                dialog.transport.state.set_selection(0);
            }
        }
        dialog.reconnect.state.set_selection(
            if device
                .reconnect
                .unwrap_or(crate::config::device::DEFAULT_RECONNECT)
            {
                0
            } else {
                1
            },
        );
        dialog
    }

    fn build(name: &str, config_path: &str, mode: DialogMode) -> Self {
        let selection_style = SelectionStyle::default();
        let input_style = InputFieldStyle::default();
        let error_style = TextStyle {
            general: ratatui::style::Style::default()
                .fg(COLOR_SCHEME.error)
                .bg(COLOR_SCHEME.bg),
        };

        let mut name_field = input("Name", "module name", &input_style, true);
        set_input(&mut name_field, name);
        let mut config_path_field = suggest_input(
            "Config Path [TOML/JSON] (optional)",
            "device.toml",
            &input_style,
            false,
            FsPathProvider::with_extensions(&["toml", "json"]),
        );
        set_suggest_input(&mut config_path_field, config_path);

        let mut dialog = MonitorSetupDialogBuilder::default()
            .name(name_field)
            .config_path(config_path_field)
            .transport(selection(
                "Transport",
                vec![Transport::Rtu, Transport::Ascii],
                &selection_style,
            ))
            .path(suggest_input(
                "Serial Path",
                "/dev/ttyUSB0",
                &input_style,
                false,
                FsPathProvider::default(),
            ))
            .baud(input("Baud", "19200", &input_style, false))
            .parity(selection(
                "Parity",
                vec![Parity::None, Parity::Odd, Parity::Even],
                &selection_style,
            ))
            .data_bits(selection(
                "Data Bits",
                vec![U8Choice(8), U8Choice(7), U8Choice(6), U8Choice(5)],
                &selection_style,
            ))
            .stop_bits(selection(
                "Stop Bits",
                vec![U8Choice(1), U8Choice(2)],
                &selection_style,
            ))
            .reconnect(selection(
                "Reconnect",
                vec![ReconnectChoice::On, ReconnectChoice::Off],
                &selection_style,
            ))
            .error(Widget {
                state: String::new(),
                widget: TextBuilder::default()
                    .title(Some("Error".into()))
                    .border(Border::Full(Margin::new(1, 0)))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .multiline(true)
                    .style(error_style)
                    .build()
                    .expect("all required builder fields are set"),
            })
            .keybinds(Widget {
                state: "<Tab> next   <Enter> confirm   <Esc> cancel".to_string(),
                widget: TextBuilder::default()
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .style(TextStyle::default())
                    .build()
                    .expect("all required builder fields are set"),
            })
            .mode(mode)
            .focus(MonitorSetupDialogFocus::Name)
            .build()
            .expect("all required builder fields are set");

        dialog
            .reconnect
            .state
            .set_selection(if crate::config::device::DEFAULT_RECONNECT {
                0
            } else {
                1
            });
        dialog
    }

    pub fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        match route_close_confirm(&mut self.close_confirm, modifiers, code) {
            CloseConfirmOutcome::NotActive => {}
            CloseConfirmOutcome::Close => {
                self.close_requested = true;
                return EventResult::Consumed;
            }
            CloseConfirmOutcome::Consumed => return EventResult::Consumed,
        }

        if code == KeyCode::Tab && modifiers == KeyModifiers::NONE {
            self.focus_next();
            return EventResult::Consumed;
        }
        if code == KeyCode::BackTab {
            self.focus_previous();
            return EventResult::Consumed;
        }
        if modifiers == KeyModifiers::NONE && code == KeyCode::Esc {
            self.close_confirm = Some(CloseConfirmDialog::new());
            return EventResult::Consumed;
        }
        HandleEvents::handle_events(self, modifiers, code)
    }

    /// Whether the close-confirm popup was confirmed since the last call; clears the flag.
    pub fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }

    /// Resolve the dialog's current field values, validating the transport (MB-R-140,
    /// belt-and-suspenders — the picker already structurally excludes every non-Rtu/Ascii
    /// transport).
    pub fn resolve(&self) -> Result<MonitorSetupOutcome, String> {
        let values = self.values()?;
        let device = if self.mode == DialogMode::New {
            let path = self.config_path.state.input().trim().to_string();
            if path.is_empty() || !ferrowl_util::path::expand(&path).exists() {
                Some((path, MonitorDeviceConfig::default()))
            } else {
                let device = crate::config::load_monitor_device(&path)
                    .map_err(|e| format!("Config: {e}"))?;
                Some((path, device))
            }
        } else {
            None
        };
        Ok(MonitorSetupOutcome { values, device })
    }

    fn values(&self) -> Result<MonitorSetupValues, String> {
        let name = self.name.state.input().trim().to_string();
        if name.is_empty() {
            return Err("Name is required.".into());
        }
        let config_path = self.config_path.state.input().trim().to_string();
        if !config_path.is_empty() && FileType::from_path(&config_path).is_none() {
            return Err(format!(
                "Unknown format for '{config_path}' (use .toml or .json)"
            ));
        }

        let mut path = self.path.state.input().trim().to_string();
        if path.is_empty() {
            path = "/dev/ttyUSB0".to_string();
        }
        let baud_rate = self.baud.state.input();
        let baud_rate = if !baud_rate.is_empty() {
            baud_rate
                .trim()
                .parse::<u32>()
                .map_err(|_| "Baud rate must be a number.".to_string())?
        } else {
            19200
        };
        let parity = self.parity.state.get_value().to_config();
        let data_bits = Some(self.data_bits.state.get_value().0);
        let stop_bits = Some(self.stop_bits.state.get_value().0);

        let endpoint = match self.transport.state.get_value() {
            Transport::Rtu => Endpoint::Rtu {
                path,
                baud_rate,
                parity,
                data_bits,
                stop_bits,
            },
            Transport::Ascii => Endpoint::Ascii {
                path,
                baud_rate,
                parity,
                data_bits,
                stop_bits,
            },
        };
        let reconnect = self.reconnect.state.get_value() == ReconnectChoice::On;

        // Belt-and-suspenders (MB-R-140): the picker structurally offers only Rtu/Ascii, but
        // validate through the same resolution step every other call site uses anyway.
        endpoint_to_monitor_config(&endpoint, reconnect).map_err(|e| e.0.to_string())?;

        Ok(MonitorSetupValues {
            name,
            endpoint,
            reconnect,
            config_path,
        })
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        // Reflect validation state in the error field.
        match self.resolve() {
            Ok(_) => self.error.state.clear(),
            Err(e) => self.error.state = e,
        }

        // border(2) + margin(2) + name(3) + config_path(3) + transport(3) + endpoint(6:
        // path/baud/reconnect, parity/data_bits/stop_bits) + error(4) + keybinds(1).
        let box_height = 24;

        let [_, hcenter, _] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(60),
            Constraint::Min(1),
        ])
        .areas(area);
        let [_, vcenter, _] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(box_height),
            Constraint::Min(1),
        ])
        .areas(hcenter);

        Clear.render(vcenter, buf);
        let block = Block::bordered()
            .style(
                ratatui::style::Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg),
            )
            .title_alignment(HorizontalAlignment::Center)
            .title(match self.mode {
                DialogMode::New => "New Monitor",
                DialogMode::Edit => "Edit Monitor",
            });
        let inner = block.inner(vcenter).inner(Margin::new(2, 1));
        block.render(vcenter, buf);

        let rows = Layout::vertical([
            Constraint::Length(3), // name
            Constraint::Length(3), // config path
            Constraint::Length(3), // transport
            Constraint::Length(6), // endpoint (path/baud/reconnect, parity/data_bits/stop_bits)
            Constraint::Length(4), // error
            Constraint::Length(1), // keybinds
        ])
        .split(inner);

        render_field!(self, name, rows[0], buf);
        render_field!(self, config_path, rows[1], buf);
        render_field!(self, transport, rows[2], buf);

        let [row0, row1] =
            Layout::vertical([Constraint::Length(3), Constraint::Length(3)]).areas(rows[3]);
        render_row!(self, row0, buf; path, baud, reconnect);
        render_row!(self, row1, buf;
            parity => Constraint::Percentage(35),
            data_bits => Constraint::Percentage(30),
            stop_bits => Constraint::Percentage(35)
        );

        if !self.error.state.is_empty() {
            render_field!(self, error, rows[4], buf);
        }
        render_field!(self, keybinds, rows[5], buf);

        // Suggestion popups draw last, over everything else in the dialog (and may overflow
        // the dialog box itself), so both must be rendered after all sibling widgets above —
        // mirrors `module/modbus/setup_dialog.rs::SetupDialog::render`'s trailing
        // `render_overlay` calls.
        self.config_path
            .widget
            .render_overlay(area, buf, &mut self.config_path.state);
        self.path
            .widget
            .render_overlay(area, buf, &mut self.path.state);

        if let Some(d) = self.close_confirm.as_mut() {
            d.render(vcenter, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_ui::traits::{IsFocus, SetFocus};

    /// The dialog renders as a centered floating popup, same as every other setup dialog
    /// (`module/modbus/setup_dialog.rs::SetupDialog`), rather than filling the area it is given:
    /// a far corner of a larger area stays untouched by its border and content.
    #[test]
    fn ut_render_centers_the_dialog_instead_of_filling_the_area() {
        let mut dialog = MonitorSetupDialog::create();
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), " ", "top-left corner must stay blank");
        assert_eq!(
            buf[(99, 59)].symbol(),
            " ",
            "bottom-right corner must stay blank"
        );
    }

    /// The config-path field's filesystem completion popup (already wired via
    /// `SuggestInput<ConfigPath, FsPathProvider>`, same as the modbus module's own
    /// `SetupDialog::config_path`) must actually draw: `render` must call `render_overlay` for
    /// it, same as `module/modbus/setup_dialog.rs`'s trailing `render_overlay` calls.
    #[test]
    fn ut_render_config_path_field_shows_suggestion_popup() {
        let mut dialog = MonitorSetupDialog::create();
        dialog.config_path.state.set_focused(true);
        dialog
            .config_path
            .state
            .handle_events(KeyModifiers::NONE, KeyCode::Char('s'));
        assert!(dialog.config_path.state.suggestions_open());

        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(text.contains("src"), "missing suggestion popup:\n{text}");
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// `render` populates `error.state` from `resolve()`, same as
    /// `module/modbus/setup_dialog.rs::SetupDialog`, so an invalid dialog carries its message
    /// (here: an empty name) and a valid one carries none.
    #[test]
    fn ut_render_populates_error_state_from_resolve() {
        let mut dialog = MonitorSetupDialog::create();
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);

        dialog.render(area, &mut buf); // empty name -> invalid
        assert!(!dialog.error.state.is_empty());
        assert!(dialog.error.state.contains("Name is required"));

        set_input(&mut dialog.name, "mon1");
        dialog.render(area, &mut buf); // valid now
        assert!(dialog.error.state.is_empty());
    }

    /// The error box stays hidden — no border or title drawn — while there is no error, and
    /// appears with its message once one is present.
    #[test]
    fn ut_render_hides_error_box_when_valid_and_shows_it_when_invalid() {
        let area = Rect::new(0, 0, 100, 60);

        let mut valid = MonitorSetupDialog::create();
        set_input(&mut valid.name, "mon1");
        let mut buf = Buffer::empty(area);
        valid.render(area, &mut buf);
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            !text.contains("Error"),
            "no error box should be drawn while the dialog is valid"
        );

        let mut invalid = MonitorSetupDialog::create(); // empty name -> invalid
        let mut buf = Buffer::empty(area);
        invalid.render(area, &mut buf);
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.contains("Name is required"),
            "the error message must be visible once the dialog is invalid"
        );
    }

    /// Serial path/baud/reconnect share one row and parity/data bits/stop bits share the next,
    /// same as `module/modbus/setup_dialog.rs::SetupDialog`'s RTU layout.
    #[test]
    fn ut_render_lays_out_endpoint_fields_in_two_shared_rows() {
        let mut dialog = MonitorSetupDialog::create();
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);

        let row_text = |y: u16| -> String {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        };
        // The dialog is horizontally centered in a 60-wide box, so its rows start at x=20.
        // The path/baud/reconnect titles must appear on the same rendered row.
        let combined = (0..area.height).map(row_text).collect::<Vec<_>>();
        let path_row = combined
            .iter()
            .position(|l| l.contains("Serial Path"))
            .expect("Serial Path row present");
        assert!(
            combined[path_row].contains("Baud") && combined[path_row].contains("Reconnect"),
            "Serial Path, Baud and Reconnect must render on the same row, got: {:?}",
            combined[path_row]
        );
    }

    /// Tab order follows the visual row layout — Serial Path, Baud, Reconnect on one row;
    /// Parity, Data Bits, Stop Bits on the next — same as
    /// `module/modbus/setup_dialog.rs::SetupDialog`'s RTU field order. Declaration order is what
    /// the `Focus` derive walks, so it has to match the rendered rows.
    #[test]
    fn ut_tab_order_follows_path_baud_reconnect_then_parity_data_bits_stop_bits() {
        let mut dialog = MonitorSetupDialog::create();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::Name);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::ConfigPath);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::Transport);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::Path);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::Baud);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::Reconnect);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::Parity);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::DataBits);
        dialog.focus_next();
        assert_eq!(dialog.focus, MonitorSetupDialogFocus::StopBits);
    }

    /// UI-R-067 — the name field is focused, and so shows a cursor, as soon as the dialog opens,
    /// same as `module/modbus/setup_dialog.rs::SetupDialog`. UI-R-194 — every other field opens
    /// unfocused.
    #[test]
    fn ut_create_focuses_the_name_field_by_default() {
        let dialog = MonitorSetupDialog::create();
        assert!(dialog.name.state.is_focused(), "name must open focused");
        assert_eq!(
            dialog.focus,
            MonitorSetupDialogFocus::Name,
            "the focus cursor must name the field carrying the flag"
        );

        // `focus == Name` alone survives a reordering that moved `Name` out of first position;
        // stepping back from it must wrap away, and forward from there must wrap to it.
        let mut walked = MonitorSetupDialog::create();
        walked.focus_previous();
        assert_ne!(walked.focus, MonitorSetupDialogFocus::Name);
        walked.focus_next();
        assert_eq!(walked.focus, MonitorSetupDialogFocus::Name);
        for (label, focused) in [
            ("config_path", dialog.config_path.state.is_focused()),
            ("transport", dialog.transport.state.is_focused()),
            ("path", dialog.path.state.is_focused()),
            ("baud", dialog.baud.state.is_focused()),
            ("reconnect", dialog.reconnect.state.is_focused()),
            ("parity", dialog.parity.state.is_focused()),
            ("data_bits", dialog.data_bits.state.is_focused()),
            ("stop_bits", dialog.stop_bits.state.is_focused()),
        ] {
            assert!(!focused, "{label} must open unfocused");
        }
    }

    /// UI-R-067 — the `:edit` open path establishes the same single-focus state as `create`,
    /// checked against the `Focus` derive's own normalisation rather than a hand-listed field set.
    #[test]
    fn ut_edit_matches_the_derive_normalised_focus_state() {
        let spec = ModuleSpec {
            name: "mon1".to_string(),
            device: String::new(),
            role: crate::config::Role::Monitor,
            endpoint: Endpoint::Ascii {
                path: "/dev/ttyS0".to_string(),
                baud_rate: 9600,
                parity: None,
                data_bits: Some(8),
                stop_bits: Some(1),
            },
        };
        let device = MonitorDeviceConfig::default();
        let area = Rect::new(0, 0, 100, 60);

        let mut as_built = MonitorSetupDialog::edit("mon1", &spec, &device);
        assert!(as_built.name.state.is_focused(), "name must open focused");
        let mut built = Buffer::empty(area);
        as_built.render(area, &mut built);

        let mut normalised = MonitorSetupDialog::edit("mon1", &spec, &device);
        normalised.set_focused(true);
        let mut after = Buffer::empty(area);
        normalised.render(area, &mut after);

        assert_eq!(
            built, after,
            "edit-path focus state differs from the derive's normalised state"
        );
    }

    /// UI-R-195 — a freshly created dialog paints exactly one text cursor, in the one focused
    /// field. The cursor rather than the border, because `name` opens empty against `NonEmpty` and
    /// so paints its error border, not its focused one.
    #[test]
    fn ut_create_paints_one_cursor() {
        let mut dialog = MonitorSetupDialog::create();
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);

        let cursor_bg = ferrowl_ui::style::InputFieldStyle::default().cursor().bg;
        let cursors = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| Some(buf[(x, y)].bg) == cursor_bg)
            .count();
        assert_eq!(cursors, 1, "expected one cursor:\n{}", buffer_text(&buf));
    }

    /// A margin separates the dialog's border from its content — one row vertically, two columns
    /// horizontally (`Margin::new(2, 1)`), same as `module/modbus/setup_dialog.rs::SetupDialog`.
    #[test]
    fn ut_render_leaves_a_margin_between_border_and_content() {
        let mut dialog = MonitorSetupDialog::create();
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);

        // The box is centered in a 60-wide, 24-tall region starting at x=20, y=18.
        let border_top_y = 18;
        let border_left_x = 20;

        // The row directly under the top border must be a blank vertical-margin row (no field
        // content drawn there yet).
        let margin_row: String = (border_left_x + 1..80 - 1)
            .map(|x| buf[(x, border_top_y + 1)].symbol())
            .collect();
        assert!(
            margin_row.trim().is_empty(),
            "row directly under the top border must be blank, got: {margin_row:?}"
        );

        // The column directly right of the left border, on the first field's row, must be a
        // blank horizontal-margin column (the field's own border starts two cells in).
        let first_field_row_y = border_top_y + 2;
        assert_eq!(
            buf[(border_left_x + 1, first_field_row_y)].symbol(),
            " ",
            "column directly right of the left border must be blank"
        );
    }

    /// The dialog's border and background use `COLOR_SCHEME.hi`/`COLOR_SCHEME.bg` and its title
    /// is centered, same as `module/modbus/setup_dialog.rs::SetupDialog`.
    #[test]
    fn ut_render_styles_the_border_and_centers_the_title() {
        let mut dialog = MonitorSetupDialog::create();
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);

        // The box is centered in a 60-wide, 24-tall region starting at x=20, y=18.
        let top_border_cell = &buf[(20, 18)];
        assert_eq!(top_border_cell.fg, COLOR_SCHEME.hi);
        assert_eq!(top_border_cell.bg, COLOR_SCHEME.bg);

        let title_chars: Vec<&str> = (20..80).map(|x| buf[(x, 18)].symbol()).collect();
        let title_row: String = title_chars.concat();
        assert!(title_row.contains("New Monitor"));
        // Centered within the 60-wide box: left padding to the title must roughly match right
        // padding (offset measured in cells, not bytes — border glyphs are multi-byte UTF-8).
        let title_start = title_chars
            .iter()
            .position(|c| *c == "N")
            .expect("title starts with 'N'");
        assert!(
            (15..25).contains(&title_start),
            "title should be roughly centered, started at cell offset {title_start}"
        );
    }

    /// UI-R-023, UI-R-112 — Esc opens the close-confirm popup and Enter confirms it, same as
    /// `module/modbus/setup_dialog.rs::SetupDialog`.
    #[test]
    fn ut_esc_then_enter_sets_close_request_and_clears_after_take() {
        let mut dialog = MonitorSetupDialog::create();
        assert!(!dialog.take_close_request());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_some());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(dialog.take_close_request());
        assert!(!dialog.take_close_request(), "flag must clear after take");
    }

    /// MB-R-140 — the monitor setup dialog offers exactly the two serial transports.
    #[test]
    fn ut_monitor_setup_dialog_transport_selector_has_exactly_two_entries() {
        let dialog = MonitorSetupDialog::create();
        assert_eq!(dialog.transport.state.values().len(), 2);
        assert_eq!(
            dialog.transport.state.values(),
            &[Transport::Rtu, Transport::Ascii]
        );
    }

    /// MB-R-191 — `resolve()` still rejects a non-serial endpoint even when the picker itself
    /// is bypassed by constructing the dialog's internal state directly, proving the check isn't
    /// solely enforced by the picker's restriction.
    #[test]
    fn ut_monitor_setup_dialog_resolve_rejects_non_serial_endpoint_if_reachable() {
        // Directly exercise the belt-and-suspenders validation path used by `values()`, since
        // the dialog's `Transport` enum has no Tcp/Udp variant to select through the picker.
        let err = endpoint_to_monitor_config(
            &Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 502,
            },
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("rtu or ascii"));
    }

    #[test]
    fn ut_resolve_default_dialog_builds_rtu_endpoint() {
        let mut dialog = MonitorSetupDialog::create();
        set_input(&mut dialog.name, "mon1");
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.name, "mon1");
        assert!(matches!(outcome.values.endpoint, Endpoint::Rtu { .. }));
    }

    #[test]
    fn ut_resolve_without_name_errors() {
        let dialog = MonitorSetupDialog::create();
        assert!(dialog.resolve().is_err());
    }

    #[test]
    fn ut_edit_prefills_transport_and_path() {
        let spec = ModuleSpec {
            name: "mon1".to_string(),
            device: String::new(),
            role: crate::config::Role::Monitor,
            endpoint: Endpoint::Ascii {
                path: "/dev/ttyS0".to_string(),
                baud_rate: 9600,
                parity: None,
                data_bits: Some(8),
                stop_bits: Some(1),
            },
        };
        let device = MonitorDeviceConfig::default();
        let dialog = MonitorSetupDialog::edit("mon1", &spec, &device);
        assert_eq!(dialog.transport.state.get_value(), Transport::Ascii);
        assert_eq!(dialog.path.state.input(), "/dev/ttyS0");
    }

    /// `:edit` on a monitor tab whose session file carries a non-serial transport degrades to
    /// the dialog's Rtu default rather than panicking (see `edit`'s own comment for why such a
    /// spec can exist at all).
    #[test]
    fn ut_edit_does_not_panic_on_non_serial_endpoint() {
        let spec = ModuleSpec {
            name: "mon1".to_string(),
            device: String::new(),
            role: crate::config::Role::Monitor,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".to_string(),
                port: 502,
            },
        };
        let device = MonitorDeviceConfig::default();
        let dialog = MonitorSetupDialog::edit("mon1", &spec, &device);
        assert_eq!(dialog.transport.state.get_value(), Transport::Rtu);
    }
}
