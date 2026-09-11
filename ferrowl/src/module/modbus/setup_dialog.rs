//! Module setup dialog. In **Edit** mode (`:e`) it edits the current tab's per-instance
//! settings (name, transport + endpoint, role). In **New** mode (`:n`/`:new`) it additionally
//! takes an optional device-config path: empty creates an empty module, otherwise the path is
//! validated live and must point at a loadable config. While any field is invalid the dialog
//! cannot be confirmed (only cancelled with Esc).

use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use ferrowl_ui::{
    Border, COLOR_SCHEME, EventResult, render_field, render_row,
    state::{InputFieldState, SelectionState, SuggestInputState},
    style::{InputFieldStyle, SelectionStyle, TextStyle},
    traits::HandleEvents,
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

use crate::config::device::ReadRanges;
use crate::config::{ClientOrServer, DeviceConfig, Endpoint, Role};
use crate::dialog::NonEmpty;
use crate::dialog::close_confirm::{CloseConfirmDialog, CloseConfirmOutcome, route_close_confirm};
use crate::dialog::path_suggest::FsPathProvider;
use crate::dialog::tls_section::{EffectiveTlsLevel, TlsSection, TlsSectionFocus};
use crate::dialog::widgets::{input, selection, set_input, set_suggest_input, suggest_input};
use ferrowl_modbus::tcp::ModbusTlsConfig;

use super::build::Timing;

mod choices;
use crate::dialog::choices::{DialogMode, Parity, ReconnectChoice, U8Choice, select_u8};
use choices::Transport;
mod tls;
use tls::{TlsInputs, TlsLevel, validate_tls};

/// The validated per-instance settings.
pub struct SetupValues {
    pub name: String,
    pub config_path: String,
    pub role: Role,
    pub endpoint: Endpoint,
    /// Optional per-instance timing overrides (ms); `None` falls back to device/app config.
    pub timeout_ms: Option<usize>,
    pub delay_ms: Option<usize>,
    pub interval_ms: Option<usize>,
    /// Auto-reconnect setting (MB-R-050–MB-R-055 client, MB-R-130–MB-R-134 server); always
    /// explicit after a dialog save.
    pub reconnect: Option<bool>,
    /// Explicit per-function-code read ranges (client only), applied to the device config.
    pub read_ranges: ReadRanges,
    /// The device config's `tls` setting (MB-R-104). `None` when the TLS section is hidden
    /// (RTU transport, MB-R-112) — a hidden section must never clobber a device config's
    /// existing setting. `Some(cfg)` means shown, with `cfg` both-`None` when the level is Off
    /// (MB-R-104's two-role container makes "off" a value of `ModbusTlsConfig` itself, not a
    /// separate `Option` layer).
    pub tls: Option<ModbusTlsConfig>,
}

/// The full validated dialog result. `device` is set in New mode: the config path (or
/// `""`) and the loaded (or empty) device config.
pub struct SetupOutcome {
    pub values: SetupValues,
    pub device: Option<(String, DeviceConfig)>,
}

#[derive(Debug, Clone)]
pub struct ConfigPath;

impl Validate for ConfigPath {
    fn validate(input: &str) -> ValidateResult {
        let input = input.trim();
        let resolved = ferrowl_util::path::expand(input);

        if input.is_empty() {
            ValidateResult::None
        } else if FileType::from_path(input).is_some() {
            if resolved.exists() {
                match crate::config::load_device(input) {
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
pub struct SetupDialog {
    #[focus]
    pub name: Widget<InputFieldState, InputField<NonEmpty>>,
    #[focus]
    pub config_path:
        Widget<SuggestInputState<FsPathProvider>, SuggestInput<ConfigPath, FsPathProvider>>,
    #[focus]
    pub transport: Widget<SelectionState<Transport>, Selection<Transport>>,
    /// TLS level, offered for TCP, RtuOverTcp (MB-R-115), and AsciiOverTcp (MB-R-127) — not
    /// RTU or Ascii (MB-R-112/121: neither carries a `tls` field at all).
    #[focus(when = {matches!(self.transport.get_value(), Transport::Tcp | Transport::RtuOverTcp | Transport::AsciiOverTcp)})]
    pub tls_level: Widget<SelectionState<TlsLevel>, Selection<TlsLevel>>,
    #[focus]
    pub role: Widget<SelectionState<ClientOrServer>, Selection<ClientOrServer>>,
    /// The TLS/mTLS cluster (self-signed toggle, own-identity cert/key pair, skip-verify toggle,
    /// peer-verification input, client-CA add/remove list), shared with the OCPP setup dialog.
    /// `tls_shown()` alone gates entry — once entered, `TlsSection`'s own internal `when` gates
    /// (fed by `sync`, called at the top of every funnel method below) take over.
    #[focus(nested, when = {self.tls_shown()})]
    pub tls: TlsSection,
    #[focus(when = {matches!(self.transport.get_value(), Transport::Tcp | Transport::RtuOverTcp | Transport::Udp | Transport::AsciiOverTcp)})]
    pub ip: Widget<InputFieldState, InputField<String>>,
    #[focus(when = {matches!(self.transport.get_value(), Transport::Tcp | Transport::RtuOverTcp | Transport::Udp | Transport::AsciiOverTcp)})]
    pub port: Widget<InputFieldState, InputField<String>>,
    #[focus(when = {matches!(self.transport.get_value(), Transport::Rtu | Transport::Ascii)})]
    pub path: Widget<SuggestInputState<FsPathProvider>, SuggestInput<String, FsPathProvider>>,
    #[focus(when = {matches!(self.transport.get_value(), Transport::Rtu | Transport::Ascii)})]
    pub baud: Widget<InputFieldState, InputField<String>>,
    /// Governs client redial (MB-R-050–055) and server bind/serial-open/mid-serve retry
    /// (MB-R-130–134); shown for every role and transport, next to Port or Baud.
    #[focus]
    pub reconnect: Widget<SelectionState<ReconnectChoice>, Selection<ReconnectChoice>>,
    #[focus(when = {matches!(self.transport.get_value(), Transport::Rtu | Transport::Ascii)})]
    pub parity: Widget<SelectionState<Parity>, Selection<Parity>>,
    #[focus(when = {matches!(self.transport.get_value(), Transport::Rtu | Transport::Ascii)})]
    pub data_bits: Widget<SelectionState<U8Choice>, Selection<U8Choice>>,
    #[focus(when = {matches!(self.transport.get_value(), Transport::Rtu | Transport::Ascii)})]
    pub stop_bits: Widget<SelectionState<U8Choice>, Selection<U8Choice>>,
    #[focus]
    pub timeout: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub delay: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub interval: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub holding_ranges: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub input_ranges: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub coil_ranges: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub discrete_ranges: Widget<InputFieldState, InputField<String>>,
    pub error: Widget<String, Text>,
    pub keybinds: Widget<String, Text>,
    pub mode: DialogMode,
    /// The original device's `tls` config, if any (Edit mode only). On save, the half of
    /// `ModbusTlsConfig` belonging to the *inactive* role is stitched back in from here, so a
    /// role toggle preserves the other role's previously-saved TLS settings instead of resetting
    /// them to `ModbusTlsConfig::default()`'s placeholder.
    #[builder(default)]
    original_tls: Option<ModbusTlsConfig>,
    /// Confirm-close popup, opened with Esc.
    #[builder(default)]
    pub close_confirm: Option<CloseConfirmDialog>,
    /// Set once the close-confirm popup is confirmed; the host checks this via
    /// `take_close_request` and closes the dialog.
    #[builder(default)]
    close_requested: bool,
}

impl SetupDialog {
    /// Edit an existing instance (`:e`). `timing` is the effective (resolved) timeout/delay/
    /// interval/reconnect settings used to prefill the inputs.
    pub fn edit(
        name: &str,
        config_path: &str,
        role: ClientOrServer,
        endpoint: &Endpoint,
        timing: Timing,
        ranges: &ReadRanges,
        tls: Option<&ModbusTlsConfig>,
    ) -> Self {
        let mut dialog = Self::build(name, config_path, DialogMode::Edit, timing, ranges);
        dialog
            .role
            .state
            .set_selection(if role == ClientOrServer::Client { 1 } else { 0 });
        // Tcp=0, Rtu=1, RtuOverTcp=2, Udp=3, Ascii=4, AsciiOverTcp=5
        match endpoint {
            Endpoint::Tcp { ip, port } => {
                dialog.transport.state.set_selection(0);
                set_input(&mut dialog.ip, ip);
                set_input(&mut dialog.port, &port.to_string());
            }
            Endpoint::RtuOverTcp { ip, port } => {
                dialog.transport.state.set_selection(2);
                set_input(&mut dialog.ip, ip);
                set_input(&mut dialog.port, &port.to_string());
            }
            Endpoint::Udp { ip, port } => {
                dialog.transport.state.set_selection(3);
                set_input(&mut dialog.ip, ip);
                set_input(&mut dialog.port, &port.to_string());
            }
            Endpoint::AsciiOverTcp { ip, port } => {
                dialog.transport.state.set_selection(5);
                set_input(&mut dialog.ip, ip);
                set_input(&mut dialog.port, &port.to_string());
            }
            Endpoint::Rtu {
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
            Endpoint::Ascii {
                path,
                baud_rate,
                parity,
                data_bits,
                stop_bits,
            } => {
                dialog.transport.state.set_selection(4);
                set_suggest_input(&mut dialog.path, path);
                set_input(&mut dialog.baud, &baud_rate.to_string());
                dialog
                    .parity
                    .state
                    .set_selection(Parity::from_config(parity.as_deref()).index());
                select_u8(&mut dialog.data_bits.state, *data_bits);
                select_u8(&mut dialog.stop_bits.state, *stop_bits);
            }
        }
        if let Some(tls) = tls {
            let level = TlsLevel::from_config(tls, role);
            dialog.tls_level.state.set_selection(level.index());
            dialog.original_tls = Some(tls.clone());
            dialog
                .tls
                .prefill(role, Some(&tls.server), Some(&tls.client));
        }
        dialog
    }

    /// Create a new module (`:n`/`:new`), with an optional device-config path. `timing` prefills
    /// the timeout/delay/interval/reconnect inputs with the global app defaults.
    pub fn create(timing: Timing) -> Self {
        Self::build("", "", DialogMode::New, timing, &ReadRanges::default())
    }

    fn build(
        name: &str,
        config_path: &str,
        mode: DialogMode,
        timing: Timing,
        ranges: &ReadRanges,
    ) -> Self {
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

        let mut dialog = SetupDialogBuilder::default()
            .name(name_field)
            .config_path(config_path_field)
            .transport(selection(
                "Transport",
                // Tcp=0, Rtu=1, RtuOverTcp=2, Udp=3, Ascii=4, AsciiOverTcp=5 — each appended
                // last to keep prior indices stable rather than reordering around them.
                vec![
                    Transport::Tcp,
                    Transport::Rtu,
                    Transport::RtuOverTcp,
                    Transport::Udp,
                    Transport::Ascii,
                    Transport::AsciiOverTcp,
                ],
                &selection_style,
            ))
            .role(selection(
                ("Role", HorizontalAlignment::Right),
                vec![ClientOrServer::Server, ClientOrServer::Client],
                &selection_style,
            ))
            .ip(input("IP", "127.0.0.1", &input_style, false))
            .port(input(
                ("Port", HorizontalAlignment::Center),
                "502",
                &input_style,
                false,
            ))
            .path(suggest_input(
                "Serial Path",
                "/dev/ttyUSB0",
                &input_style,
                false,
                FsPathProvider::default(),
            ))
            .baud(input(
                ("Baud", HorizontalAlignment::Center),
                "19200",
                &input_style,
                false,
            ))
            .parity(selection(
                "Parity",
                vec![Parity::None, Parity::Odd, Parity::Even],
                &selection_style,
            ))
            .data_bits(selection(
                ("Data Bits", HorizontalAlignment::Center),
                vec![U8Choice(8), U8Choice(7), U8Choice(6), U8Choice(5)],
                &selection_style,
            ))
            .stop_bits(selection(
                ("Stop Bits", HorizontalAlignment::Right),
                vec![U8Choice(1), U8Choice(2)],
                &selection_style,
            ))
            .tls_level(selection(
                ("TLS", HorizontalAlignment::Center),
                vec![TlsLevel::Off, TlsLevel::Tls, TlsLevel::MutualTls],
                &selection_style,
            ))
            .tls(TlsSection::new())
            .timeout(input("Timeout ms", "", &input_style, false))
            .delay(input("Delay ms", "", &input_style, false))
            .interval(input("Interval ms", "", &input_style, false))
            .reconnect(selection(
                ("Reconnect", HorizontalAlignment::Right),
                vec![ReconnectChoice::On, ReconnectChoice::Off],
                &selection_style,
            ))
            .holding_ranges(input(
                "Holding ranges",
                "0-100,140-160",
                &input_style,
                false,
            ))
            .input_ranges(input("Input ranges", "0-9", &input_style, false))
            .coil_ranges(input("Coil ranges", "0-31", &input_style, false))
            .discrete_ranges(input("Discrete ranges", "0-31", &input_style, false))
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
            .focus(SetupDialogFocus::Name)
            .build()
            .expect("all required builder fields are set");

        // Prefill timing inputs with the resolved defaults so clients always show a value.
        set_input(&mut dialog.timeout, &timing.timeout_ms.to_string());
        set_input(&mut dialog.delay, &timing.delay_ms.to_string());
        set_input(&mut dialog.interval, &timing.interval_ms.to_string());
        dialog
            .reconnect
            .state
            .set_selection(if timing.reconnect { 0 } else { 1 });

        // Prefill explicit read ranges from the device config.
        for (field, value) in [
            (&mut dialog.holding_ranges, &ranges.holding),
            (&mut dialog.input_ranges, &ranges.input),
            (&mut dialog.coil_ranges, &ranges.coils),
            (&mut dialog.discrete_ranges, &ranges.discrete),
        ] {
            if let Some(v) = value {
                set_input(field, v);
            }
        }
        dialog
    }

    // --- TLS-field visibility ------------------------------------------------------------------
    // Single source of truth consumed by the `#[focus(when)]` gates, the render branches and the
    // dialog-height computation, so keyboard focus, painting and layout can never disagree about
    // which fields exist. Mirrors `ferrowl::module::ocpp::setup_dialog`'s own `show_*` methods.

    /// The TLS level selection row (TCP, RtuOverTcp (MB-R-115), or AsciiOverTcp (MB-R-127);
    /// never RTU or Ascii, MB-R-112/121).
    fn tls_shown(&self) -> bool {
        matches!(
            self.transport.get_value(),
            Transport::Tcp | Transport::RtuOverTcp | Transport::AsciiOverTcp
        ) && self.tls_level.get_value() != TlsLevel::Off
    }

    /// The currently selected TLS level.
    fn tls_level(&self) -> TlsLevel {
        self.tls_level.get_value()
    }

    /// Push this dialog's own live role/level widgets into `self.tls` so its `when` gates read
    /// fresh state. Below `tls_shown()`, the level collapses to `Off` regardless of the raw
    /// `tls_level` widget's own value — a hidden section's fields must never appear reachable.
    fn sync_tls(&mut self) {
        let level = if self.tls_shown() {
            self.tls_level().into()
        } else {
            EffectiveTlsLevel::Off
        };
        self.tls.sync(self.role.get_value(), level);
    }

    /// Route a key: the close-confirm popup captures all keys while open; then the client-CA
    /// add-dialog (MB-R-136), if open; then the client-CA ADD/DEL buttons (Enter/Space); Esc
    /// (with nothing else open) opens the close-confirm popup; everything else falls through to
    /// the derived per-field routing.
    pub fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        self.sync_tls();

        match route_close_confirm(&mut self.close_confirm, modifiers, code) {
            CloseConfirmOutcome::NotActive => {}
            CloseConfirmOutcome::Close => {
                self.close_requested = true;
                return EventResult::Consumed;
            }
            CloseConfirmOutcome::Consumed => return EventResult::Consumed,
        }

        if self.tls.ca_add_dialog.is_some() {
            return self.tls.handle_events(modifiers, code);
        }

        if modifiers == KeyModifiers::NONE
            && matches!(code, KeyCode::Enter | KeyCode::Char(' '))
            && self.focus == SetupDialogFocus::Tls
            && matches!(
                self.tls.focus(),
                TlsSectionFocus::CaAddButton | TlsSectionFocus::CaDeleteButton
            )
        {
            return self.tls.handle_events(modifiers, code);
        }

        if modifiers == KeyModifiers::NONE && code == KeyCode::Esc {
            self.close_confirm = Some(CloseConfirmDialog::new());
            return EventResult::Consumed;
        }

        <Self as HandleEvents>::handle_events(self, modifiers, code)
    }

    /// Whether the close-confirm popup was confirmed since the last call; clears the flag.
    pub fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }

    /// Validate everything and produce the outcome. In New mode the (optional) config path is
    /// loaded/validated here, so an invalid path is reported as an error.
    pub fn resolve(&self) -> Result<SetupOutcome, String> {
        let values = self.values()?;
        let device = if self.mode == DialogMode::New {
            let path = self.config_path.state.input().trim().to_string();
            if path.is_empty() || !ferrowl_util::path::expand(&path).exists() {
                Some((path, DeviceConfig::default()))
            } else {
                let device =
                    crate::config::load_device(&path).map_err(|e| format!("Config: {e}"))?;
                Some((path, device))
            }
        } else {
            None
        };
        Ok(SetupOutcome { values, device })
    }

    fn values(&self) -> Result<SetupValues, String> {
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
        let role = self.role.state.get_value();
        let endpoint = match self.transport.state.get_value() {
            Transport::Tcp => {
                let mut ip = self.ip.state.input().trim().to_string();
                if ip.is_empty() {
                    ip = "127.0.0.1".to_string();
                }
                let port = self.port.state.input();
                let port = if !port.is_empty() {
                    port.trim()
                        .parse::<u16>()
                        .map_err(|_| "Port must be a number (0-65535).".to_string())?
                } else {
                    502
                };
                Endpoint::Tcp { ip, port }
            }
            Transport::RtuOverTcp => {
                let mut ip = self.ip.state.input().trim().to_string();
                if ip.is_empty() {
                    ip = "127.0.0.1".to_string();
                }
                let port = self.port.state.input();
                let port = if !port.is_empty() {
                    port.trim()
                        .parse::<u16>()
                        .map_err(|_| "Port must be a number (0-65535).".to_string())?
                } else {
                    502
                };
                Endpoint::RtuOverTcp { ip, port }
            }
            Transport::Udp => {
                let mut ip = self.ip.state.input().trim().to_string();
                if ip.is_empty() {
                    ip = "127.0.0.1".to_string();
                }
                let port = self.port.state.input();
                let port = if !port.is_empty() {
                    port.trim()
                        .parse::<u16>()
                        .map_err(|_| "Port must be a number (0-65535).".to_string())?
                } else {
                    502
                };
                Endpoint::Udp { ip, port }
            }
            Transport::AsciiOverTcp => {
                let mut ip = self.ip.state.input().trim().to_string();
                if ip.is_empty() {
                    ip = "127.0.0.1".to_string();
                }
                let port = self.port.state.input();
                let port = if !port.is_empty() {
                    port.trim()
                        .parse::<u16>()
                        .map_err(|_| "Port must be a number (0-65535).".to_string())?
                } else {
                    502
                };
                Endpoint::AsciiOverTcp { ip, port }
            }
            Transport::Rtu => {
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
                Endpoint::Rtu {
                    path,
                    baud_rate,
                    parity: self.parity.state.get_value().to_config(),
                    data_bits: Some(self.data_bits.state.get_value().0),
                    stop_bits: Some(self.stop_bits.state.get_value().0),
                }
            }
            Transport::Ascii => {
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
                Endpoint::Ascii {
                    path,
                    baud_rate,
                    parity: self.parity.state.get_value().to_config(),
                    data_bits: Some(self.data_bits.state.get_value().0),
                    stop_bits: Some(self.stop_bits.state.get_value().0),
                }
            }
        };
        let parse_ms = |input: &str, label: &str| -> Result<Option<usize>, String> {
            let t = input.trim();
            if t.is_empty() {
                Ok(None)
            } else {
                t.parse::<usize>()
                    .map(Some)
                    .map_err(|_| format!("{label} must be a whole number of milliseconds."))
            }
        };
        // Timing and explicit read ranges are shown and captured for all roles.
        let opt = |s: &str| {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        };
        let timeout_ms = parse_ms(self.timeout.state.input(), "Timeout")?;
        let delay_ms = parse_ms(self.delay.state.input(), "Delay")?;
        let interval_ms = parse_ms(self.interval.state.input(), "Interval")?;
        // Reconnect is shown for every role: it governs client redial (MB-R-050–055) and, since
        // the shared backoff driver, server bind/serial-open/mid-serve retry (MB-R-130–134).
        let reconnect = Some(self.reconnect.state.get_value() == ReconnectChoice::On);
        let read_ranges = ReadRanges {
            holding: opt(self.holding_ranges.state.input()),
            input: opt(self.input_ranges.state.input()),
            coils: opt(self.coil_ranges.state.input()),
            discrete: opt(self.discrete_ranges.state.input()),
        };

        // TLS is hidden entirely for RTU (MB-R-112) and Ascii (MB-R-121): report no value at
        // all, so a save on an RTU/Ascii instance never clobbers a device config's existing
        // `tls` setting — neither carries a `tls` field. RtuOverTcp carries TLS exactly like
        // Tcp (MB-R-115), and so does AsciiOverTcp (MB-R-127).
        let tls = if matches!(
            endpoint,
            Endpoint::Tcp { .. } | Endpoint::RtuOverTcp { .. } | Endpoint::AsciiOverTcp { .. }
        ) {
            let level = self.tls_level.state.get_value();
            let mut cfg = if level == TlsLevel::Off {
                ModbusTlsConfig::default()
            } else {
                // MB-R-135/136/139: `build_config` resolves the active role's policy directly
                // from the raw text together with the toggle widgets (self_signed/skip_verify/
                // client_cert_skip_verify), so a toggle excludes stale text from the resolved
                // config rather than layering a flag on top of whatever raw text happened to be
                // present. `extract()` reads every field's raw text/toggle state uniformly,
                // regardless of role/level — it never consults `self.tls`'s own `role`/`level`
                // (those only gate which fields are focusable/visible, checked separately by
                // `render`/`handle_events`), so no `sync` call is needed on this read-only path.
                let extracted = self.tls.extract();
                level.build_config(
                    role,
                    TlsInputs {
                        cert_file: &extracted.cert_file,
                        key_file: &extracted.key_file,
                        client_cert_file: &extracted.client_cert_file,
                        client_key_file: &extracted.client_key_file,
                        ca_files: &extracted.ca_files,
                        self_signed: extracted.self_signed,
                        skip_verify: extracted.skip_verify,
                        client_cert_skip_verify: extracted.client_cert_skip_verify,
                        root_store: extracted.root_store,
                    },
                )?
            };
            // Stitch the inactive role's half back in from the original config (if any), so a
            // role toggle preserves the other role's previously-saved TLS settings instead of
            // resetting them to `ModbusTlsConfig::default()`'s placeholder. Applies at every
            // level, `Off` included (MB-R-104's two-role container must survive a save at `Off`
            // too), so both branches above hand back a bare `cfg` and share this one stitch.
            if let Some(orig) = &self.original_tls {
                match role {
                    ClientOrServer::Server => cfg.client = orig.client.clone(),
                    ClientOrServer::Client => cfg.server = orig.server.clone(),
                }
            }
            if level != TlsLevel::Off {
                validate_tls(&cfg, role, level, &|p| {
                    ferrowl_util::path::expand(p).exists()
                })?;
            }
            Some(cfg)
        } else {
            None
        };

        Ok(SetupValues {
            name,
            config_path,
            role: role.into(),
            endpoint,
            timeout_ms,
            delay_ms,
            interval_ms,
            reconnect,
            read_ranges,
            tls,
        })
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.sync_tls();

        // Reflect validation state in the error field.
        match self.resolve() {
            Ok(_) => self.error.state.clear(),
            Err(e) => self.error.state = e,
        }

        let is_new = self.mode == DialogMode::New;
        // Deliberately not `!= Transport::Tcp`: RtuOverTcp and AsciiOverTcp use the 1-row
        // TCP-shaped ip/port layout, not RTU's/Ascii's 2-row serial layout, so both must stay
        // excluded from `is_rtu` here, while Ascii must be included alongside Rtu.
        let is_rtu = matches!(
            self.transport.state.get_value(),
            Transport::Rtu | Transport::Ascii
        );
        let is_udp = matches!(self.transport.state.get_value(), Transport::Udp);
        // RTU needs two endpoint rows (path/baud, parity/data-bits/stop-bits); TCP one.
        let endpoint_rows: u16 = if is_rtu { 2 } else { 1 };
        let show_tls = self.tls_shown();
        // Fixed 4-slot TLS row order (both roles): Self-Signed, own-identity cert/key pair,
        // Skip Verify, peer-verification input — each flag already folds in `tls_shown()`, so a
        // row's absence (e.g. the client's Self-Signed row outside mTLS) simply isn't budgeted.
        let show_self_signed_row = self.tls.show_self_signed_row();
        let show_identity_row = self.tls.show_identity_row();
        let show_skip_verify_row = self.tls.show_skip_verify_row();
        let show_peer_verify_row = self.tls.show_peer_verify_row();
        let tls_rows: u16 = show_self_signed_row as u16
            + show_identity_row as u16
            + show_skip_verify_row as u16
            + show_peer_verify_row as u16;
        // border(2) + inner margin(2) + name(3) + device(3) + select(3) + endpoint + timing(3) + ranges(6)
        // + error(4) + keybinds(1) + optional config-path row (New mode) + optional TLS rows.
        let box_height = 27 + endpoint_rows * 3 + tls_rows * 3;

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
        let title = if is_new { "New Module" } else { "Module Setup" };
        let block = Block::bordered()
            .style(
                ratatui::style::Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg),
            )
            .title_alignment(HorizontalAlignment::Center)
            .title(title);
        let block_inner = block.inner(vcenter);
        let inner = block_inner.inner(Margin::new(2, 1));
        ratatui::prelude::Widget::render(&ratatui::widgets::Clear, vcenter, buf);
        block.render(vcenter, buf);

        let mut constraints = vec![
            Constraint::Length(3),                 // name
            Constraint::Length(3),                 // config path
            Constraint::Length(3),                 // transport + role
            Constraint::Length(endpoint_rows * 3), // endpoint
            Constraint::Length(3),                 // timeout + delay + interval
            Constraint::Length(3),                 // holding + input ranges
            Constraint::Length(3),                 // coil + discrete ranges
        ];
        if show_self_signed_row {
            constraints.push(Constraint::Length(3)); // Self-Signed
        }
        if show_identity_row {
            constraints.push(Constraint::Length(3)); // own-identity cert/key pair
        }
        if show_skip_verify_row {
            constraints.push(Constraint::Length(3)); // Skip Verify
        }
        if show_peer_verify_row {
            constraints.push(Constraint::Length(3)); // peer-verification input (CA file, or client-CA list)
        }
        constraints.push(Constraint::Length(4)); // error
        constraints.push(Constraint::Length(1)); // keybinds
        let rows = Layout::vertical(constraints).split(inner);

        let mut idx = 0;
        render_field!(self, name, rows[idx], buf);
        idx += 1;

        render_field!(self, config_path, rows[idx], buf);
        idx += 1;

        if is_rtu || is_udp {
            render_row!(self, rows[idx], buf; transport, role);
        } else {
            render_row!(self, rows[idx], buf;
                transport => Constraint::Percentage(40),
                tls_level => Constraint::Percentage(20),
                role => Constraint::Percentage(40)
            );
        }
        idx += 1;

        if show_tls {
            let is_server = self.role.state.get_value() == ClientOrServer::Server;
            // Rebinding so `render_field!`/`render_row!` see a bare ident bound to `TlsSection`
            // instead of literally `self` — the macros only need `.field.widget`/`.field.state`
            // on whatever they're given, not `self` specifically.
            let tls = &mut self.tls;

            // Row 1: Self-Signed (both roles).
            if show_self_signed_row {
                render_field!(tls, self_signed, rows[idx], buf);
                idx += 1;
            }

            // Row 2: own-identity cert/key pair.
            if show_identity_row {
                if is_server {
                    render_row!(tls, rows[idx], buf; cert_file, key_file);
                } else {
                    render_row!(tls, rows[idx], buf; client_cert_file, client_key_file);
                }
                idx += 1;
            }

            // Row 3: Skip Verify -- client role folds the Root Store toggle onto the same row
            // (MB-R-156) rather than adding a fifth row to a 60-column dialog.
            if show_skip_verify_row {
                if is_server {
                    render_field!(tls, client_cert_skip_verify, rows[idx], buf);
                } else if tls.show_root_store_row() {
                    render_row!(tls, rows[idx], buf;
                        skip_verify => Constraint::Percentage(50),
                        root_store => Constraint::Fill(1)
                    );
                } else {
                    render_field!(tls, skip_verify, rows[idx], buf);
                }
                idx += 1;
            }

            // Row 4: peer-verification input -- the shared CA list, for both roles (MB-R-136/
            // MB-R-156).
            if show_peer_verify_row {
                // No CA entries yet: give ADD the row's full remaining width and skip DEL
                // entirely rather than paint an empty, nothing-to-delete button.
                if tls.ca_files.state.values().is_empty() {
                    // Hidden button shouldn't be focused.
                    if tls.focus() == TlsSectionFocus::CaDeleteButton {
                        tls.focus_previous();
                    }
                    render_row!(tls, rows[idx], buf;
                        ca_files => Constraint::Percentage(80),
                        ca_add_button => Constraint::Fill(1)
                    );
                } else {
                    render_row!(tls, rows[idx], buf;
                        ca_files => Constraint::Percentage(60),
                        ca_add_button => Constraint::Percentage(20),
                        ca_delete_button => Constraint::Fill(1)
                    );
                }
                idx += 1;
            }
        }

        let endpoint_area = rows[idx];
        idx += 1;
        if is_rtu {
            let [row0, row1] = Layout::vertical([Constraint::Length(3), Constraint::Length(3)])
                .areas(endpoint_area);
            render_row!(self, row0, buf; path, baud, reconnect);
            render_row!(self, row1, buf;
                parity => Constraint::Percentage(35),
                data_bits => Constraint::Percentage(30),
                stop_bits => Constraint::Percentage(35)
            );
        } else {
            render_row!(self, endpoint_area, buf; ip, port, reconnect);
        }

        render_row!(self, rows[idx], buf; timeout, delay, interval);
        idx += 1;

        render_row!(self, rows[idx], buf; holding_ranges, input_ranges);
        idx += 1;
        render_row!(self, rows[idx], buf; coil_ranges, discrete_ranges);
        idx += 1;

        let error_area = rows[idx];
        idx += 1;
        if !self.error.state.is_empty() {
            render_field!(self, error, error_area, buf);
        }

        render_field!(self, keybinds, rows[idx], buf);

        // Suggestion popups draw last, over everything else in the dialog (and may overflow
        // the dialog box itself), so both must be rendered after all sibling widgets above.
        self.config_path
            .widget
            .render_overlay(area, buf, &mut self.config_path.state);
        self.path
            .widget
            .render_overlay(area, buf, &mut self.path.state);
        {
            let tls = &mut self.tls;
            tls.cert_file
                .widget
                .render_overlay(area, buf, &mut tls.cert_file.state);
            tls.key_file
                .widget
                .render_overlay(area, buf, &mut tls.key_file.state);
            tls.client_cert_file
                .widget
                .render_overlay(area, buf, &mut tls.client_cert_file.state);
            tls.client_key_file
                .widget
                .render_overlay(area, buf, &mut tls.client_key_file.state);
            // `ca_files` is a `Selection`, not a `SuggestInput` — no completion overlay.

            if let Some(d) = tls.ca_add_dialog.as_mut() {
                d.render(area, buf);
            }
        }

        if let Some(d) = self.close_confirm.as_mut() {
            d.render(vcenter, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_test_support::reserve_temp_dir;
    use ferrowl_ui::traits::{IsFocus, SetFocus};

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

    #[test]
    /// NF-R-054 — a `~/...` config path validates the same way `resolve()` will later load it.
    fn ut_config_path_validate_expands_tilde() {
        let home = std::env::home_dir().expect("HOME must resolve in test environment");
        let name = format!("ferrowl_modbus_setup_tilde_cfg_{}.toml", std::process::id());
        ferrowl_util::convert::Converter::save(
            &DeviceConfig::default(),
            home.join(&name).to_str().unwrap(),
            FileType::Toml,
        )
        .unwrap();

        let result = ConfigPath::validate(&format!("~/{name}"));
        let _ = std::fs::remove_file(home.join(&name));

        assert!(matches!(result, ValidateResult::Success));
    }

    #[test]
    /// NF-R-054 — `resolve()`'s config-path existence gate expands a leading `~`.
    fn ut_resolve_config_path_tilde_loads_device() {
        let home = std::env::home_dir().expect("HOME must resolve in test environment");
        let name = format!(
            "ferrowl_modbus_setup_tilde_resolve_{}.toml",
            std::process::id()
        );
        // A distinguishing (non-default) marker: if `resolve()`'s exists-check gate fails to
        // expand `~`, it falls back to the "path doesn't exist, use a default device" branch,
        // which would also produce `Some(...)` -- so a plain `is_some()` assertion wouldn't
        // actually prove the file was loaded. Asserting the marker survived does.
        let saved = DeviceConfig {
            timeout_ms: Some(12345),
            ..DeviceConfig::default()
        };
        ferrowl_util::convert::Converter::save(
            &saved,
            home.join(&name).to_str().unwrap(),
            FileType::Toml,
        )
        .unwrap();

        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        set_suggest_input(&mut dialog.config_path, &format!("~/{name}"));

        let outcome = dialog.resolve();
        let _ = std::fs::remove_file(home.join(&name));

        let outcome = outcome.expect("a valid ~/-prefixed config path must resolve");
        let (_, device) = outcome.device.expect("New mode always sets device");
        assert_eq!(device.timeout_ms, Some(12345));
    }

    #[test]
    /// NF-R-054 — `validate_tls`'s existence check (wired via `resolve()`) expands a leading `~`
    /// in cert/key paths, so a valid `~/...` path validates the same way TLS material loading
    /// will.
    fn ut_resolve_tls_cert_key_tilde_paths_validate() {
        let home = std::env::home_dir().expect("HOME must resolve in test environment");
        let cert_name = format!("ferrowl_modbus_setup_tilde_{}.crt", std::process::id());
        let key_name = format!("ferrowl_modbus_setup_tilde_{}.key", std::process::id());
        std::fs::write(home.join(&cert_name), b"").unwrap();
        std::fs::write(home.join(&key_name), b"").unwrap();

        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut dialog.tls.cert_file, &format!("~/{cert_name}"));
        set_suggest_input(&mut dialog.tls.key_file, &format!("~/{key_name}"));

        let outcome = dialog.resolve();
        let _ = std::fs::remove_file(home.join(&cert_name));
        let _ = std::fs::remove_file(home.join(&key_name));

        outcome.expect("a valid ~/-prefixed cert/key path must validate");
    }

    /// Typing into the config-path field opens the filesystem suggestion popup, and the
    /// popup is drawn on top of the dialog by the trailing `render_overlay` calls in `render`.
    #[test]
    /// UI-R-026 — the config-path field shows a completion popup.
    fn ut_render_config_path_field_shows_suggestion_popup() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        dialog.config_path.state.set_focused(true);
        dialog
            .config_path
            .state
            .handle_events(KeyModifiers::NONE, KeyCode::Char('s'));
        assert!(dialog.config_path.state.suggestions_open());

        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(text.contains("src"), "missing suggestion popup:\n{text}");
    }

    /// UI-R-067, UI-R-194 — the `:edit` open path establishes the same single-focus state as
    /// `create`, including the nested TLS section opening unfocused.
    /// Compared against the `Focus` derive's own normalisation rather than a hand-listed field
    /// set, so a field added later is covered as soon as it renders.
    #[test]
    fn ut_edit_matches_the_derive_normalised_focus_state() {
        let timing = Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        };
        // Two fixtures, because no single one renders every conditional field: the serial fields
        // are gated on an Rtu/Ascii transport, while the TLS section needs a TCP-family transport
        // at mTLS. Buffer equality is blind to any field that is not painted, so a fixture that
        // hides a field proves nothing about its focus flag.
        let serial = Endpoint::Rtu {
            path: "/dev/ttyS0".to_string(),
            baud_rate: 9600,
            parity: None,
            data_bits: Some(8),
            stop_bits: Some(1),
        };
        let tcp = Endpoint::Tcp {
            ip: "127.0.0.1".to_string(),
            port: 502,
        };
        // The widest client-role shape: verifying, explicit-identity mTLS renders the CA-file
        // input and the client cert/key pair, which SkipVerify/SelfSigned would hide.
        let tls = ModbusTlsConfig {
            client: ferrowl_util::tls::ClientTlsPolicy::Mutual {
                verification: ferrowl_util::tls::CertVerification::RootStore {
                    extra_ca_files: vec!["ca.pem".to_string()],
                },
                identity: ferrowl_util::tls::CertSource::Files {
                    cert_file: "client.crt".to_string(),
                    key_file: "client.key".to_string(),
                },
            },
            ..ModbusTlsConfig::default()
        };
        let area = Rect::new(0, 0, 100, 60);

        for (label, endpoint, tls) in [("serial", &serial, None), ("tcp+mtls", &tcp, Some(&tls))] {
            let build = || {
                SetupDialog::edit(
                    "dev",
                    "",
                    ClientOrServer::Client,
                    endpoint,
                    timing,
                    &ReadRanges::default(),
                    tls,
                )
            };

            let mut as_built = build();
            assert!(
                as_built.name.state.is_focused(),
                "{label}: name must open focused"
            );
            assert!(
                !as_built.tls.is_focused(),
                "{label}: the nested TLS section must open unfocused"
            );
            let mut built = Buffer::empty(area);
            as_built.render(area, &mut built);

            let mut normalised = build();
            normalised.set_focused(true);
            let mut after = Buffer::empty(area);
            normalised.render(area, &mut after);

            assert_eq!(
                built, after,
                "{label}: edit-path focus state differs from the derive's normalised state"
            );
        }
    }

    /// UI-R-067, UI-R-195 — a freshly created dialog focuses exactly one field (`name`, first in
    /// the Tab cycle) and paints exactly one text cursor for it. The cursor rather than the border,
    /// because `name` opens empty against `NonEmpty` and so paints its error border, not its
    /// focused one — border color alone would not identify the focused field.
    #[test]
    fn ut_create_focuses_name_only_and_paints_one_cursor() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        assert!(dialog.name.state.is_focused(), "name must open focused");
        assert_eq!(
            dialog.focus,
            SetupDialogFocus::Name,
            "the focus cursor must name the field carrying the flag"
        );
        assert!(
            !dialog.tls.is_focused(),
            "the nested TLS section must open unfocused"
        );

        // `focus == Name` alone survives a reordering that moved `Name` out of first position;
        // stepping back from it must wrap away, and forward from there must wrap to it.
        let mut walked = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        walked.focus_previous();
        assert_ne!(walked.focus, SetupDialogFocus::Name);
        walked.focus_next();
        assert_eq!(walked.focus, SetupDialogFocus::Name);
        for (label, focused) in [
            ("config_path", dialog.config_path.state.is_focused()),
            ("transport", dialog.transport.state.is_focused()),
            ("tls_level", dialog.tls_level.state.is_focused()),
            ("role", dialog.role.state.is_focused()),
            ("ip", dialog.ip.state.is_focused()),
            ("port", dialog.port.state.is_focused()),
            ("path", dialog.path.state.is_focused()),
            ("baud", dialog.baud.state.is_focused()),
            ("reconnect", dialog.reconnect.state.is_focused()),
            ("parity", dialog.parity.state.is_focused()),
            ("data_bits", dialog.data_bits.state.is_focused()),
            ("stop_bits", dialog.stop_bits.state.is_focused()),
            ("timeout", dialog.timeout.state.is_focused()),
            ("delay", dialog.delay.state.is_focused()),
            ("interval", dialog.interval.state.is_focused()),
            ("holding_ranges", dialog.holding_ranges.state.is_focused()),
            ("input_ranges", dialog.input_ranges.state.is_focused()),
            ("coil_ranges", dialog.coil_ranges.state.is_focused()),
            ("discrete_ranges", dialog.discrete_ranges.state.is_focused()),
            ("tls.self_signed", dialog.tls.self_signed.state.is_focused()),
            ("tls.cert_file", dialog.tls.cert_file.state.is_focused()),
            ("tls.key_file", dialog.tls.key_file.state.is_focused()),
            (
                "tls.client_cert_file",
                dialog.tls.client_cert_file.state.is_focused(),
            ),
            (
                "tls.client_key_file",
                dialog.tls.client_key_file.state.is_focused(),
            ),
            (
                "tls.client_cert_skip_verify",
                dialog.tls.client_cert_skip_verify.state.is_focused(),
            ),
            ("tls.skip_verify", dialog.tls.skip_verify.state.is_focused()),
            ("tls.root_store", dialog.tls.root_store.state.is_focused()),
            ("tls.ca_files", dialog.tls.ca_files.state.is_focused()),
            (
                "tls.ca_add_button",
                dialog.tls.ca_add_button.state.is_focused(),
            ),
            (
                "tls.ca_delete_button",
                dialog.tls.ca_delete_button.state.is_focused(),
            ),
        ] {
            assert!(!focused, "{label} must open unfocused");
        }

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

    #[test]
    /// The setup dialog resolves a reconnect-off selection into the config value.
    fn ut_resolve_reconnect_off_maps_to_some_false() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.role.state.set_selection(1); // Client
        dialog.reconnect.state.set_selection(1); // Off
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.reconnect, Some(false));
    }

    #[test]
    /// MB-R-130, MB-R-134 — a server role also reports a reconnect setting: server-side
    /// reconnect (bind/serial-open/mid-serve retry) is governed by the same config field as
    /// the client's, so the dialog must resolve a value for it regardless of role.
    fn ut_resolve_server_role_reports_reconnect() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        // Default role is Server; default reconnect selection is On.
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.reconnect, Some(true));

        dialog.reconnect.state.set_selection(1); // Off
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.reconnect, Some(false));
    }

    #[test]
    /// Edit mode pre-fills the dialog from the existing config.
    fn ut_edit_prefills_reconnect_off() {
        let timing = Timing {
            timeout_ms: 100,
            delay_ms: 10,
            interval_ms: 50,
            reconnect: false,
        };
        let endpoint = Endpoint::Tcp {
            ip: "127.0.0.1".to_string(),
            port: 502,
        };
        let dialog = SetupDialog::edit(
            "dev",
            "",
            ClientOrServer::Client,
            &endpoint,
            timing,
            &ReadRanges::default(),
            None,
        );
        assert_eq!(dialog.reconnect.state.get_value(), ReconnectChoice::Off);
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.reconnect, Some(false));
    }

    #[test]
    /// A TCP setup dialog always offers the TLS level selector, but the detail
    /// section (self-signed/cert/etc.) only appears once a level above Off is actually picked
    /// — the level selector alone must never imply the rest of the section is showing.
    fn ut_tcp_dialog_shows_tls_level_selector_but_not_detail_section_at_off() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        assert_eq!(dialog.transport.state.get_value(), Transport::Tcp);
        assert_eq!(dialog.tls_level.state.get_value(), TlsLevel::Off);
        // Level selector itself: always rendered for TCP, regardless of the chosen level.
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(text.contains("TLS"), "missing TLS level selector:\n{text}");
        // Detail section: hidden at Off, so the toggle/cert rows aren't allocated or drawn.
        assert!(!dialog.tls_shown());
        assert!(
            !text.contains("Self-Signed"),
            "self-signed toggle shown at TLS level Off:\n{text}"
        );
    }

    #[test]
    /// Picking a TLS level above Off reveals the detail section (MB-R-104's fields
    /// become settable only once the user has actually opted into TLS).
    fn ut_tls_shown_once_level_selected_above_off() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        assert!(!dialog.tls_shown());
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        assert!(dialog.tls_shown());
    }

    #[test]
    /// A server that turns on Self-Signed does not need (or show) the
    /// cert/key file row; toggling it back off restores the row.
    fn ut_self_signed_hides_server_cert_row() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        // Default role is Server.
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        dialog.sync_tls();
        assert!(dialog.tls.show_identity_row());
        dialog.tls.self_signed.state.set_selection(1); // On
        assert!(!dialog.tls.show_identity_row());
        dialog.tls.self_signed.state.set_selection(0); // Off
        assert!(dialog.tls.show_identity_row());
    }

    #[test]
    /// A client that turns on Skip Verify does not need (or show) the CA-file
    /// row; toggling it back off restores the row.
    fn ut_skip_verify_hides_ca_file_row() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        dialog.role.state.set_selection(1); // Client
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        dialog.sync_tls();
        assert!(dialog.tls.show_peer_verify_row());
        dialog.tls.skip_verify.state.set_selection(1); // On
        assert!(!dialog.tls.show_peer_verify_row());
        dialog.tls.skip_verify.state.set_selection(0); // Off
        assert!(dialog.tls.show_peer_verify_row());
    }

    fn row_of(buf: &Buffer, needle: &str) -> u16 {
        let text = buffer_text(buf);
        text.lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not found in:\n{text}")) as u16
    }

    #[test]
    /// The client-CA row's ADD/DEL buttons hug the dialog's right inner edge with
    /// no trailing dead space, matching every other full-width row in the dialog. The row's own
    /// internal layout (border height, DEL visibility, row order) is `TlsSection`'s own concern,
    /// covered directly against a bare `TlsSection`; this test is specifically about the outer
    /// dialog's row placement relative to a sibling non-TLS row (`Name`).
    fn ut_ca_delete_button_hugs_right_edge() {
        let mut dialog = SetupDialog::create(default_timing());
        set_input(&mut dialog.name, "dev");
        dialog.role.state.set_selection(0); // Server
        dialog
            .tls_level
            .state
            .set_selection(TlsLevel::MutualTls.index());
        dialog
            .tls
            .ca_files
            .state
            .set_values(vec!["ca1.pem".to_string()]);
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);

        fn rightmost_non_space(buf: &Buffer, y: u16) -> u16 {
            (0..buf.area.width)
                .rev()
                .find(|&x| buf[(x, y)].symbol() != " ")
                .unwrap_or(0)
        }

        let name_row = row_of(&buf, "Name");
        let ca_row = row_of(&buf, "CA(s)");
        assert_eq!(
            rightmost_non_space(&buf, name_row),
            rightmost_non_space(&buf, ca_row),
            "DEL button leaves trailing dead space vs. the dialog's other full-width rows"
        );
    }

    #[test]
    /// MB-R-112 — an RTU setup dialog never shows the TLS section.
    fn ut_rtu_dialog_hides_tls_section() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        dialog.transport.state.set_selection(1); // Rtu
        assert_eq!(dialog.transport.state.get_value(), Transport::Rtu);
        assert!(!dialog.tls_shown());
    }

    #[test]
    /// MB-R-112 — resolving an RTU dialog reports no TLS value at all, so applying it can never
    /// clobber a device config's existing `tls` setting.
    fn ut_resolve_rtu_reports_no_tls() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.transport.state.set_selection(1); // Rtu
        set_suggest_input(&mut dialog.path, "/dev/ttyUSB0");
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.tls, None);
    }

    #[test]
    /// MB-R-113 — an RtuOverTcp setup dialog shows the same fields as TCP (ip/port,
    /// TLS level selector) and none of RTU's serial fields.
    fn ut_rtu_over_tcp_dialog_shows_tcp_like_fields() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        dialog.transport.state.set_selection(2); // Tcp=0, Rtu=1, RtuOverTcp=2
        assert_eq!(dialog.transport.state.get_value(), Transport::RtuOverTcp);
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            text.contains("IP"),
            "missing IP field for RtuOverTcp:\n{text}"
        );
        assert!(
            text.contains("TLS"),
            "missing TLS selector for RtuOverTcp:\n{text}"
        );
        assert!(
            !text.contains("Baud"),
            "RTU serial field leaked into RtuOverTcp:\n{text}"
        );
    }

    #[test]
    /// MB-R-113 — resolving an RtuOverTcp dialog produces `Endpoint::RtuOverTcp`
    /// with the entered ip/port.
    fn ut_resolve_rtu_over_tcp_endpoint() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.transport.state.set_selection(2);
        set_input(&mut dialog.ip, "10.0.0.9");
        set_input(&mut dialog.port, "1502");
        let outcome = dialog.resolve().unwrap();
        assert_eq!(
            outcome.values.endpoint,
            Endpoint::RtuOverTcp {
                ip: "10.0.0.9".into(),
                port: 1502
            }
        );
    }

    #[test]
    /// MB-R-116 — resolving a Udp dialog produces `Endpoint::Udp` with the entered ip/port,
    /// for either role.
    fn ut_resolve_udp_endpoint() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.transport.state.set_selection(3); // Tcp=0, Rtu=1, RtuOverTcp=2, Udp=3
        set_input(&mut dialog.ip, "10.0.0.9");
        set_input(&mut dialog.port, "1502");
        let outcome = dialog.resolve().unwrap();
        assert_eq!(
            outcome.values.endpoint,
            Endpoint::Udp {
                ip: "10.0.0.9".into(),
                port: 1502
            }
        );

        dialog.role.state.set_selection(1); // Role::Server=0, Role::Client=1
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.role, Role::Client);
    }

    #[test]
    /// MB-R-121 — selecting an Ascii endpoint sets the transport selector to index 4.
    fn ut_edit_ascii_sets_transport_selection() {
        let timing = Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        };
        let endpoint = Endpoint::Ascii {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            parity: None,
            data_bits: None,
            stop_bits: None,
        };
        let dialog = SetupDialog::edit(
            "dev",
            "",
            ClientOrServer::Client,
            &endpoint,
            timing,
            &ReadRanges::default(),
            None,
        );
        assert_eq!(dialog.transport.state.get_value(), Transport::Ascii);
    }

    #[test]
    /// MB-R-125 — selecting an AsciiOverTcp endpoint sets the transport selector to index 5.
    fn ut_edit_ascii_over_tcp_sets_transport_selection() {
        let timing = Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        };
        let endpoint = Endpoint::AsciiOverTcp {
            ip: "10.0.0.9".to_string(),
            port: 1502,
        };
        let dialog = SetupDialog::edit(
            "dev",
            "",
            ClientOrServer::Client,
            &endpoint,
            timing,
            &ReadRanges::default(),
            None,
        );
        assert_eq!(dialog.transport.state.get_value(), Transport::AsciiOverTcp);
    }

    #[test]
    /// MB-R-121 — resolving an Ascii dialog produces `Endpoint::Ascii` with the entered
    /// path/baud/parity/data_bits/stop_bits, mirroring the Rtu resolve test.
    fn ut_values_ascii_produces_endpoint() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.transport.state.set_selection(4); // Tcp=0, Rtu=1, RtuOverTcp=2, Udp=3, Ascii=4
        assert_eq!(dialog.transport.state.get_value(), Transport::Ascii);
        set_suggest_input(&mut dialog.path, "/dev/ttyUSB1");
        set_input(&mut dialog.baud, "9600");
        let outcome = dialog.resolve().unwrap();
        assert_eq!(
            outcome.values.endpoint,
            Endpoint::Ascii {
                path: "/dev/ttyUSB1".into(),
                baud_rate: 9600,
                parity: dialog.parity.state.get_value().to_config(),
                data_bits: Some(dialog.data_bits.state.get_value().0),
                stop_bits: Some(dialog.stop_bits.state.get_value().0),
            }
        );
    }

    #[test]
    /// MB-R-125 — resolving an AsciiOverTcp dialog produces `Endpoint::AsciiOverTcp` with the
    /// entered ip/port, mirroring the RtuOverTcp/Udp resolve tests.
    fn ut_values_ascii_over_tcp_produces_endpoint() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.transport.state.set_selection(5); // ..Udp=3, Ascii=4, AsciiOverTcp=5
        assert_eq!(dialog.transport.state.get_value(), Transport::AsciiOverTcp);
        set_input(&mut dialog.ip, "10.0.0.9");
        set_input(&mut dialog.port, "1502");
        let outcome = dialog.resolve().unwrap();
        assert_eq!(
            outcome.values.endpoint,
            Endpoint::AsciiOverTcp {
                ip: "10.0.0.9".into(),
                port: 1502
            }
        );
    }

    #[test]
    /// MB-R-127 — TLS is offered (tls_shown() true once a level is picked) for AsciiOverTcp
    /// exactly as for Tcp/RtuOverTcp, never for Ascii (MB-R-121: rtu::Config carries no tls
    /// field).
    fn ut_tls_shown_for_ascii_over_tcp_not_ascii() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        dialog.transport.state.set_selection(5); // AsciiOverTcp
        assert_eq!(dialog.transport.state.get_value(), Transport::AsciiOverTcp);
        assert!(!dialog.tls_shown());
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        assert!(dialog.tls_shown());

        dialog.transport.state.set_selection(4); // Ascii
        assert_eq!(dialog.transport.state.get_value(), Transport::Ascii);
        assert!(!dialog.tls_shown());
    }

    #[test]
    /// Resolving a TCP dialog at TLS level Off reports an explicit no-TLS setting.
    fn ut_resolve_tcp_tls_off_reports_some_none() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        let outcome = dialog.resolve().unwrap();
        assert_eq!(outcome.values.tls, Some(ModbusTlsConfig::default()));
    }

    #[test]
    /// MB-R-161 — resolving an edited server-role dialog at TLS level Off preserves the
    /// inactive (client) role's previously-saved policy, stitched from `original_tls`, instead
    /// of resetting the whole two-role container to `ModbusTlsConfig::default()`.
    fn ut_resolve_tls_off_preserves_inactive_role_policy() {
        let original = ModbusTlsConfig {
            client: ferrowl_util::tls::ClientTlsPolicy::Tls {
                verification: ferrowl_util::tls::CertVerification::Skip {},
            },
            ..Default::default()
        };
        let mut dialog = SetupDialog::edit(
            "dev",
            "device.toml",
            ClientOrServer::Server,
            &Endpoint::Tcp {
                ip: "127.0.0.1".to_string(),
                port: 0,
            },
            Timing {
                timeout_ms: 0,
                delay_ms: 0,
                interval_ms: 0,
                reconnect: true,
            },
            &ReadRanges::default(),
            Some(&original),
        );
        dialog.tls_level.state.set_selection(TlsLevel::Off.index());
        let outcome = dialog.resolve().unwrap();
        let cfg = outcome.values.tls.unwrap();
        assert_eq!(cfg.client, original.client);
        assert_eq!(cfg.server, ferrowl_util::tls::ServerTlsPolicy::None {});
    }

    #[test]
    /// Resolving a TCP dialog at TLS level Tls (server, self-signed) builds a config
    /// with `self_signed` set and drops the mTLS-only client-CA field.
    fn ut_resolve_tcp_tls_server_self_signed_builds_config() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        dialog.tls.self_signed.state.set_selection(1); // On
        *dialog.tls.ca_files.state.values_mut() = vec!["client_ca.pem".to_string()];
        let outcome = dialog.resolve().unwrap();
        let cfg = outcome.values.tls.unwrap();
        assert_eq!(
            cfg.server,
            ferrowl_util::tls::ServerTlsPolicy::Tls {
                identity: ferrowl_util::tls::CertSource::SelfSigned {}
            }
        );
    }

    #[test]
    /// MB-R-135 — toggling Self-Signed On excludes stale cert_file/key_file text from the
    /// resolved config, even though the widgets' stored text is untouched.
    fn ut_resolve_self_signed_excludes_stale_cert_key_text() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut dialog.tls.cert_file, "s.crt");
        set_suggest_input(&mut dialog.tls.key_file, "s.key");
        dialog.tls.self_signed.state.set_selection(1); // On, after the text was typed

        let outcome = dialog.resolve().unwrap();
        let cfg = outcome.values.tls.unwrap();
        assert_eq!(
            cfg.server,
            ferrowl_util::tls::ServerTlsPolicy::Tls {
                identity: ferrowl_util::tls::CertSource::SelfSigned {}
            }
        );
        // The stored text survives the toggle -- only the resolved config excludes it.
        assert_eq!(dialog.tls.cert_file.state.input(), "s.crt");
        assert_eq!(dialog.tls.key_file.state.input(), "s.key");
    }

    #[test]
    /// MB-R-135 — toggling Skip-Verify On excludes the stale CA list from the resolved config,
    /// even though the widget's stored list is untouched.
    fn ut_resolve_skip_verify_excludes_stale_ca_list() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.role.state.set_selection(1); // Role::Server=0, Role::Client=1
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        *dialog.tls.ca_files.state.values_mut() = vec!["ca.pem".to_string()];
        dialog.tls.skip_verify.state.set_selection(1); // On, after the list was populated

        let outcome = dialog.resolve().unwrap();
        let cfg = outcome.values.tls.unwrap();
        assert_eq!(
            cfg.client,
            ferrowl_util::tls::ClientTlsPolicy::Tls {
                verification: ferrowl_util::tls::CertVerification::Skip {}
            }
        );
        assert_eq!(dialog.tls.ca_files.state.values(), &["ca.pem".to_string()]);
    }

    #[test]
    /// MB-R-186 — toggling Self-Signed back Off restores the previously entered cert/key paths
    /// (nothing was cleared, only excluded while On).
    fn ut_resolve_toggle_self_signed_back_off_restores_cert_key() {
        let dir = reserve_temp_dir("ferrowl_modbus_setup");
        let cert = dir.join("s.crt");
        let key = dir.join("s.key");
        std::fs::write(&cert, b"").unwrap();
        std::fs::write(&key, b"").unwrap();
        let cert = cert.to_str().unwrap().to_string();
        let key = key.to_str().unwrap().to_string();

        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut dialog.tls.cert_file, &cert);
        set_suggest_input(&mut dialog.tls.key_file, &key);
        dialog.tls.self_signed.state.set_selection(1); // On
        dialog.tls.self_signed.state.set_selection(0); // Off again

        let outcome = dialog.resolve().unwrap();
        let cfg = outcome.values.tls.unwrap();
        assert_eq!(
            cfg.server,
            ferrowl_util::tls::ServerTlsPolicy::Tls {
                identity: ferrowl_util::tls::CertSource::Files {
                    cert_file: cert,
                    key_file: key,
                }
            }
        );
    }

    #[test]
    /// MB-R-215 — toggling the client-role Self-Signed switch back Off restores the previously
    /// entered client cert/key paths (nothing was cleared, only excluded while On).
    fn ut_resolve_client_toggle_self_signed_back_off_restores_cert_key() {
        let dir = reserve_temp_dir("ferrowl_modbus_setup");
        let cert = dir.join("c.crt");
        let key = dir.join("c.key");
        std::fs::write(&cert, b"").unwrap();
        std::fs::write(&key, b"").unwrap();
        let cert = cert.to_str().unwrap().to_string();
        let key = key.to_str().unwrap().to_string();

        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        set_input(&mut dialog.name, "dev");
        dialog.role.state.set_selection(1); // Role::Server=0, Role::Client=1
        dialog
            .tls_level
            .state
            .set_selection(TlsLevel::MutualTls.index());
        set_suggest_input(&mut dialog.tls.client_cert_file, &cert);
        set_suggest_input(&mut dialog.tls.client_key_file, &key);
        dialog.tls.self_signed.state.set_selection(1); // On
        dialog.tls.self_signed.state.set_selection(0); // Off again

        let outcome = dialog.resolve().unwrap();
        let cfg = outcome.values.tls.unwrap();
        assert_eq!(
            cfg.client,
            ferrowl_util::tls::ClientTlsPolicy::Mutual {
                verification: ferrowl_util::tls::CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
                identity: ferrowl_util::tls::CertSource::Files {
                    cert_file: cert,
                    key_file: key,
                }
            }
        );
    }

    #[test]
    /// UI-R-022 — the focus cycle visits the reconnect field for every role, since server-side
    /// reconnect (MB-R-130–134) makes it applicable regardless of role.
    fn ut_focus_next_reaches_reconnect_for_server_role() {
        let mut dialog = SetupDialog::create(Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        });
        // Default role is Server, and default transport is Tcp, so `baud` is gated off and
        // traversal moves straight from `port` to `reconnect`.
        dialog.focus = SetupDialogFocus::Port;
        dialog.port.state.set_focused(true);
        dialog.focus_next();
        assert!(dialog.reconnect.state.is_focused());
    }

    /// One step of the flattened forward Tab walk used by the tab-order tests below: while
    /// `self.focus` sits on the nested `Tls` field, Tab steps *within* `TlsSection`'s own panes
    /// (dispatched to its `HandleEvents`, which tries `NestedFocus::try_focus_next` on an
    /// `Unhandled` Tab/BackTab); once that inner scan is exhausted, the outer struct's own
    /// `focus_next()` advances to the next top-level field. The outer wrap-around walk alone only
    /// ever treats `tls` as a single field — both mechanisms together walk the full sequence.
    #[derive(Debug, Clone, PartialEq)]
    enum Stop {
        Outer(SetupDialogFocus),
        Tls(TlsSectionFocus),
    }

    fn current_stop(dialog: &SetupDialog) -> Stop {
        if dialog.focus == SetupDialogFocus::Tls {
            Stop::Tls(dialog.tls.focus())
        } else {
            Stop::Outer(dialog.focus)
        }
    }

    /// Drive a flattened forward Tab walk from `SetupDialogFocus::Role` up to and including
    /// `SetupDialogFocus::Ip`, recording every stop (outer field, or a pane inside `tls`).
    fn tab_sequence_from_role(dialog: &mut SetupDialog) -> Vec<Stop> {
        // Production always runs `handle_events()` (which calls `sync_tls()` first) before any
        // Tab/BackTab reaches `focus_next()`/`focus_previous()`, so `self.tls`'s role/level are
        // always fresh. A test driving `focus_next()` directly must reproduce that precondition:
        // unsynced, `self.tls`'s role/level default to `Server`/`Off`, making every pane
        // ineligible, so `focus_next()` would skip straight past `tls` as if it weren't there.
        dialog.sync_tls();
        dialog.focus = SetupDialogFocus::Role;
        let mut seq = vec![current_stop(dialog)];
        loop {
            if dialog.focus == SetupDialogFocus::Tls {
                let result = dialog.handle_events(KeyModifiers::NONE, KeyCode::Tab);
                if matches!(result, EventResult::Consumed) {
                    seq.push(current_stop(dialog));
                    continue;
                }
                // `TlsSection`'s own bounded scan is exhausted (already at its last eligible
                // pane) — the outer struct's own wrap-around walk advances past it.
                dialog.focus_next();
            } else {
                dialog.focus_next();
            }
            seq.push(current_stop(dialog));
            if dialog.focus == SetupDialogFocus::Ip {
                break;
            }
        }
        seq
    }

    /// Backward counterpart of [`tab_sequence_from_role`], driven with `SHIFT`+`BackTab` from
    /// `SetupDialogFocus::Ip` down to and including `SetupDialogFocus::Role`. Entering `tls`
    /// *backward* lands on its *last* eligible pane, not its first.
    fn back_tab_sequence_from_ip(dialog: &mut SetupDialog) -> Vec<Stop> {
        dialog.sync_tls(); // see `tab_sequence_from_role`'s comment on why this must run first
        dialog.focus = SetupDialogFocus::Ip;
        let mut seq = vec![current_stop(dialog)];
        loop {
            if dialog.focus == SetupDialogFocus::Tls {
                let result = dialog.handle_events(KeyModifiers::SHIFT, KeyCode::BackTab);
                if matches!(result, EventResult::Consumed) {
                    seq.push(current_stop(dialog));
                    continue;
                }
                dialog.focus_previous();
            } else {
                dialog.focus_previous();
            }
            seq.push(current_stop(dialog));
            if dialog.focus == SetupDialogFocus::Role {
                break;
            }
        }
        seq
    }

    #[test]
    /// MB-R-136, UI-R-049 — the mTLS server-role Tab order matches the dialog's pre-migration
    /// visual row order: Role, Self-Signed, own cert/key, Skip Verify, then the client-CA list
    /// and its ADD/DEL buttons, then IP — now reached by bubbling into/out of the nested `tls`
    /// field rather than a flat per-field sequence.
    fn ut_tab_order_server_mtls() {
        let mut dialog = SetupDialog::create(default_timing());
        dialog.role.state.set_selection(0); // Server
        dialog
            .tls_level
            .state
            .set_selection(TlsLevel::MutualTls.index());
        dialog
            .tls
            .ca_files
            .state
            .set_values(vec!["ca1.pem".to_string()]); // non-empty, so DEL is eligible
        let seq = tab_sequence_from_role(&mut dialog);
        assert_eq!(
            seq,
            vec![
                Stop::Outer(SetupDialogFocus::Role),
                Stop::Tls(TlsSectionFocus::SelfSigned),
                Stop::Tls(TlsSectionFocus::CertFile),
                Stop::Tls(TlsSectionFocus::KeyFile),
                Stop::Tls(TlsSectionFocus::ClientCertSkipVerify),
                Stop::Tls(TlsSectionFocus::CaFiles),
                Stop::Tls(TlsSectionFocus::CaAddButton),
                Stop::Tls(TlsSectionFocus::CaDeleteButton),
                Stop::Outer(SetupDialogFocus::Ip),
            ]
        );

        // BackTab from IP must land on the *last* pane (DEL), not the first (Self-Signed).
        let back_seq = back_tab_sequence_from_ip(&mut dialog);
        assert_eq!(
            back_seq,
            vec![
                Stop::Outer(SetupDialogFocus::Ip),
                Stop::Tls(TlsSectionFocus::CaDeleteButton),
                Stop::Tls(TlsSectionFocus::CaAddButton),
                Stop::Tls(TlsSectionFocus::CaFiles),
                Stop::Tls(TlsSectionFocus::ClientCertSkipVerify),
                Stop::Tls(TlsSectionFocus::KeyFile),
                Stop::Tls(TlsSectionFocus::CertFile),
                Stop::Tls(TlsSectionFocus::SelfSigned),
                Stop::Outer(SetupDialogFocus::Role),
            ]
        );
    }

    #[test]
    /// MB-R-136, MB-R-156, UI-R-049 — the mTLS client-role Tab order: Role, Self-Signed, own
    /// cert/key, Skip Verify, Root Store, then the shared CA list's ADD button (empty list, no
    /// DEL), then IP.
    fn ut_tab_order_client_mtls() {
        let mut dialog = SetupDialog::create(default_timing());
        dialog.role.state.set_selection(1); // Client
        dialog
            .tls_level
            .state
            .set_selection(TlsLevel::MutualTls.index());
        let seq = tab_sequence_from_role(&mut dialog);
        assert_eq!(
            seq,
            vec![
                Stop::Outer(SetupDialogFocus::Role),
                Stop::Tls(TlsSectionFocus::SelfSigned),
                Stop::Tls(TlsSectionFocus::ClientCertFile),
                Stop::Tls(TlsSectionFocus::ClientKeyFile),
                Stop::Tls(TlsSectionFocus::SkipVerify),
                Stop::Tls(TlsSectionFocus::RootStore),
                Stop::Tls(TlsSectionFocus::CaAddButton),
                Stop::Outer(SetupDialogFocus::Ip),
            ]
        );
    }

    /// MB-R-171 — with `self_signed` off, a blank `cert_file`/`key_file` refuses to resolve
    /// (submission-blocking), distinct from MB-R-106's own resolve-time self-signed fallback for
    /// a config file loaded outside the dialog.
    #[test]
    fn ut_resolve_tls_blank_cert_key_without_self_signed_fails() {
        let mut dialog = SetupDialog::create(default_timing());
        set_input(&mut dialog.name, "dev");
        dialog.tls_level.state.set_selection(TlsLevel::Tls.index());
        // self_signed stays at its Off default; cert_file/key_file stay blank.
        let err = match dialog.resolve() {
            Ok(_) => panic!("blank cert/key with self_signed off must not resolve"),
            Err(e) => e,
        };
        assert!(
            err.contains("Certificate file is required"),
            "unexpected error: {err}"
        );
    }

    /// Steps `dialog.tls.focus_next()` until it lands on `target`, bounded at `TlsSection`'s own
    /// field count so a `target` that's ineligible under the caller's role/level setup panics
    /// immediately instead of spinning forever.
    fn focus_tls_until(dialog: &mut SetupDialog, target: TlsSectionFocus) {
        for _ in 0..11 {
            if dialog.tls.focus() == target {
                return;
            }
            dialog.tls.focus_next();
        }
        panic!("{target:?} never became eligible under the current role/level setup");
    }

    /// MB-R-136 — the setup dialog's own ADD/DEL routing (`handle_events`, not `TlsSection`'s
    /// inherent method of the same name, which this dialog doesn't call): ADD opens the sub-
    /// dialog and appends a confirmed path, DEL removes the selected entry, and draining to
    /// empty falls focus back to ADD.
    #[test]
    fn ut_client_ca_add_delete_lifecycle_via_outer_dialog() {
        let dir = reserve_temp_dir("ferrowl_modbus_setup");
        let ca = {
            let path = dir.join("ca.pem");
            std::fs::write(&path, b"").unwrap();
            path.to_str().unwrap().to_string()
        };
        let mut dialog = SetupDialog::create(default_timing());
        dialog.role.state.set_selection(0); // Server: client-CA row is server-only
        dialog
            .tls_level
            .state
            .set_selection(TlsLevel::MutualTls.index());
        dialog.tls.client_cert_skip_verify.state.set_selection(0); // Off: client-CA row shows
        dialog.sync_tls();
        focus_tls_until(&mut dialog, TlsSectionFocus::CaAddButton);
        dialog.focus = SetupDialogFocus::Tls;

        dialog.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(dialog.tls.ca_add_dialog.is_some());
        set_suggest_input(&mut dialog.tls.ca_add_dialog.as_mut().unwrap().path, &ca);
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(dialog.tls.ca_add_dialog.is_none());
        assert_eq!(
            dialog.tls.ca_files.state.values(),
            std::slice::from_ref(&ca)
        );

        focus_tls_until(&mut dialog, TlsSectionFocus::CaDeleteButton);
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(dialog.tls.ca_files.state.values().is_empty());
        assert_ne!(
            dialog.tls.focus(),
            TlsSectionFocus::CaDeleteButton,
            "draining the list must not strand focus on the now-hidden DEL button"
        );
    }

    /// MB-R-136 — an empty client-CA list hides the DEL button (ADD alone takes the full row)
    /// and moves focus off it if it still holds it, exercised through the outer dialog's own
    /// render. DEL is only reachable via focus-stepping while the list is non-empty (its own
    /// `when` gate excludes it otherwise), so this seeds one entry, focuses DEL, then drains the
    /// list directly to reproduce "focus still on DEL, list now empty" without going through the
    /// button handler.
    #[test]
    fn ut_render_client_ca_empty_list_hides_delete_button_outer() {
        let mut dialog = SetupDialog::create(default_timing());
        dialog.role.state.set_selection(0); // Server: client-CA row is server-only
        dialog
            .tls_level
            .state
            .set_selection(TlsLevel::MutualTls.index());
        dialog.tls.client_cert_skip_verify.state.set_selection(0);
        dialog.sync_tls();
        dialog
            .tls
            .ca_files
            .state
            .values_mut()
            .push("placeholder".to_string());
        focus_tls_until(&mut dialog, TlsSectionFocus::CaDeleteButton);
        dialog.tls.ca_files.state.values_mut().clear();

        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains("DEL"), "empty list must hide DEL:\n{text}");
        assert_ne!(
            dialog.tls.focus(),
            TlsSectionFocus::CaDeleteButton,
            "render must move focus off the now-hidden DEL button"
        );
    }

    fn default_timing() -> Timing {
        Timing {
            timeout_ms: 0,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        }
    }

    #[test]
    /// UI-R-023 — Esc-then-Enter sets the close request, which clears after being taken.
    fn ut_take_close_request_set_via_esc_enter_and_cleared_after_take() {
        let mut dialog = SetupDialog::create(default_timing());
        assert!(!dialog.take_close_request());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_some());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(dialog.take_close_request());
        assert!(!dialog.take_close_request(), "flag must clear after take");
    }

    #[test]
    /// UI-R-014 — `:` types into a setup text field rather than entering command mode.
    fn ut_colon_in_text_input_types() {
        let mut dialog = SetupDialog::create(default_timing());
        // Default focus is Name, a free-text field; `:` is typed as ordinary text.
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Char(':'));
        assert_eq!(dialog.name.state.input(), ":");
    }

    #[test]
    /// UI-R-023 — Esc in the close-confirm keeps the setup dialog open.
    fn ut_esc_in_confirm_keeps_setup_open() {
        let mut dialog = SetupDialog::create(default_timing());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_some());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_none());
        assert!(!dialog.take_close_request());
    }

    #[test]
    /// UI-R-023 — Space in the close-confirm closes the setup dialog.
    fn ut_space_in_confirm_closes() {
        let mut dialog = SetupDialog::create(default_timing());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_some());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Char(' '));
        assert!(dialog.take_close_request());
    }

    #[test]
    /// UI-R-111 — a fresh dialog's `name` field shows a focused field with an error border
    /// (empty against `NonEmpty`); `:edit`'s prefilled `name` field is focused and valid, so it
    /// shows the plain focused border instead.
    fn ut_name_border_is_error_when_fresh_and_focused_when_prefilled() {
        let style = InputFieldStyle::default();
        let error = style.error().fg.unwrap();
        let focused = style.focused().fg.unwrap();
        assert_ne!(
            error, focused,
            "the two border colors must be distinguishable"
        );

        let name_corner = |buf: &Buffer| -> (u16, u16) {
            let mut corners: Vec<(u16, u16)> = Vec::new();
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    if buf[(x, y)].symbol() == "┌" {
                        corners.push((x, y));
                    }
                }
            }
            corners.sort_by_key(|(x, y)| (*y, *x));
            assert!(
                corners.len() >= 2,
                "expected at least the dialog's own box and the name field's box"
            );
            corners[1]
        };

        let area = Rect::new(0, 0, 100, 60);

        let mut fresh = SetupDialog::create(default_timing());
        assert!(
            fresh.name.state.is_focused(),
            "fresh dialog: name must open focused"
        );
        let mut fresh_buf = Buffer::empty(area);
        fresh.render(area, &mut fresh_buf);
        let corner = name_corner(&fresh_buf);
        assert_eq!(
            fresh_buf[corner].fg, error,
            "fresh dialog: empty NonEmpty name paints the error border"
        );

        let endpoint = Endpoint::Tcp {
            ip: "127.0.0.1".to_string(),
            port: 502,
        };
        let mut edit = SetupDialog::edit(
            "dev",
            "",
            ClientOrServer::Client,
            &endpoint,
            default_timing(),
            &ReadRanges::default(),
            None,
        );
        assert!(
            edit.name.state.is_focused(),
            "edit dialog: name must open focused"
        );
        let mut edit_buf = Buffer::empty(area);
        edit.render(area, &mut edit_buf);
        let corner = name_corner(&edit_buf);
        assert_eq!(
            edit_buf[corner].fg, focused,
            "edit dialog: prefilled, valid name paints the focused border"
        );
    }
}
