//! OCPP module setup dialog (`:new`). Collects name, version, role, the websocket endpoint
//! (ip/port), a TLS selector (Off / TLS / mTLS) mapping onto `ServerTlsPolicy`/`ClientTlsPolicy`
//! and driving the read-only Protocol display, and an independent Basic Authentication toggle
//! with its credential inputs, validating live like the Modbus dialog.

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

use crate::config::ClientOrServer;
use crate::dialog::NonEmpty;
use crate::dialog::choices::ReconnectChoice;
use crate::dialog::close_confirm::{CloseConfirmDialog, CloseConfirmOutcome, route_close_confirm};
use crate::dialog::path_suggest::FsPathProvider;
use crate::dialog::tls_section::{TlsSection, TlsSectionFocus};
use crate::dialog::widgets::{input, selection, set_input, set_suggest_input, suggest_input};
use crate::module::ocpp::config::device::OcppSecurityConfig;
use crate::module::ocpp::config::session::{OcppProtocol, OcppRole, OcppSpec, OcppVersion};

mod headers;
mod security;
use headers::{
    HeaderEditOutcome, HeaderEditPrompt, HeaderTable, HeaderTableRef, header_name_input,
    header_table, header_value_input, route_header_edit,
};
use security::{BasicAuthChoice, SecurityInputs, TlsLevel, validate_security};

use crate::module::modbus::dialog::{
    ConfirmDeleteDialog, DeleteConfirmOutcome, route_delete_confirm,
};

/// Live validator for the device-config path field: empty is allowed (a fresh empty config),
/// otherwise the path must be a TOML/JSON file, and — if it exists — a loadable OCPP device
/// config. Mirrors the Modbus dialog's `ConfigPath`.
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
                match crate::config::load_ocpp_device(input) {
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
pub struct OcppSetupDialog {
    #[focus]
    pub name: Widget<InputFieldState, InputField<NonEmpty>>,
    /// Path to the OCPP device-config file (empty = a fresh, empty device config).
    #[focus]
    pub config_path:
        Widget<SuggestInputState<FsPathProvider>, SuggestInput<ConfigPath, FsPathProvider>>,
    #[focus]
    pub version: Widget<SelectionState<OcppVersion>, Selection<OcppVersion>>,
    #[focus]
    pub role: Widget<SelectionState<OcppRole>, Selection<OcppRole>>,
    /// Automatically reconnect (with backoff) instead of ending the module task on failure:
    /// client redial (OC-R-048) or server bind retry (OC-R-083). Shown for every role.
    #[focus]
    pub reconnect: Widget<SelectionState<ReconnectChoice>, Selection<ReconnectChoice>>,
    /// Read-only display, derived from `tls_level` alone (OC-R-127): `wss://` whenever the
    /// selector is not `Off`, `ws://` when it is. Not a focusable field — the derive-generated
    /// focus cycle skips it, and its selection is written by `sync_tls` on every render/event
    /// pass, never by a key event.
    pub protocol: Widget<SelectionState<OcppProtocol>, Selection<OcppProtocol>>,
    #[focus]
    pub ip: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub port: Widget<InputFieldState, InputField<u16>>,
    /// Optional URL path appended after the endpoint, e.g. `/ocpp/cp001`.
    #[focus(when = {self.role.get_value() == OcppRole::Client})]
    pub path: Widget<InputFieldState, InputField<String>>,
    /// Extra HTTP headers sent on the client's websocket handshake (OC-R-117/118/119,
    /// UI-R-059). Client role only — a CSMS server has no outbound handshake to attach headers
    /// to. Hidden while `extra_headers` is empty (mirrors `ca_files`) — an empty table
    /// has nothing to select/edit/delete, so it is skipped by focus and never painted.
    #[focus(when = {self.show_headers_table()})]
    pub headers_table: HeaderTable,
    /// Add-a-header name input, sharing a row with `header_value_input` below the table.
    #[focus(when = {self.show_headers()})]
    pub header_name_input: Widget<InputFieldState, InputField<String>>,
    /// Add-a-header value input.
    #[focus(when = {self.show_headers()})]
    pub header_value_input: Widget<InputFieldState, InputField<String>>,
    /// Working copy of the extra-headers list edited via `headers_table`/the add-inputs/the edit
    /// popup. Threaded onto [`crate::module::ocpp::config::device::OcppDeviceConfig::extra_headers`]
    /// by the host (not part of [`OcppSpec`], which `resolve` builds — headers live on the device
    /// config, not the spec).
    #[builder(default)]
    pub extra_headers: Vec<ferrowl_ocpp::HeaderDef>,
    /// Edit-in-place popup for the selected header row, opened by Enter on `headers_table`; not
    /// itself a `#[focus]` field — routed specially in `handle_events`, mirroring
    /// `ca_add_dialog`.
    #[builder(default)]
    pub header_edit_prompt: Option<HeaderEditPrompt>,
    /// Delete-confirmation popup for the selected header row, opened by `d` on `headers_table`.
    #[builder(default)]
    pub header_delete_confirm: Option<ConfirmDeleteDialog>,
    /// Validation error from the add-inputs/edit-popup (OC-R-117/118), shown via `error` alongside
    /// `resolve`'s own errors. Kept separate from `resolve`'s `Result` because `extra_headers` is
    /// not part of `OcppSpec` — `resolve` has no way to see or report a header rejection itself.
    #[builder(default)]
    pub header_error: Option<String>,
    /// TLS selector (OC-R-127), shown unconditionally for both roles.
    #[focus]
    pub tls_level: Widget<SelectionState<TlsLevel>, Selection<TlsLevel>>,
    /// Basic Authentication toggle (OC-R-128), independent of `tls_level`, shown unconditionally.
    #[focus]
    pub basic_auth: Widget<SelectionState<BasicAuthChoice>, Selection<BasicAuthChoice>>,
    /// Basic Auth username. Note: rendered as plain text — no masked-input widget exists yet.
    #[focus(when = {self.show_credentials()})]
    pub username: Widget<InputFieldState, InputField<String>>,
    /// Basic Auth password. Note: rendered as plain text (no masking) — same limitation as
    /// `username`; the field is not obscured on screen.
    #[focus(when = {self.show_credentials()})]
    pub password: Widget<InputFieldState, InputField<String>>,
    /// The TLS/mTLS cluster (self-signed toggle, own-identity cert/key pair, skip-verify toggle,
    /// peer-verification input, client-CA add/remove list), shared with the Modbus setup dialog.
    /// `tls_shown()` alone gates entry — once entered, `TlsSection`'s own internal `when` gates
    /// (fed by `sync`, called at the top of every funnel method below) take over.
    #[focus(nested, when = {self.tls_shown()})]
    pub tls: TlsSection,
    /// Security section the dialog was opened with (`edit`; `Default` for a fresh dialog): the
    /// source for stitching the *inactive* role's half back into the resolved config, so a role
    /// toggle preserves the other role's previously-saved settings instead of resetting them to
    /// [`OcppSecurityConfig::default`]'s placeholder (mirrors the Modbus dialog's `original_tls`).
    pub preserved_security: OcppSecurityConfig,
    /// `Path::exists` results with a timestamp, so the per-tick live validation does not stat
    /// the filesystem on every redraw (see [`path_exists`](Self::path_exists)).
    pub fs_cache: std::cell::RefCell<std::collections::HashMap<String, (bool, std::time::Instant)>>,
    pub error: Widget<String, Text>,
    pub keybinds: Widget<String, Text>,
    /// Close-confirm popup, opened by Esc.
    #[builder(default)]
    pub close_confirm: Option<CloseConfirmDialog>,
    /// Set on confirmed close; the host checks this via `take_close_request` and closes the dialog.
    #[builder(default)]
    close_requested: bool,
}

impl OcppSetupDialog {
    pub fn new() -> Self {
        let input_style = InputFieldStyle::default();
        let selection_style = SelectionStyle::default();

        OcppSetupDialogBuilder::default()
            .name(input("Name", "cs-1", &input_style, true))
            .config_path(suggest_input(
                "Config",
                "device.toml",
                &input_style,
                false,
                FsPathProvider::with_extensions(&["toml", "json"]),
            ))
            .version(selection(
                "Version",
                vec![OcppVersion::V1_6, OcppVersion::V2_0_1, OcppVersion::V2_1],
                &selection_style,
            ))
            .role(selection(
                ("Role", HorizontalAlignment::Center),
                vec![OcppRole::Client, OcppRole::Server],
                &selection_style,
            ))
            .protocol(selection(
                "Protocol",
                vec![OcppProtocol::Ws, OcppProtocol::Wss],
                &selection_style,
            ))
            .ip(input("IP", "127.0.0.1", &input_style, false))
            .port(input("Port", "9000", &input_style, false))
            .path(input("Path", "/ocpp/cp001", &input_style, false))
            .headers_table(header_table(Vec::new()))
            .header_name_input(header_name_input(crate::view::border_style()))
            .header_value_input(header_value_input(crate::view::border_style()))
            .extra_headers(Vec::new())
            .reconnect(selection(
                ("Reconnect", HorizontalAlignment::Right),
                vec![ReconnectChoice::On, ReconnectChoice::Off],
                &selection_style,
            ))
            .tls_level(selection(
                "TLS",
                vec![TlsLevel::Off, TlsLevel::Tls, TlsLevel::MutualTls],
                &selection_style,
            ))
            .basic_auth(selection(
                "Basic Auth",
                vec![BasicAuthChoice::Off, BasicAuthChoice::On],
                &selection_style,
            ))
            .username(input("Username", "cp001", &input_style, false))
            .password(input("Password", "", &input_style, false))
            .tls(TlsSection::new())
            .preserved_security(OcppSecurityConfig::default())
            .fs_cache(Default::default())
            .error(text(TextStyle {
                general: ratatui::prelude::Style::default()
                    .fg(COLOR_SCHEME.error)
                    .bg(COLOR_SCHEME.bg),
            }))
            .keybinds(keybinds_text())
            .focus(OcppSetupDialogFocus::Name)
            .build()
            .expect("all required builder fields are set")
    }

    /// Build a dialog pre-filled with an existing spec + device-config path, for `:edit`.
    /// `extra_headers` seeds the headers table (OC-R-117/118/119, UI-R-059) — it comes from
    /// [`crate::module::ocpp::config::device::OcppDeviceConfig::extra_headers`], not `spec`,
    /// since headers live on the device config rather than the resolved spec.
    pub fn edit(
        spec: &OcppSpec,
        device_path: &str,
        extra_headers: &[ferrowl_ocpp::HeaderDef],
    ) -> Self {
        let mut d = Self::new();
        d.extra_headers = extra_headers.to_vec();
        d.headers_table = header_table(headers::rows(&d.extra_headers));
        set_input(&mut d.name, &spec.name);
        set_suggest_input(&mut d.config_path, device_path);
        d.version.state.set_selection(match spec.version {
            OcppVersion::V1_6 => 0,
            OcppVersion::V2_0_1 => 1,
            OcppVersion::V2_1 => 2,
        });
        d.role.state.set_selection(match spec.role {
            OcppRole::Client => 0,
            OcppRole::Server => 1,
        });
        set_input(&mut d.ip, &spec.ip);
        set_input(&mut d.port, &spec.port.to_string());
        set_input(&mut d.path, &spec.path);
        d.reconnect
            .state
            .set_selection(if spec.reconnect.unwrap_or(true) { 0 } else { 1 });

        let level = TlsLevel::from_config(&spec.security, spec.role);
        d.tls_level.state.set_selection(level.index());
        d.basic_auth.state.set_selection(
            if spec.security.username.is_some() && spec.security.password.is_some() {
                1
            } else {
                0
            },
        );
        set_input(
            &mut d.username,
            spec.security.username.as_deref().unwrap_or(""),
        );
        set_input(
            &mut d.password,
            spec.security.password.as_deref().unwrap_or(""),
        );

        // `TlsSection`'s public surface is typed on `ClientOrServer` (Modbus's own role enum, the
        // repo's existing cross-module client/server marker), not `OcppRole` — a local 2-arm
        // match at each of `TlsSection`'s 3 call sites in this file, not a shared trait impl.
        let role_for_tls = match spec.role {
            OcppRole::Client => ClientOrServer::Client,
            OcppRole::Server => ClientOrServer::Server,
        };
        d.tls.prefill(
            role_for_tls,
            Some(&spec.security.tls.server),
            Some(&spec.security.tls.client),
        );
        d.preserved_security = spec.security.clone();
        d
    }

    /// Validate every field and produce the spec, or an error message for the live display.
    pub fn resolve(&self) -> Result<OcppSpec, String> {
        let name = self.name.state.input().trim().to_string();
        if name.is_empty() {
            return Err("Name is required.".into());
        }
        if let ValidateResult::Error(e) = ConfigPath::validate(self.config_path.state.input()) {
            return Err(e);
        }
        let mut ip = self.ip.state.input().trim().to_string();
        if ip.is_empty() {
            ip = "127.0.0.1".to_string();
        }
        let port_in = self.port.state.input();
        let port = if port_in.trim().is_empty() {
            9000
        } else {
            port_in
                .trim()
                .parse::<u16>()
                .map_err(|_| "Port must be a number (0-65535).".to_string())?
        };

        // Normalize the optional URL path: trim, and ensure a leading '/' when non-empty. The
        // server role has no URL path, so it is always empty there.
        let mut path = if self.path_hidden() {
            String::new()
        } else {
            self.path.state.input().trim().to_string()
        };
        if !path.is_empty() && !path.starts_with('/') {
            path.insert(0, '/');
        }

        let role = self.role.get_value();
        let reconnect = Some(self.reconnect.state.get_value() == ReconnectChoice::On);
        let protocol = self.protocol();
        let level = self.tls_level.get_value();
        let is_client = role == OcppRole::Client;
        let is_server = role == OcppRole::Server;
        // `extract()` never reads `self.tls`'s own `role`/`level` (it's a uniform read of
        // raw text/toggle state, regardless of what's currently focusable/visible), so this
        // read-only path needs no `sync()` call first — unlike `render`/`handle_events`,
        // which do read `TlsSection`'s `when`-gated fields and must stay fresh.
        let extracted = self.tls.extract();
        let mut cfg = level.build_config(
            role,
            SecurityInputs {
                username: self.username.state.input(),
                password: self.password.state.input(),
                basic_auth: self.basic_auth.get_value() == BasicAuthChoice::On,
                cert_file: &extracted.cert_file,
                key_file: &extracted.key_file,
                client_cert_file: &extracted.client_cert_file,
                client_key_file: &extracted.client_key_file,
                ca_files: &extracted.ca_files,
                self_signed: extracted.self_signed,
                skip_verify: is_client && extracted.skip_verify,
                client_cert_skip_verify: is_server && extracted.client_cert_skip_verify,
                root_store: extracted.root_store,
            },
        )?;
        // Stitch the inactive role's half back in from the config the dialog was opened
        // with (if any), so a role toggle preserves the other role's previously-saved
        // security settings instead of resetting them to `OcppSecurityConfig::default`'s
        // placeholder (mirrors the Modbus dialog's `original_tls` stitching).
        match role {
            OcppRole::Server => cfg.tls.client = self.preserved_security.tls.client.clone(),
            OcppRole::Client => cfg.tls.server = self.preserved_security.tls.server.clone(),
        }
        validate_security(&cfg, role, level, &|p| self.path_exists(p))?;
        let security = cfg;

        Ok(OcppSpec {
            name,
            version: self.version.state.get_value(),
            role,
            protocol,
            ip,
            port,
            path,
            timeout_ms: None,
            reconnect,
            security,
        })
    }

    /// The entered device-config path (trimmed; empty when none).
    pub fn config_path(&self) -> String {
        self.config_path.state.input().trim().to_string()
    }

    /// The working extra-headers list edited via `headers_table` (OC-R-117/118/119, UI-R-059).
    pub fn extra_headers(&self) -> Vec<ferrowl_ocpp::HeaderDef> {
        self.extra_headers.clone()
    }

    /// Route a key: the close-confirm popup captures all keys while open; then the client-CA
    /// add-dialog (OC-R-113), if open; then the client-CA ADD/DEL buttons (Enter/Space); Esc
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

        match route_header_edit(&mut self.header_edit_prompt, modifiers, code) {
            HeaderEditOutcome::NotActive => {}
            HeaderEditOutcome::Consumed => return EventResult::Consumed,
            HeaderEditOutcome::Commit(name, value) => {
                if let Some(index) = self.headers_ref().selected() {
                    match self.headers_ref().commit_edit(index, &name, &value) {
                        Ok(()) => {
                            self.header_error = None;
                            self.header_edit_prompt = None;
                        }
                        Err(e) => self.header_error = Some(e.to_string()),
                    }
                } else {
                    // The selected row vanished (e.g. deleted from elsewhere) while the prompt
                    // was open; drop the now-stale prompt rather than apply to nothing.
                    self.header_edit_prompt = None;
                }
                return EventResult::Consumed;
            }
        }

        match route_delete_confirm(&mut self.header_delete_confirm, modifiers, code) {
            DeleteConfirmOutcome::NotActive => {}
            DeleteConfirmOutcome::Confirmed => {
                self.headers_ref().delete_selected();
                return EventResult::Consumed;
            }
            DeleteConfirmOutcome::Consumed => return EventResult::Consumed,
        }

        if self.focus == OcppSetupDialogFocus::HeadersTable {
            if modifiers == KeyModifiers::NONE && code == KeyCode::Enter {
                if let Some(prompt) = self.headers_ref().open_edit_prompt() {
                    self.header_edit_prompt = Some(prompt);
                }
                return EventResult::Consumed;
            }
            if modifiers == KeyModifiers::NONE && code == KeyCode::Char('d') {
                if self.headers_ref().selected().is_some() {
                    self.header_delete_confirm = Some(ConfirmDeleteDialog::new("this header"));
                }
                return EventResult::Consumed;
            }
        }

        if modifiers == KeyModifiers::NONE
            && code == KeyCode::Enter
            && matches!(
                self.focus,
                OcppSetupDialogFocus::HeaderNameInput | OcppSetupDialogFocus::HeaderValueInput
            )
            // Only treat Enter as "add" when the user actually typed something. Both inputs are
            // cleared on a successful add but focus stays put (there is nowhere sensible to move
            // it to), so a bare Enter pressed again right after adding a header — the natural
            // next keystroke to confirm/close the whole dialog — must not be swallowed here
            // trying to add an empty header (which always fails OC-R-118's non-empty-name check
            // and would otherwise trap Enter in this cluster indefinitely).
            && !(self.header_name_input.state.input().trim().is_empty()
                && self.header_value_input.state.input().trim().is_empty())
        {
            match self.headers_ref().add() {
                Ok(()) => self.header_error = None,
                Err(e) => self.header_error = Some(e.to_string()),
            }
            return EventResult::Consumed;
        }

        if modifiers == KeyModifiers::NONE
            && matches!(code, KeyCode::Enter | KeyCode::Char(' '))
            && self.focus == OcppSetupDialogFocus::Tls
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

    /// The URL `path` field is only meaningful for the client (CS) role — the CSMS server binds a
    /// host:port and ignores it — so it is hidden (and skipped by focus) when the role is Server.
    fn path_hidden(&self) -> bool {
        self.role.get_value() == OcppRole::Server
    }

    /// The extra-headers cluster (table + add-inputs, OC-R-117/118/119, UI-R-059) — client role
    /// only, independent of `wss`/security: headers ride the plain websocket handshake too.
    fn show_headers(&self) -> bool {
        self.role.get_value() == OcppRole::Client
    }

    /// The table itself, as opposed to the always-visible add-inputs (when `show_headers`):
    /// hidden while `extra_headers` is empty, mirroring `ca_files`'s
    /// `show_ca_list() && !values().is_empty()` gate — an empty table has nothing to
    /// select/edit/delete, so painting an empty box would waste a row for no purpose.
    fn show_headers_table(&self) -> bool {
        self.show_headers() && !self.extra_headers.is_empty()
    }

    /// Bundle of `&mut` borrows into this dialog's own headers-cluster fields (see
    /// [`HeaderTableRef`]'s doc comment for why this can't be a nested owned struct).
    fn headers_ref(&mut self) -> HeaderTableRef<'_> {
        HeaderTableRef {
            headers: &mut self.extra_headers,
            table: &mut self.headers_table,
            name_input: &mut self.header_name_input,
            value_input: &mut self.header_value_input,
        }
    }

    /// The derived scheme (OC-R-127): `wss://` whenever the TLS selector is not `Off`, `ws://`
    /// when it is. The single source for both the read-only display and `resolve`.
    fn protocol(&self) -> OcppProtocol {
        if self.tls_level.get_value() == TlsLevel::Off {
            OcppProtocol::Ws
        } else {
            OcppProtocol::Wss
        }
    }

    fn level(&self) -> TlsLevel {
        self.tls_level.get_value()
    }

    /// Whether `TlsSection`'s own fields are reachable at all.
    fn tls_shown(&self) -> bool {
        self.level() != TlsLevel::Off
    }

    /// Push this dialog's own live role/level widgets into `self.tls` so its `when` gates read
    /// fresh state, and the read-only protocol display so it always tracks the selector.
    fn sync_tls(&mut self) {
        self.protocol.state.set_selection(match self.protocol() {
            OcppProtocol::Ws => 0,
            OcppProtocol::Wss => 1,
        });
        let role = match self.role.get_value() {
            OcppRole::Client => ClientOrServer::Client,
            OcppRole::Server => ClientOrServer::Server,
        };
        self.tls.sync(role, self.level().into());
    }

    // --- Security-field visibility -----------------------------------------------------------
    // Single source of truth consumed by the `#[focus(when)]` gates, the render branches and the
    // dialog-height computation, so keyboard focus, painting and layout can never disagree about
    // which fields exist.

    /// Basic Auth credential inputs, shown while the toggle is On (OC-R-128) — no scheme or TLS-
    /// level term.
    fn show_credentials(&self) -> bool {
        self.basic_auth.get_value() == BasicAuthChoice::On
    }

    /// Cached `Path::exists` with a short TTL: `render` re-runs `resolve` (and so the security
    /// validation) on every 100ms tick, and stat-ing configured certificate paths each tick is
    /// wasted I/O — and visibly laggy on network filesystems. One second of staleness is
    /// imperceptible next to typing latency.
    fn path_exists(&self, path: &str) -> bool {
        const TTL: std::time::Duration = std::time::Duration::from_secs(1);
        let now = std::time::Instant::now();
        let mut cache = self.fs_cache.borrow_mut();
        if let Some((hit, at)) = cache.get(path)
            && now.duration_since(*at) < TTL
        {
            return *hit;
        }
        let exists = ferrowl_util::path::expand(path).exists();
        cache.insert(path.to_string(), (exists, now));
        exists
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.sync_tls();

        match (self.resolve(), &self.header_error) {
            (Err(e), _) => self.error.state = e,
            (Ok(_), Some(e)) => self.error.state = e.clone(),
            (Ok(_), None) => self.error.state.clear(),
        }

        let has_error = !self.error.state.is_empty();
        let role = self.role.get_value();
        let is_server = role == OcppRole::Server;
        let show_headers = self.show_headers();
        let show_headers_table = self.show_headers_table();
        let show_credentials = self.show_credentials();
        // Fixed 3-row TLS layout (both roles): TLS + Basic Auth (unconditional), a Self-Signed
        // row that also carries the own-identity cert/key pair, and a Skip Verify row that also
        // carries the peer-verification input — each combined row's existence is governed by its
        // toggle field's own gate, since the paired file field's gate is always a strict subset
        // of it.
        let show_self_signed_row = self.tls.show_self_signed_row();
        let show_identity_row = self.tls.show_identity_row();
        let show_skip_verify_row = self.tls.show_skip_verify_row();
        let show_peer_verify_row = self.tls.show_peer_verify_row();

        // border(2) + inner margin(2) + name(3) + config path(3) + version|role|reconnect(3)
        // + protocol|ip|port|path(3) + the TLS + Basic Auth row(3) + keybinds(1), plus the error
        // box (3), the headers add-inputs (3, client role only), the headers table itself (7 =
        // border(2) + header(1) + 4 rows, only once `extra_headers` is non-empty), the
        // self-signed row (3, mTLS only for client), and the skip-verify row (3), only when
        // applicable.
        let box_height = 20
            + if has_error { 3 } else { 0 }
            + if show_headers { 3 } else { 0 }
            + if show_headers_table { 7 } else { 0 }
            + if show_self_signed_row { 3 } else { 0 }
            + if show_skip_verify_row { 3 } else { 0 };
        let box_width = 80;

        let [_, hcenter, _] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(box_width),
            Constraint::Min(1),
        ])
        .areas(area);
        let [_, vcenter, _] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(box_height),
            Constraint::Min(1),
        ])
        .areas(hcenter);

        let block = Block::bordered()
            .style(
                ratatui::prelude::Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg),
            )
            .title_alignment(HorizontalAlignment::Center)
            .title("New OCPP Module");
        let block_inner = block.inner(vcenter);
        let inner = block_inner.inner(Margin::new(2, 1));
        UiWidget::render(&Clear, vcenter, buf);
        block.render(vcenter, buf);

        let error_height = if has_error { 3 } else { 0 };
        let headers_table_height = if show_headers_table { 7 } else { 0 };
        let headers_inputs_height = if show_headers { 3 } else { 0 };
        let self_signed_height = if show_self_signed_row { 3 } else { 0 };
        let skip_verify_height = if show_skip_verify_row { 3 } else { 0 };
        let rows = Layout::vertical([
            Constraint::Length(3),                     // 0: name
            Constraint::Length(3),                     // 1: config path
            Constraint::Length(3), // 2: version | role | reconnect (client only)
            Constraint::Length(3), // 3: protocol | ip | port | path
            Constraint::Length(headers_table_height), // 4: headers table (client only, non-empty)
            Constraint::Length(headers_inputs_height), // 5: header name | value add-inputs
            Constraint::Length(3), // 6: tls_level | basic_auth | username | password
            Constraint::Length(self_signed_height), // 7: self_signed (+ own-identity cert/key pair)
            Constraint::Length(skip_verify_height), // 8: skip-verify toggle (+ peer-verification input)
            Constraint::Length(error_height),       // 9: error (hidden when empty)
            Constraint::Length(1),                  // 10: keybinds
        ])
        .split(inner);

        render_field!(self, name, rows[0], buf);
        render_field!(self, config_path, rows[1], buf);
        render_row!(self, rows[2], buf; version, role, reconnect);

        if self.path_hidden() {
            // No URL path for the server role — let ip take the freed space.
            render_row!(self, rows[3], buf;
                protocol => Constraint::Length(12),
                ip => Constraint::Min(1),
                port => Constraint::Length(13)
            );
        } else {
            render_row!(self, rows[3], buf;
                protocol => Constraint::Length(12),
                ip => Constraint::Min(1),
                port => Constraint::Length(13),
                path => Constraint::Length(24)
            );
        }

        if show_headers_table {
            render_field!(self, headers_table, rows[4], buf);
        } else if self.focus == OcppSetupDialogFocus::HeadersTable {
            self.focus_next();
        }
        if show_headers {
            render_row!(self, rows[5], buf; header_name_input, header_value_input);
        }

        if show_credentials {
            render_row!(self, rows[6], buf;
                tls_level => Constraint::Percentage(20),
                basic_auth => Constraint::Percentage(20),
                username => Constraint::Fill(1),
                password => Constraint::Fill(1)
            );
        } else {
            render_row!(self, rows[6], buf;
                tls_level => Constraint::Percentage(50),
                basic_auth => Constraint::Fill(1)
            );
        }

        // Self-Signed row: always Self-Signed itself, plus (when applicable) the role's own
        // identity cert/key pair sharing the same row.
        {
            // Rebinding so `render_field!`/`render_row!` see a bare ident bound to `TlsSection`
            // instead of literally `self` — the macros only need `.field.widget`/`.field.state`
            // on whatever they're given, not `self` specifically.
            let tls = &mut self.tls;
            if show_self_signed_row {
                if show_identity_row {
                    if is_server {
                        render_row!(tls, rows[7], buf;
                            self_signed => Constraint::Percentage(25),
                            cert_file => Constraint::Fill(1),
                            key_file => Constraint::Fill(1)
                        );
                    } else {
                        render_row!(tls, rows[7], buf;
                            self_signed => Constraint::Percentage(25),
                            client_cert_file => Constraint::Fill(1),
                            client_key_file => Constraint::Fill(1)
                        );
                    }
                } else {
                    render_field!(tls, self_signed, rows[7], buf);
                }
            }

            // Skip Verify row: always the Skip Verify toggle itself, plus (when applicable) the
            // Root Store toggle (client only, OC-R-125) and the peer-verification list
            // sharing the same row.
            if show_skip_verify_row {
                if show_peer_verify_row {
                    // No CA entries yet: give ADD the remaining width and skip DEL entirely
                    // rather than paint an empty, nothing-to-delete button.
                    let empty = tls.ca_files.state.values().is_empty();
                    if is_server {
                        if empty {
                            render_row!(tls, rows[8], buf;
                                client_cert_skip_verify => Constraint::Percentage(25),
                                ca_files => Constraint::Percentage(60),
                                ca_add_button => Constraint::Fill(1)
                            );
                        } else {
                            render_row!(tls, rows[8], buf;
                                client_cert_skip_verify => Constraint::Percentage(25),
                                ca_files => Constraint::Percentage(45),
                                ca_add_button => Constraint::Percentage(15),
                                ca_delete_button => Constraint::Fill(1)
                            );
                        }
                    } else if empty {
                        render_row!(tls, rows[8], buf;
                            skip_verify => Constraint::Percentage(25),
                            root_store => Constraint::Percentage(20),
                            ca_files => Constraint::Percentage(40),
                            ca_add_button => Constraint::Fill(1)
                        );
                    } else {
                        render_row!(tls, rows[8], buf;
                            skip_verify => Constraint::Percentage(25),
                            root_store => Constraint::Percentage(20),
                            ca_files => Constraint::Percentage(25),
                            ca_add_button => Constraint::Percentage(15),
                            ca_delete_button => Constraint::Fill(1)
                        );
                    }
                } else if is_server {
                    render_field!(tls, client_cert_skip_verify, rows[8], buf);
                } else {
                    render_field!(tls, skip_verify, rows[8], buf);
                }
            }
        }

        if has_error {
            render_field!(self, error, rows[9], buf);
        }
        render_field!(self, keybinds, rows[10], buf);

        // Must be called after every sibling widget above has been rendered, so a popup paints on
        // top rather than being overwritten (painter's-algorithm buffer model).
        self.config_path
            .widget
            .render_overlay(area, buf, &mut self.config_path.state);
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

        if let Some(prompt) = self.header_edit_prompt.as_mut() {
            prompt.render(area, buf);
        }

        if let Some(confirm) = self.header_delete_confirm.as_mut() {
            confirm.render(area, buf);
        }

        if let Some(confirm) = self.close_confirm.as_mut() {
            confirm.render(vcenter, buf);
        }
    }
}

fn text(style: TextStyle) -> Widget<String, Text> {
    Widget {
        state: String::new(),
        widget: TextBuilder::default()
            .multiline(true)
            .border(Border::Full(Margin::new(1, 0)))
            .title(Some(("Error", HorizontalAlignment::Left).into()))
            .margin(Margin {
                vertical: 0,
                horizontal: 1,
            })
            .horizontal_alignment(HorizontalAlignment::Center)
            .style(style)
            .build()
            .expect("all required builder fields are set"),
    }
}

fn keybinds_text() -> Widget<String, Text> {
    Widget {
        state: "<Tab>: next | <\u{2191}/\u{2193}>: select | <Enter>: confirm | <Esc>: cancel"
            .to_string(),
        widget: TextBuilder::default()
            .margin(Margin {
                vertical: 0,
                horizontal: 1,
            })
            .horizontal_alignment(HorizontalAlignment::Center)
            .style(TextStyle::default())
            .build()
            .expect("all required builder fields are set"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::tls_section::SkipVerifyChoice;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_test_support::{TempDirGuard, reserve_temp_dir};
    use ferrowl_ui::traits::{IsFocus, SetFocus};
    use ferrowl_util::tls::{CertSource, CertVerification, ClientTlsPolicy, ServerTlsPolicy};

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

    fn tmp_file(dir: &TempDirGuard, name: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, b"").unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    /// NF-R-054 — a `~/...` config path validates the same way it will later load.
    fn ut_config_path_validate_expands_tilde() {
        let home = std::env::home_dir().expect("HOME must resolve in test environment");
        let name = format!("ferrowl_ocpp_setup_tilde_cfg_{}.toml", std::process::id());
        ferrowl_util::convert::Converter::save(
            &crate::module::ocpp::config::device::OcppDeviceConfig::default(),
            home.join(&name).to_str().unwrap(),
            FileType::Toml,
        )
        .unwrap();

        let result = ConfigPath::validate(&format!("~/{name}"));
        let _ = std::fs::remove_file(home.join(&name));

        assert!(matches!(result, ValidateResult::Success));
    }

    #[test]
    /// NF-R-054 — `path_exists` (wired via `resolve()`'s `validate_security`) expands a leading
    /// `~` in cert/key paths, so a valid `~/...` path validates the same way TLS material
    /// loading will.
    fn ut_resolve_tls_cert_key_tilde_paths_validate() {
        let home = std::env::home_dir().expect("HOME must resolve in test environment");
        let cert_name = format!("ferrowl_ocpp_setup_tilde_{}.crt", std::process::id());
        let key_name = format!("ferrowl_ocpp_setup_tilde_{}.key", std::process::id());
        std::fs::write(home.join(&cert_name), b"").unwrap();
        std::fs::write(home.join(&key_name), b"").unwrap();

        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut d.tls.cert_file, &format!("~/{cert_name}"));
        set_suggest_input(&mut d.tls.key_file, &format!("~/{key_name}"));

        let outcome = d.resolve();
        let _ = std::fs::remove_file(home.join(&cert_name));
        let _ = std::fs::remove_file(home.join(&key_name));

        outcome.expect("a valid ~/-prefixed cert/key path must validate");
    }

    #[test]
    /// OC-R-048, OC-R-107 — the setup dialog resolves a reconnect-off selection into the spec.
    fn ut_resolve_reconnect_off_maps_to_some_false() {
        let mut d = OcppSetupDialog::new(); // Client by default
        set_input(&mut d.name, "cs-1");
        d.reconnect.state.set_selection(1); // Off
        let spec = d.resolve().expect("valid client config");
        assert_eq!(spec.reconnect, Some(false));
    }

    #[test]
    /// OC-R-083 — a server-role setup reports its own reconnect setting (default On), same as
    /// the client role.
    fn ut_resolve_server_role_reports_reconnect() {
        let mut d = OcppSetupDialog::new();
        set_input(&mut d.name, "csms-1");
        d.role.state.set_selection(1); // Server
        let spec = d.resolve().expect("valid server config");
        assert_eq!(spec.reconnect, Some(true));
        d.reconnect.state.set_selection(1); // Off
        let spec = d.resolve().expect("valid server config");
        assert_eq!(spec.reconnect, Some(false));
    }

    #[test]
    /// OC-R-107 — editing an existing client spec prefills the reconnect toggle from the spec.
    fn ut_edit_prefills_reconnect_off() {
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: Some(false),
            security: OcppSecurityConfig::default(),
        };
        let dialog = OcppSetupDialog::edit(&spec, "device.toml", &[]);
        assert_eq!(dialog.reconnect.state.get_value(), ReconnectChoice::Off);
        let resolved = dialog.resolve().expect("valid client config");
        assert_eq!(resolved.reconnect, Some(false));
    }

    #[test]
    /// UI-R-022 — the focus cycle reaches the reconnect field for a server role too (OC-R-083).
    fn ut_focus_next_reaches_reconnect_for_server_role() {
        let mut d = OcppSetupDialog::new();
        d.role.state.set_selection(1); // Server
        d.set_focused(true);
        let mut visited = false;
        for _ in 0..20 {
            d.focus_next();
            if d.reconnect.state.is_focused() {
                visited = true;
                break;
            }
        }
        assert!(
            visited,
            "reconnect field must be reachable for a server role"
        );
    }

    // Editing a ws module whose device file carries a security section (Basic Auth over plain ws
    // is valid, config-file-only) hands that section back unchanged: the security UI is hidden for
    // ws, and a hidden section must never clobber the file.
    #[test]
    /// A ws setup resolves preserving the prefilled security.
    fn ut_resolve_ws_preserves_prefilled_security() {
        let security = OcppSecurityConfig {
            username: Some("cp001".into()),
            password: Some("secret".into()),
            ..Default::default()
        };
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: security.clone(),
        };
        let d = OcppSetupDialog::edit(&spec, "", &[]);
        let resolved = d.resolve().expect("ws edit resolves");
        assert_eq!(resolved.security, security);
    }

    fn type_into(state: &mut InputFieldState, s: &str) {
        state.set_focused(true);
        for c in s.chars() {
            state.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
    }

    // --- dialog-level validation ---------------------------------------------------------------

    fn dialog_with(role_idx: usize) -> OcppSetupDialog {
        let mut d = OcppSetupDialog::new();
        set_input(&mut d.name, "cs-1");
        d.role.state.set_selection(role_idx);
        d
    }

    // --- OC-R-127: TLS selector ---------------------------------------------------------------

    #[test]
    /// OC-R-127, OC-R-160 — selector `Off` resolves the server role's policy to `None {}` and the resolved
    /// scheme to `ws://`; the client role resolves its policy to `None {}` too.
    fn ut_selector_off_resolves_none_policy_and_ws_scheme() {
        let d = dialog_with(1); // Server, selector defaults to Off
        let spec = d.resolve().expect("selector off resolves");
        assert_eq!(spec.security.tls.server, ServerTlsPolicy::None {});
        assert_eq!(spec.protocol, OcppProtocol::Ws);

        let d = dialog_with(0); // Client
        let spec = d.resolve().expect("selector off resolves");
        assert_eq!(spec.security.tls.client, ClientTlsPolicy::None {});
        assert_eq!(spec.protocol, OcppProtocol::Ws);
    }

    #[test]
    /// OC-R-127 — selector TLS and mTLS resolve to `Tls`/`Mutual` per role, with the resolved
    /// scheme `wss://`.
    fn ut_selector_tls_and_mtls_resolve_policy_and_wss_scheme() {
        let dir = reserve_temp_dir("ferrowl_ocpp_setup");
        let cert = tmp_file(&dir, "selector_cert.crt");
        let key = tmp_file(&dir, "selector_key.key");
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut d.tls.cert_file, &cert);
        set_suggest_input(&mut d.tls.key_file, &key);
        let spec = d.resolve().expect("tls resolves");
        assert!(matches!(
            spec.security.tls.server,
            ServerTlsPolicy::Tls { .. }
        ));
        assert_eq!(spec.protocol, OcppProtocol::Wss);

        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.tls.client_cert_skip_verify.state.set_selection(1); // Skip Verify On, avoid CA-list requirement
        let spec = d.resolve().expect("mtls resolves");
        assert!(matches!(
            spec.security.tls.server,
            ServerTlsPolicy::Mutual { .. }
        ));
        assert_eq!(spec.protocol, OcppProtocol::Wss);
    }

    #[test]
    /// OC-R-161 — the Protocol field is not in the focus cycle, and its rendered text follows
    /// the selector alone.
    fn ut_protocol_is_not_in_the_focus_cycle_and_follows_the_selector() {
        let mut d = OcppSetupDialog::new();
        d.set_focused(true);
        for _ in 0..30 {
            d.focus_next();
            assert!(
                !d.protocol.state.is_focused(),
                "protocol must never receive focus"
            );
        }

        d.sync_tls();
        assert_eq!(d.protocol.get_value(), OcppProtocol::Ws);
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert_eq!(d.protocol.get_value(), OcppProtocol::Wss);
    }

    #[test]
    /// OC-R-163 — moving the selector Off after cert paths, toggle positions, and CA entries
    /// were entered, then back to mTLS, restores every widget's prior state.
    fn ut_selector_off_then_back_to_mtls_restores_widget_state() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        set_suggest_input(&mut d.tls.cert_file, "s.crt");
        set_suggest_input(&mut d.tls.key_file, "s.key");
        d.tls.self_signed.state.set_selection(1);
        d.tls.ca_files.state.set_values(vec!["ca1.pem".to_string()]);

        d.tls_level.state.set_selection(TlsLevel::Off.index());
        let spec = d.resolve().expect("off resolves");
        assert_eq!(spec.security.tls.server, ServerTlsPolicy::None {});

        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        assert_eq!(d.tls.cert_file.state.input(), "s.crt");
        assert_eq!(d.tls.key_file.state.input(), "s.key");
        assert_eq!(d.tls.self_signed.state.selection(), 1);
        assert_eq!(d.tls.ca_files.state.values(), &["ca1.pem".to_string()]);
    }

    // --- OC-R-128: Basic Authentication ---------------------------------------------------------

    #[test]
    /// OC-R-164 — Basic Authentication On with both credentials set resolves `username`/
    /// `password`.
    fn ut_basic_auth_on_resolves_credentials() {
        let mut d = dialog_with(0); // Client
        d.basic_auth.state.set_selection(1); // On
        set_input(&mut d.username, "cp001");
        set_input(&mut d.password, "s3cret");
        let spec = d.resolve().expect("valid client config");
        assert_eq!(spec.security.username.as_deref(), Some("cp001"));
        assert_eq!(spec.security.password.as_deref(), Some("s3cret"));
    }

    #[test]
    /// OC-R-164 — Basic Authentication Off resolves both `username`/`password` unset,
    /// preserving the inputs' stored text.
    fn ut_basic_auth_off_unsets_credentials_and_keeps_input_text() {
        // TLS selector at `Tls`, not `Off`: proves the gate is Basic Auth alone, not the level
        // (a fixture at `Off` cannot distinguish the two, since the old level-derived rule and
        // the independent toggle happen to agree there).
        let mut d = dialog_with(0); // Client
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.basic_auth.state.set_selection(1); // On
        set_input(&mut d.username, "cp001");
        set_input(&mut d.password, "s3cret");
        d.basic_auth.state.set_selection(0); // Off, after text was typed

        let spec = d.resolve().expect("valid client config");
        assert_eq!(spec.security.username, None);
        assert_eq!(spec.security.password, None);
        assert_eq!(d.username.state.input(), "cp001");
        assert_eq!(d.password.state.input(), "s3cret");
    }

    #[test]
    /// OC-R-165 — Basic Authentication On together with the TLS selector Off is accepted
    /// (Profile 1) and produces a `ws://` endpoint carrying the credentials.
    fn ut_basic_auth_on_with_tls_off_is_accepted_over_ws() {
        let mut d = dialog_with(0); // Client, selector Off by default
        d.basic_auth.state.set_selection(1); // On
        set_input(&mut d.username, "cp001");
        set_input(&mut d.password, "s3cret");
        let spec = d.resolve().expect("basic auth over ws is valid");
        assert_eq!(spec.protocol, OcppProtocol::Ws);
        assert_eq!(spec.security.username.as_deref(), Some("cp001"));
    }

    #[test]
    /// OC-R-128 — editing an existing config with credentials prefills Basic Authentication On.
    fn ut_edit_prefills_basic_auth_on_from_credentials() {
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: OcppSecurityConfig {
                username: Some("cp001".into()),
                password: Some("s3cret".into()),
                ..Default::default()
            },
        };
        let dialog = OcppSetupDialog::edit(&spec, "device.toml", &[]);
        assert_eq!(dialog.basic_auth.state.selection(), 1);
        assert_eq!(dialog.username.state.input(), "cp001");
        assert_eq!(dialog.password.state.input(), "s3cret");
    }

    #[test]
    /// OC-R-161 — reopening a hand-written `ws://` instance whose own-role policy is not
    /// `None` shows the derived selector at TLS/mTLS and the `wss://` display; confirming
    /// unchanged promotes the inert pairing to a live `wss://` one (OC-R-127 working as
    /// specified, not preserving the mismatch OC-R-042/OC-R-097 render inert).
    fn ut_edit_of_ws_instance_with_inert_policy_promotes_to_wss() {
        let policy = ClientTlsPolicy::Tls {
            verification: CertVerification::RootStore {
                extra_ca_files: vec![],
            },
        };
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: OcppSecurityConfig {
                tls: crate::module::ocpp::config::device::OcppTlsConfig {
                    client: policy.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let mut dialog = OcppSetupDialog::edit(&spec, "device.toml", &[]);
        assert_eq!(dialog.tls_level.state.get_value(), TlsLevel::Tls);
        // `protocol`'s stored selection is a derived display, refreshed by `sync_tls` at the top
        // of `render` (never eagerly at construction) — drive one render pass to check the
        // display through the same path a real dialog paint takes.
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);
        assert_eq!(dialog.protocol.get_value(), OcppProtocol::Wss);
        let resolved = dialog.resolve().expect("valid client config");
        assert_eq!(resolved.protocol, OcppProtocol::Wss);
        assert_eq!(resolved.security.tls.client, policy);
    }

    #[test]
    /// OC-R-128 — a one-of-two credential config (username without password, or vice versa)
    /// leaves Basic Authentication Off on reopen, matching ocpp/edge-cases.md's "field is inert"
    /// rule for a lone `username`/`password`.
    fn ut_edit_leaves_basic_auth_off_when_only_one_credential_is_set() {
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: OcppSecurityConfig {
                username: Some("cp001".into()),
                password: None,
                ..Default::default()
            },
        };
        let dialog = OcppSetupDialog::edit(&spec, "device.toml", &[]);
        assert_eq!(dialog.basic_auth.state.selection(), 0);
    }

    // --- OC-R-110/OC-R-111: Self-Signed / Skip-Verify toggle parity with the Modbus dialog -----

    #[test]
    /// OC-R-110 — the Self-Signed toggle is hidden at selector Off and shown at TLS and mTLS.
    fn ut_self_signed_row_hidden_at_selector_off_shown_at_tls_and_mtls() {
        let mut d = dialog_with(1); // Server
        d.sync_tls();
        assert!(!d.tls.show_self_signed_row());

        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(d.tls.show_self_signed_row());

        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.sync_tls();
        assert!(d.tls.show_self_signed_row());
    }

    #[test]
    /// OC-R-143 — toggling Self-Signed On hides the server cert/key row.
    fn ut_self_signed_hides_server_cert_row() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(d.tls.show_identity_row());
        d.tls.self_signed.state.set_selection(1); // On
        assert!(!d.tls.show_identity_row());
        d.tls.self_signed.state.set_selection(0); // Off again
        assert!(d.tls.show_identity_row());
    }

    #[test]
    /// OC-R-110, OC-R-144 — toggling Self-Signed On excludes stale cert_file/key_file text from the
    /// resolved config, even though the widgets' stored text is untouched (mirrors MB-R-135 in
    /// the Modbus dialog).
    fn ut_resolve_self_signed_excludes_stale_cert_key_text() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut d.tls.cert_file, "s.crt");
        set_suggest_input(&mut d.tls.key_file, "s.key");
        d.tls.self_signed.state.set_selection(1); // On, after the text was typed

        let spec = d.resolve().expect("self-signed needs no cert/key files");
        assert_eq!(
            spec.security.tls.server,
            ServerTlsPolicy::Tls {
                identity: CertSource::SelfSigned {}
            }
        );
        // The stored text survives the toggle -- only the resolved config excludes it.
        assert_eq!(d.tls.cert_file.state.input(), "s.crt");
        assert_eq!(d.tls.key_file.state.input(), "s.key");
    }

    #[test]
    /// OC-R-143 — Self-Signed On at TLS level needs no cert/key files to resolve successfully
    /// — the cert/key requirement at Tls+ applies only while Self-Signed is Off.
    fn ut_validate_security_self_signed_needs_no_cert_files() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.tls.self_signed.state.set_selection(1); // On
        assert!(d.resolve().is_ok());
    }

    #[test]
    /// OC-R-111, OC-R-155 — toggling Skip-Verify On hides the Root Store toggle and the CA-file row.
    fn ut_cs_skip_verify_hides_root_store_and_list() {
        let mut d = dialog_with(0); // Client
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(d.tls.show_peer_verify_row());
        assert!(d.tls.show_root_store_row());
        d.tls.skip_verify.state.set_selection(1); // On
        assert!(!d.tls.show_peer_verify_row());
        assert!(!d.tls.show_root_store_row());
        d.tls.skip_verify.state.set_selection(0); // Off again
        assert!(d.tls.show_peer_verify_row());
        assert!(d.tls.show_root_store_row());
    }

    #[test]
    /// OC-R-125 — the Root Store toggle and the shared CA list are hidden at selector Off,
    /// including with Basic Authentication On, and shown once the selector reaches TLS or mTLS.
    fn ut_root_store_and_ca_list_shown_only_at_tls_or_mtls_with_skip_verify_off() {
        let mut d = dialog_with(0); // Client
        d.sync_tls();
        assert!(!d.tls.show_root_store_row());
        d.basic_auth.state.set_selection(1); // On
        d.sync_tls();
        assert!(!d.tls.show_root_store_row());
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(d.tls.show_root_store_row());
    }

    #[test]
    /// OC-R-113, OC-R-125 — the CA list is the exact same shared field for both the CSMS (server)
    /// and CS (client) roles, not a second copy: switching role alone flips which gate
    /// (`client_cert_skip_verify` vs `skip_verify`) controls `show_peer_verify_row`, but both
    /// paths read the one `ca_files` widget.
    fn ut_ca_list_shared_by_csms_and_cs_roles() {
        let mut d = dialog_with(1); // Server (CSMS)
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.sync_tls();
        *d.tls.ca_files.state.values_mut() = vec!["fleet-ca.pem".to_string()];
        assert!(d.tls.show_peer_verify_row());

        d.role.state.set_selection(0); // Client (CS)
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(d.tls.show_peer_verify_row());
        assert_eq!(
            d.tls.ca_files.state.values(),
            &["fleet-ca.pem".to_string()],
            "the same widget/list survives a role switch"
        );
    }

    #[test]
    /// OC-R-145, OC-R-146 — toggling Skip-Verify On excludes the stale CA list from the resolved config,
    /// even though the widget's stored list is untouched.
    fn ut_resolve_skip_verify_excludes_stale_ca_list() {
        let mut d = dialog_with(0); // Client
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        *d.tls.ca_files.state.values_mut() = vec!["ca.pem".to_string()];
        d.tls.skip_verify.state.set_selection(1); // On, after the list was populated

        let spec = d.resolve().expect("skip-verify needs no ca list");
        assert_eq!(
            spec.security.tls.client,
            ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {}
            }
        );
        assert_eq!(d.tls.ca_files.state.values(), &["ca.pem".to_string()]);
    }

    #[test]
    /// OC-R-095 — a station created through the dialog with the selector Off, whose `wss://`
    /// endpoint is written by hand afterwards, still takes the ephemeral-identity fallback and
    /// logs it: the dialog's Off never resolves to a `Tls { identity: SelfSigned }` placeholder
    /// in place of `None`, so a hand-edited scheme change reaches OC-R-095's real fallback path.
    fn ut_dialog_off_plus_hand_edited_wss_takes_the_ephemeral_fallback() {
        let d = dialog_with(1); // Server, selector Off
        let mut spec = d.resolve().expect("selector off resolves");
        assert_eq!(spec.security.tls.server, ServerTlsPolicy::None {});

        spec.protocol = OcppProtocol::Wss;
        assert!(spec.csms_self_signed_fallback());
        assert_eq!(
            spec.effective_csms_tls(),
            ServerTlsPolicy::Tls {
                identity: CertSource::Ephemeral {}
            }
        );
    }

    #[test]
    /// OC-R-112 — a server TLS setup with both `cert_file`/`key_file` blank and
    /// `self_signed` off refuses to resolve, keeping the dialog open.
    fn ut_server_tls_missing_cert_is_rejected() {
        let mut d = dialog_with(1);
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        let err = d.resolve().unwrap_err();
        assert!(err.contains("Certificate file is required"), "{err}");
    }

    #[test]
    /// OC-R-112 — `cert_file` set alone (not both), while `self_signed` is off, fails resolution
    /// rather than silently falling through to an inert listener.
    fn ut_server_tls_cert_file_alone_is_rejected() {
        let dir = reserve_temp_dir("ferrowl_ocpp_setup");
        let cert = tmp_file(&dir, "cert_alone.crt");
        let mut d = dialog_with(1);
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut d.tls.cert_file, &cert);
        let err = d.resolve().unwrap_err();
        assert!(err.contains("must both be set, or neither"), "{err}");
    }

    #[test]
    /// A server TLS setup with a nonexistent cert file fails validation.
    fn ut_server_tls_nonexistent_cert_is_rejected() {
        let mut d = dialog_with(1);
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut d.tls.cert_file, "/no/such/cert.crt");
        set_suggest_input(&mut d.tls.key_file, "/no/such/key.key");
        let err = d.resolve().unwrap_err();
        assert!(err.contains("Certificate file not found"), "{err}");
    }

    #[test]
    /// A server TLS setup with valid files passes validation.
    fn ut_server_tls_valid_files_pass() {
        let dir = reserve_temp_dir("ferrowl_ocpp_setup");
        let cert = tmp_file(&dir, "cert.crt");
        let key = tmp_file(&dir, "key.key");
        let mut d = dialog_with(1);
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut d.tls.cert_file, &cert);
        set_suggest_input(&mut d.tls.key_file, &key);
        assert!(d.resolve().is_ok());
    }

    #[test]
    /// A mutual-TLS server missing its client CA fails validation.
    fn ut_server_mutual_tls_missing_client_ca_is_rejected() {
        let dir = reserve_temp_dir("ferrowl_ocpp_setup");
        let cert = tmp_file(&dir, "cert2.crt");
        let key = tmp_file(&dir, "key2.key");
        let mut d = dialog_with(1);
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        set_suggest_input(&mut d.tls.cert_file, &cert);
        set_suggest_input(&mut d.tls.key_file, &key);
        let err = d.resolve().unwrap_err();
        assert!(err.contains("Client CA list is required"), "{err}");
    }

    /// Steps `d.tls.focus_next()` until it lands on `target`, bounded at `TlsSection`'s own field
    /// count so a `target` that's ineligible under the caller's role/level setup panics
    /// immediately instead of spinning forever.
    fn focus_tls_until(d: &mut OcppSetupDialog, target: TlsSectionFocus) {
        for _ in 0..11 {
            if d.tls.focus() == target {
                return;
            }
            d.tls.focus_next();
        }
        panic!("{target:?} never became eligible under the current role/level setup");
    }

    #[test]
    /// OC-R-113 — the setup dialog's own ADD/DEL routing (`handle_events`, not `TlsSection`'s
    /// inherent method of the same name, which this dialog doesn't call): ADD opens the sub-
    /// dialog; an empty path is rejected with an inline error and nothing appended; Esc closes
    /// the sub-dialog with nothing appended; a confirmed path is appended; DEL removes the
    /// selected entry, and draining to empty falls focus back to ADD.
    fn ut_client_ca_add_delete_lifecycle_via_outer_dialog() {
        let dir = reserve_temp_dir("ferrowl_ocpp_setup");
        let ca = tmp_file(&dir, "ocpp_outer_ca.pem");
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.tls.client_cert_skip_verify.state.set_selection(0); // Off: client-CA row shows
        d.sync_tls();
        focus_tls_until(&mut d, TlsSectionFocus::CaAddButton);
        d.focus = OcppSetupDialogFocus::Tls;

        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.tls.ca_add_dialog.is_some());

        // An empty path is rejected: the sub-dialog stays open with an error, nothing appended.
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.tls.ca_add_dialog.is_some());
        assert!(!d.tls.ca_add_dialog.as_ref().unwrap().error.state.is_empty());

        // Esc closes the sub-dialog with nothing appended.
        d.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(d.tls.ca_add_dialog.is_none());
        assert!(d.tls.ca_files.state.values().is_empty());

        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.tls.ca_add_dialog.is_some());
        set_suggest_input(&mut d.tls.ca_add_dialog.as_mut().unwrap().path, &ca);
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.tls.ca_add_dialog.is_none());
        assert_eq!(d.tls.ca_files.state.values(), std::slice::from_ref(&ca));

        focus_tls_until(&mut d, TlsSectionFocus::CaDeleteButton);
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.tls.ca_files.state.values().is_empty());
        assert_ne!(
            d.tls.focus(),
            TlsSectionFocus::CaDeleteButton,
            "draining the list must not strand focus on the now-hidden DEL button"
        );
    }

    /// UI-R-026 — the client-CA add sub-dialog's path field, sharing `TlsSection`'s routing,
    /// honors the general suggestion-popup contract: Enter accepts the highlighted suggestion
    /// while the popup is open, rather than submitting the sub-dialog immediately.
    #[test]
    fn ut_ca_add_dialog_enter_accepts_suggestion_before_submit() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.tls.client_cert_skip_verify.state.set_selection(0);
        d.sync_tls();
        focus_tls_until(&mut d, TlsSectionFocus::CaAddButton);
        d.focus = OcppSetupDialogFocus::Tls;

        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        let sub = d.tls.ca_add_dialog.as_mut().unwrap();
        sub.path.state.set_focused(true);
        sub.path
            .state
            .handle_events(KeyModifiers::NONE, KeyCode::Char('s'));
        assert!(
            sub.path.state.suggestions_open(),
            "no completion popup offered for a 's' prefix (expects to match e.g. 'src')"
        );

        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        let sub = d.tls.ca_add_dialog.as_ref().unwrap();
        assert_ne!(
            sub.path.state.input(),
            "s",
            "Enter with the completion popup open must accept the highlighted suggestion, \
             extending the typed prefix -- unchanged text means Enter fell through to submit \
             (and failed) instead of accepting the popup"
        );
        assert!(
            d.tls.ca_add_dialog.is_some(),
            "accepting a (possibly partial) suggestion must not itself close the sub-dialog"
        );
    }

    #[test]
    /// OC-R-116, OC-R-148 — the client role's Self-Signed toggle is shown only at MutualTls, and excludes
    /// stale client-cert/key text from the resolved config when on.
    fn ut_client_self_signed_shown_only_at_mutual_tls_and_excludes_stale_cert_key() {
        let mut d = dialog_with(0); // Client
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(!d.tls.show_self_signed_row());

        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.sync_tls();
        assert!(d.tls.show_self_signed_row());
        set_suggest_input(&mut d.tls.client_cert_file, "stale.crt");
        set_suggest_input(&mut d.tls.client_key_file, "stale.key");
        d.tls.self_signed.state.set_selection(1); // On, after the text was typed

        let spec = d
            .resolve()
            .expect("self-signed client needs no cert/key files");
        assert_eq!(
            spec.security.tls.client,
            ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
                identity: CertSource::SelfSigned {},
            }
        );
        assert_eq!(d.tls.client_cert_file.state.input(), "stale.crt");
        assert_eq!(d.tls.client_key_file.state.input(), "stale.key");
    }

    #[test]
    /// OC-R-151 — the server-role client-cert-skip-verify toggle is shown only at MutualTls and
    /// hides the client-CA list row when on.
    fn ut_server_client_cert_skip_verify_shown_only_at_mutual_tls_hides_ca_list() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(!d.tls.show_skip_verify_row());

        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.sync_tls();
        assert!(d.tls.show_skip_verify_row());
        assert!(d.tls.show_peer_verify_row());
        d.tls.client_cert_skip_verify.state.set_selection(1); // On
        assert!(!d.tls.show_peer_verify_row());
    }

    #[test]
    /// OC-R-112 — a mutual-TLS client missing its cert/key fails validation.
    fn ut_client_mutual_tls_missing_cert_key_is_rejected() {
        let mut d = dialog_with(0); // Client
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        let err = d.resolve().unwrap_err();
        // `TlsLevel::build_config` itself rejects "neither cert nor key nor self-signed" before
        // `validate_security` ever runs (mirrors the Modbus dialog's `build_config`, which resolves
        // the client identity the same way) — the raw resolver message, not `validate_security`'s
        // own (now unreachable for this exact case) "Client certificate file is required" text.
        assert!(
            err.contains("client_cert_file and client_key_file must both be set"),
            "{err}"
        );
    }

    #[test]
    /// A client CA list entry, when set, must exist to pass validation.
    fn ut_client_ca_files_entry_when_set_must_exist() {
        let mut d = dialog_with(0);
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        *d.tls.ca_files.state.values_mut() = vec!["/no/such/ca.pem".to_string()];
        let err = d.resolve().unwrap_err();
        assert!(err.contains("CA file not found"), "{err}");
    }

    #[test]
    /// A client with the selector Off passes validation.
    fn ut_client_selector_off_is_allowed() {
        let d = dialog_with(0); // Client, selector defaults to Off
        assert!(d.resolve().is_ok());
    }

    #[test]
    /// A ws setup never requires security material.
    fn ut_ws_never_requires_security() {
        let mut d = OcppSetupDialog::new(); // Ws, Client by default
        set_input(&mut d.name, "cs-1");
        let spec = d.resolve().unwrap();
        assert_eq!(spec.security, OcppSecurityConfig::default());
    }

    // --- edit -> resolve round trip ------------------------------------------------------------

    #[test]
    /// Edit mode round-trips a mutual-TLS server config through the dialog.
    fn ut_edit_resolve_roundtrip_mutual_tls_server() {
        let dir = reserve_temp_dir("ferrowl_ocpp_setup");
        let cert = tmp_file(&dir, "rt_cert.crt");
        let key = tmp_file(&dir, "rt_key.key");
        let cca = tmp_file(&dir, "rt_cca.pem");
        let spec = OcppSpec {
            name: "csms-1".into(),
            version: OcppVersion::V2_0_1,
            role: OcppRole::Server,
            protocol: OcppProtocol::Wss,
            ip: "127.0.0.1".into(),
            port: 9443,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: OcppSecurityConfig {
                tls: crate::module::ocpp::config::device::OcppTlsConfig {
                    server: ServerTlsPolicy::Mutual {
                        identity: CertSource::Files {
                            cert_file: cert,
                            key_file: key,
                        },
                        verification: CertVerification::CaFiles {
                            ca_files: vec![cca],
                        },
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let dialog = OcppSetupDialog::edit(&spec, "device.toml", &[]);
        let resolved = dialog.resolve().expect("valid mTLS server config");
        assert_eq!(resolved.security, spec.security);
    }

    #[test]
    /// Edit mode round-trips a skip-verify client config through the dialog.
    fn ut_edit_resolve_roundtrip_client_skip_verify() {
        let spec = OcppSpec {
            name: "cp-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol: OcppProtocol::Wss,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: "/ocpp/cp001".into(),
            timeout_ms: None,
            reconnect: None,
            security: OcppSecurityConfig {
                tls: crate::module::ocpp::config::device::OcppTlsConfig {
                    client: ClientTlsPolicy::Tls {
                        verification: CertVerification::Skip {},
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let dialog = OcppSetupDialog::edit(&spec, "device.toml", &[]);
        assert_eq!(
            dialog.tls.skip_verify.state.get_value(),
            SkipVerifyChoice::On
        );
        let resolved = dialog.resolve().expect("valid client config");
        assert_eq!(
            resolved.security.tls.client,
            ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {}
            }
        );
    }

    // --- render height -----------------------------------------------------------------------

    #[test]
    /// OC-R-162 — the dialog renders no certificate-generation hint row at any selector
    /// position, for the server role in particular: the condition it warned of (a `wss` scheme
    /// below TLS) is unreachable once the scheme follows the selector.
    fn ut_no_hint_row_at_any_selector_position() {
        let area = Rect::new(0, 0, 80, 60);
        let needle = "Self-signed certificate is generated";

        let mut off = dialog_with(1); // Server, selector Off
        // Directly forces the retired `wss`-below-TLS state the old hint row was written for.
        // `render` calls `sync_tls` first, which always derives `protocol` from the selector and
        // so overwrites this before anything reads it — the manual set only matters for proving
        // the assertion below fails against code that lacked such a derivation; it
        // gives no coverage against a regression reintroducing the hint post-`sync_tls`.
        off.protocol.state.set_selection(1); // Wss
        let mut buf = Buffer::empty(area);
        off.render(area, &mut buf);
        assert!(!buffer_text(&buf).contains(needle));

        let dir = reserve_temp_dir("ferrowl_ocpp_setup");
        let cert = tmp_file(&dir, "hint_cert.crt");
        let key = tmp_file(&dir, "hint_key.key");
        let mut tls = dialog_with(1);
        tls.tls_level.state.set_selection(TlsLevel::Tls.index());
        set_suggest_input(&mut tls.tls.cert_file, &cert);
        set_suggest_input(&mut tls.tls.key_file, &key);
        let mut buf2 = Buffer::empty(area);
        tls.render(area, &mut buf2);
        assert!(!buffer_text(&buf2).contains(needle));

        let mut mtls = dialog_with(1);
        mtls.tls_level
            .state
            .set_selection(TlsLevel::MutualTls.index());
        mtls.tls.client_cert_skip_verify.state.set_selection(1);
        let mut buf3 = Buffer::empty(area);
        mtls.render(area, &mut buf3);
        assert!(!buffer_text(&buf3).contains(needle));
    }

    // --- focus traversal ------------------------------------------------------------------------

    #[test]
    /// UI-R-022 — a fresh (selector Off, Basic Auth Off) dialog's focus cycle reaches the TLS
    /// selector and Basic Auth toggle (both unconditional, OC-R-127/OC-R-128), but hides
    /// Username/Password and the entire nested `tls` field, none of them eligible at Off.
    fn ut_focus_ws_hides_credential_and_tls_section_fields() {
        let mut d = OcppSetupDialog::new(); // TLS selector Off by default
        d.set_focused(true);
        assert_eq!(d.focus, OcppSetupDialogFocus::Name);
        let mut visited = Vec::new();
        for _ in 0..20 {
            d.focus_next();
            visited.push(d.focus);
            assert!(!matches!(
                d.focus,
                OcppSetupDialogFocus::Username
                    | OcppSetupDialogFocus::Password
                    | OcppSetupDialogFocus::Tls
            ));
        }
        assert!(visited.contains(&OcppSetupDialogFocus::TlsLevel));
        assert!(visited.contains(&OcppSetupDialogFocus::BasicAuth));
    }

    #[test]
    /// UI-R-022 — a client focus cycle at selector Off includes the TLS selector, but never
    /// enters the nested `tls` field (`tls_shown()` requires the selector off `Off`) or reaches
    /// Username (`show_credentials()` requires Basic Authentication On).
    fn ut_focus_selector_off_shows_tls_selector_for_client() {
        let mut d = dialog_with(0); // Client, selector Off
        d.set_focused(true);
        let mut visited = Vec::new();
        for _ in 0..20 {
            d.focus_next();
            visited.push(d.focus);
        }
        assert!(visited.contains(&OcppSetupDialogFocus::TlsLevel));
        assert!(!visited.contains(&OcppSetupDialogFocus::Tls));
        assert!(!visited.contains(&OcppSetupDialogFocus::Username));
    }

    #[test]
    /// OC-R-111 — the client Skip-Verify toggle is hidden at selector Off, including with Basic
    /// Authentication On, and shown at TLS/mTLS.
    fn ut_skip_verify_row_hidden_at_selector_off_including_with_basic_auth_on() {
        let mut d = dialog_with(0); // Client

        d.sync_tls();
        assert!(!d.tls.show_skip_verify_row());

        d.basic_auth.state.set_selection(1); // On
        d.sync_tls();
        assert!(!d.tls.show_skip_verify_row());

        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        d.sync_tls();
        assert!(d.tls.show_skip_verify_row());

        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.sync_tls();
        assert!(d.tls.show_skip_verify_row());
    }

    #[test]
    /// UI-R-022 — a server focus cycle at selector Off never enters the nested `tls` field.
    fn ut_focus_selector_off_server_has_no_skip_verify() {
        let mut d = dialog_with(1); // Server, selector Off
        d.set_focused(true);
        let mut visited = Vec::new();
        for _ in 0..20 {
            d.focus_next();
            visited.push(d.focus);
        }
        assert!(visited.contains(&OcppSetupDialogFocus::TlsLevel));
        assert!(!visited.contains(&OcppSetupDialogFocus::Tls));
    }

    /// One stop of the flattened Tab walk used by the tab-order tests below: while `self.focus`
    /// sits on the nested `Tls` field, Tab steps *within* `TlsSection`'s own panes (dispatched to
    /// its `HandleEvents`, which tries `NestedFocus::try_focus_next` on an `Unhandled` Tab/
    /// BackTab); once that inner scan is exhausted, the outer struct's own `focus_next()`
    /// advances to the next top-level field. The outer wrap-around walk alone only ever treats
    /// `tls` as a single field — both mechanisms together walk the full sequence.
    #[derive(Debug, Clone, PartialEq)]
    enum Stop {
        Outer(OcppSetupDialogFocus),
        Tls(TlsSectionFocus),
    }

    fn current_stop(d: &OcppSetupDialog) -> Stop {
        if d.focus == OcppSetupDialogFocus::Tls {
            Stop::Tls(d.tls.focus())
        } else {
            Stop::Outer(d.focus)
        }
    }

    /// Drive a flattened forward Tab walk from `OcppSetupDialogFocus::TlsLevel` back around to
    /// `OcppSetupDialogFocus::Name` (`tls` is the last declared focusable field, so the outer
    /// wrap-around walk returns to `Name` once past it), recording every stop.
    fn tab_sequence_from_tls_level(d: &mut OcppSetupDialog) -> Vec<Stop> {
        // Production always runs `handle_events()` (which calls `sync_tls()` first) before any
        // Tab/BackTab reaches `focus_next()`/`focus_previous()`, so `self.tls`'s role/level are
        // always fresh. A test driving `focus_next()` directly must reproduce that precondition:
        // unsynced, `self.tls`'s role/level default to `Off`, making every pane ineligible, so
        // `focus_next()` would skip straight past `tls` as if it weren't there.
        d.sync_tls();
        d.focus = OcppSetupDialogFocus::TlsLevel;
        let mut seq = vec![current_stop(d)];
        loop {
            if d.focus == OcppSetupDialogFocus::Tls {
                let result = d.handle_events(KeyModifiers::NONE, KeyCode::Tab);
                if matches!(result, EventResult::Consumed) {
                    seq.push(current_stop(d));
                    continue;
                }
                d.focus_next();
            } else {
                d.focus_next();
            }
            seq.push(current_stop(d));
            if d.focus == OcppSetupDialogFocus::Name {
                break;
            }
        }
        seq
    }

    /// Backward counterpart, driven with `SHIFT`+`BackTab` from `OcppSetupDialogFocus::Name` back
    /// to `OcppSetupDialogFocus::TlsLevel`. Entering `tls` *backward* lands on its *last*
    /// eligible pane, not its first.
    fn back_tab_sequence_from_name(d: &mut OcppSetupDialog) -> Vec<Stop> {
        d.sync_tls();
        d.focus = OcppSetupDialogFocus::Name;
        let mut seq = vec![current_stop(d)];
        loop {
            if d.focus == OcppSetupDialogFocus::Tls {
                let result = d.handle_events(KeyModifiers::SHIFT, KeyCode::BackTab);
                if matches!(result, EventResult::Consumed) {
                    seq.push(current_stop(d));
                    continue;
                }
                d.focus_previous();
            } else {
                d.focus_previous();
            }
            seq.push(current_stop(d));
            if d.focus == OcppSetupDialogFocus::TlsLevel {
                break;
            }
        }
        seq
    }

    #[test]
    /// OC-R-113, UI-R-049 — the mTLS server-role Tab order: own cert/key, then Skip Verify,
    /// then the client-CA list and its ADD/DEL buttons — reached by bubbling into/out of the
    /// nested `tls` field. Also proves entering `tls` backward lands on its last pane (DEL), not
    /// its first (Self-Signed).
    fn ut_tab_order_server_mtls() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.tls.ca_files.state.set_values(vec!["ca1.pem".to_string()]); // non-empty, so DEL is eligible
        let seq = tab_sequence_from_tls_level(&mut d);
        assert_eq!(
            seq,
            vec![
                Stop::Outer(OcppSetupDialogFocus::TlsLevel),
                Stop::Outer(OcppSetupDialogFocus::BasicAuth),
                Stop::Tls(TlsSectionFocus::SelfSigned),
                Stop::Tls(TlsSectionFocus::CertFile),
                Stop::Tls(TlsSectionFocus::KeyFile),
                Stop::Tls(TlsSectionFocus::ClientCertSkipVerify),
                Stop::Tls(TlsSectionFocus::CaFiles),
                Stop::Tls(TlsSectionFocus::CaAddButton),
                Stop::Tls(TlsSectionFocus::CaDeleteButton),
                Stop::Outer(OcppSetupDialogFocus::Name),
            ]
        );

        let back_seq = back_tab_sequence_from_name(&mut d);
        assert_eq!(
            back_seq,
            vec![
                Stop::Outer(OcppSetupDialogFocus::Name),
                Stop::Tls(TlsSectionFocus::CaDeleteButton),
                Stop::Tls(TlsSectionFocus::CaAddButton),
                Stop::Tls(TlsSectionFocus::CaFiles),
                Stop::Tls(TlsSectionFocus::ClientCertSkipVerify),
                Stop::Tls(TlsSectionFocus::KeyFile),
                Stop::Tls(TlsSectionFocus::CertFile),
                Stop::Tls(TlsSectionFocus::SelfSigned),
                Stop::Outer(OcppSetupDialogFocus::BasicAuth),
                Stop::Outer(OcppSetupDialogFocus::TlsLevel),
            ]
        );
    }

    #[test]
    /// OC-R-111, OC-R-116, OC-R-125, UI-R-049 — the mTLS client-role Tab order: own cert/key,
    /// Skip Verify, Root Store, then the shared CA list's ADD button (empty list, no DEL).
    fn ut_tab_order_client_mtls() {
        let mut d = dialog_with(0); // Client
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        let seq = tab_sequence_from_tls_level(&mut d);
        assert_eq!(
            seq,
            vec![
                Stop::Outer(OcppSetupDialogFocus::TlsLevel),
                Stop::Outer(OcppSetupDialogFocus::BasicAuth),
                Stop::Tls(TlsSectionFocus::SelfSigned),
                Stop::Tls(TlsSectionFocus::ClientCertFile),
                Stop::Tls(TlsSectionFocus::ClientKeyFile),
                Stop::Tls(TlsSectionFocus::SkipVerify),
                Stop::Tls(TlsSectionFocus::RootStore),
                Stop::Tls(TlsSectionFocus::CaAddButton),
                Stop::Outer(OcppSetupDialogFocus::Name),
            ]
        );
    }

    /// Typing into the config-path field opens the filesystem suggestion popup, and the popup is
    /// drawn on top of the dialog by the trailing `render_overlay` calls in `render`.
    #[test]
    /// UI-R-026 — the config-path field shows a completion popup.
    fn ut_render_config_path_field_shows_suggestion_popup() {
        let mut dialog = OcppSetupDialog::new();
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

    // --- close-confirm --------------------------------------------------------------------------

    #[test]
    /// UI-R-023 — Esc-then-Enter sets the close request, which clears after being taken.
    fn ut_take_close_request_set_via_esc_enter_and_cleared_after_take() {
        let mut dialog = OcppSetupDialog::new();
        assert!(!dialog.take_close_request());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_some());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(dialog.take_close_request());
        assert!(!dialog.take_close_request(), "flag must clear after take");
    }

    #[test]
    /// UI-R-023 — Esc in the close-confirm keeps the setup dialog open.
    fn ut_esc_in_confirm_keeps_open() {
        let mut dialog = OcppSetupDialog::new();
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_some());
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(dialog.close_confirm.is_none());
        assert!(!dialog.take_close_request());
    }

    #[test]
    /// UI-R-014 — `:` types into a setup text field rather than entering command mode.
    fn ut_colon_in_text_input_types() {
        let mut dialog = OcppSetupDialog::new();
        // Default focus is Name, a free-text field; `:` must be typed as ordinary text.
        dialog.handle_events(KeyModifiers::NONE, KeyCode::Char(':'));
        assert_eq!(dialog.name.state.input(), ":");
        assert!(dialog.close_confirm.is_none());
    }

    // --- row-layout refinement ---------------------------------------------------

    fn row_of(buf: &Buffer, needle: &str) -> u16 {
        let text = buffer_text(buf);
        text.lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not found in:\n{text}")) as u16
    }

    #[test]
    /// OC-R-110, OC-R-113, OC-R-116 — mTLS row order, server role: the TLS + Basic Auth row
    /// carries only tls_level/basic_auth/username/password (no side toggle); Self-Signed shares
    /// a row with the server's own cert/key pair; Skip Verify shares a row with the client-CA list.
    fn ut_mtls_row_order_server() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        let tls_row = row_of(&buf, "TLS");
        let self_signed_row = row_of(&buf, "Self-Signed");
        let cert_row = row_of(&buf, "Cert File");
        let skip_row = row_of(&buf, "Skip Verify");
        let ca_row = row_of(&buf, "CA(s)");
        assert!(
            tls_row < self_signed_row,
            "the TLS row must render before self-signed:\n{text}"
        );
        assert_eq!(
            self_signed_row, cert_row,
            "self-signed and the own-identity cert/key pair must share a row:\n{text}"
        );
        assert!(
            self_signed_row < skip_row,
            "self-signed must render before skip-verify:\n{text}"
        );
        assert_eq!(
            skip_row, ca_row,
            "skip-verify and the client-CA list must share a row:\n{text}"
        );
    }

    #[test]
    /// OC-R-110, OC-R-111, OC-R-116 — mTLS row order, client role: the TLS + Basic Auth row
    /// carries only tls_level/basic_auth/username/password; Self-Signed shares a row with the
    /// client's own cert/key pair; Skip Verify shares a row with the CA-file input.
    fn ut_mtls_row_order_client() {
        let mut d = dialog_with(0); // Client
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        let tls_row = row_of(&buf, "TLS");
        let self_signed_row = row_of(&buf, "Self-Signed");
        let cert_row = row_of(&buf, "Client Cert");
        let skip_row = row_of(&buf, "Skip Verify");
        let root_store_row = row_of(&buf, "Root Store");
        let ca_row = row_of(&buf, "CA(s)");
        assert!(
            tls_row < self_signed_row,
            "the TLS row must render before self-signed:\n{text}"
        );
        assert_eq!(
            self_signed_row, cert_row,
            "self-signed and the client's own cert/key pair must share a row:\n{text}"
        );
        assert!(
            self_signed_row < skip_row,
            "self-signed must render before skip-verify:\n{text}"
        );
        assert_eq!(
            skip_row, root_store_row,
            "skip-verify and root-store must share a row:\n{text}"
        );
        assert_eq!(
            skip_row, ca_row,
            "skip-verify and the CA list must share a row:\n{text}"
        );
    }

    #[test]
    /// The TLS + Basic Auth row carries no Self-Signed/Skip-Verify toggle:
    /// those never appear on the same line as TLS/Basic Auth/Username/Password.
    fn ut_tls_level_row_has_no_side_toggle() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::Tls.index());
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        let tls_level_line = text
            .lines()
            .find(|l| l.contains("TLS"))
            .expect("TLS row present");
        assert!(
            !tls_level_line.contains("Self-Signed"),
            "TLS row still carries the side toggle:\n{text}"
        );
    }

    /// An empty client-CA list shows no placeholder entry, and the DEL button is not
    /// rendered at all, so ADD gets the row's full width, exercised through the outer dialog's
    /// own render (unlike the Modbus dialog, this render path does not itself recover focus off
    /// a now-hidden DEL button — that guard exists only in Modbus's render(); OCPP's own DEL
    /// handler, `TlsSection::delete_selected_ca`, already handles the fallback whenever
    /// the list is actually drained via the button, which is the reachable path in practice).
    #[test]
    fn ut_render_client_ca_empty_list_hides_delete_button_outer() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.tls.client_cert_skip_verify.state.set_selection(0);
        d.sync_tls();
        assert!(d.tls.ca_files.state.values().is_empty());

        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains("DEL"), "empty list must hide DEL:\n{text}");
        assert!(text.contains("ADD"), "ADD button missing:\n{text}");
    }

    #[test]
    /// The client-CA row's DEL button hugs the dialog's right inner edge with no
    /// trailing dead space, matching every other full-width row (mirrors the Modbus dialog).
    fn ut_ca_delete_button_hugs_right_edge() {
        let mut d = dialog_with(1); // Server
        d.tls_level.state.set_selection(TlsLevel::MutualTls.index());
        d.tls.ca_files.state.set_values(vec!["ca1.pem".to_string()]);
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);

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

    // --- OC-R-117/118/119, UI-R-059: extra-headers table ------------------------------------

    fn client_dialog() -> OcppSetupDialog {
        let mut d = OcppSetupDialog::new();
        set_input(&mut d.name, "cs-1");
        d // Client by default
    }

    fn header(name: &str, value: &str) -> ferrowl_ocpp::HeaderDef {
        ferrowl_ocpp::HeaderDef::new(name, value).unwrap()
    }

    #[test]
    /// OC-R-117 — editing an existing device seeds the headers table from `extra_headers`.
    fn ut_edit_seeds_extra_headers_table() {
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: OcppSecurityConfig::default(),
        };
        let headers = vec![header("X-Tenant", "acme-1")];
        let dialog = OcppSetupDialog::edit(&spec, "device.toml", &headers);
        assert_eq!(dialog.extra_headers, headers);
        assert_eq!(dialog.headers_table.state.values().len(), 1);
    }

    #[test]
    /// UI-R-059 — the headers cluster is client-only; a server-role dialog hides it (and never
    /// grows the dialog's box height for it), mirroring `path`.
    fn ut_headers_table_hidden_for_server_role() {
        let mut d = client_dialog();
        d.extra_headers = vec![header("X-A", "1")];
        d.headers_table = header_table(headers::rows(&d.extra_headers));
        assert!(d.show_headers());
        assert!(d.show_headers_table());
        d.role.state.set_selection(1); // Server
        assert!(!d.show_headers());
        assert!(!d.show_headers_table());

        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            !text.contains("Extra Headers"),
            "headers table rendered for the server role:\n{text}"
        );
    }

    #[test]
    /// UI-R-059 (mid-review addendum) — the headers table itself is hidden while `extra_headers`
    /// is empty, even for the client role; the add-inputs stay visible so the first header can
    /// still be entered.
    fn ut_headers_table_hidden_when_empty_shown_once_populated() {
        let mut d = client_dialog();
        assert!(d.show_headers());
        assert!(!d.show_headers_table());
        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            !text.contains("Extra Headers"),
            "empty headers table must not be painted:\n{text}"
        );
        assert!(
            text.contains("Header Name"),
            "add-inputs must stay visible even with an empty table:\n{text}"
        );

        d.extra_headers = vec![header("X-A", "1")];
        d.headers_table = header_table(headers::rows(&d.extra_headers));
        assert!(d.show_headers_table());
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Extra Headers"),
            "headers table must appear once a header exists:\n{text}"
        );
    }

    #[test]
    /// UI-R-096 — `Enter` on the selected header row opens an edit prompt prefilled from it.
    fn ut_enter_on_selected_header_row_opens_edit_prompt_prefilled() {
        let mut d = client_dialog();
        d.extra_headers = vec![header("X-A", "1")];
        d.headers_table = header_table(headers::rows(&d.extra_headers));
        d.focus = OcppSetupDialogFocus::HeadersTable;
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        let prompt = d.header_edit_prompt.as_ref().expect("edit prompt opened");
        assert_eq!(prompt.name_input().state.input(), "X-A");
        assert_eq!(prompt.value_input().state.input(), "1");
    }

    #[test]
    /// UI-R-096 — `Enter` on an empty (unselected) header table is a no-op: no prompt opens.
    fn ut_enter_on_unselected_header_table_is_noop() {
        let mut d = client_dialog();
        d.focus = OcppSetupDialogFocus::HeadersTable;
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.header_edit_prompt.is_none());
    }

    #[test]
    /// UI-R-097 — `d` on the selected header row opens a delete-confirm popup; confirming
    /// removes the row from `extra_headers`.
    fn ut_d_on_selected_header_row_opens_delete_confirm_then_removes_on_yes() {
        let mut d = client_dialog();
        d.extra_headers = vec![header("X-A", "1")];
        d.headers_table = header_table(headers::rows(&d.extra_headers));
        d.focus = OcppSetupDialogFocus::HeadersTable;
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('d'));
        assert!(d.header_delete_confirm.is_some());

        // The DELETE button, not CANCEL, is the confirm dialog's default under test — select it
        // explicitly rather than assume its default focus.
        d.header_delete_confirm.as_mut().unwrap().focus_next();
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.header_delete_confirm.is_none());
        assert!(d.extra_headers.is_empty());
    }

    #[test]
    /// UI-R-097 — `d` on an empty (unselected) header table is a no-op: no confirm popup opens.
    fn ut_d_on_unselected_header_table_is_noop() {
        let mut d = client_dialog();
        d.focus = OcppSetupDialogFocus::HeadersTable;
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('d'));
        assert!(d.header_delete_confirm.is_none());
    }

    #[test]
    /// OC-R-117, OC-R-118 — typing into the add-inputs and pressing `Enter` appends a header and
    /// clears both inputs.
    fn ut_add_header_via_inputs_then_enter() {
        let mut d = client_dialog();
        d.focus = OcppSetupDialogFocus::HeaderValueInput;
        type_into(&mut d.header_name_input.state, "X-Tenant");
        type_into(&mut d.header_value_input.state, "acme-1");
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(d.extra_headers.len(), 1);
        assert_eq!(d.extra_headers[0].name, "X-Tenant");
        assert_eq!(d.header_name_input.state.input(), "");
        assert_eq!(d.header_value_input.state.input(), "");
    }

    #[test]
    /// OC-R-117, UI-R-095 — a refused add (reserved header name) leaves both inputs' text in place and
    /// surfaces the error via the dialog's own error box, rather than silently dropping it.
    fn ut_add_header_refused_inline_keeps_prompt_open() {
        let mut d = client_dialog();
        d.focus = OcppSetupDialogFocus::HeaderNameInput;
        type_into(&mut d.header_name_input.state, "Authorization");
        d.focus = OcppSetupDialogFocus::HeaderValueInput;
        type_into(&mut d.header_value_input.state, "Basic xyz");
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.extra_headers.is_empty());
        assert_eq!(d.header_name_input.state.input(), "Authorization");
        assert_eq!(d.header_value_input.state.input(), "Basic xyz");
        assert!(d.header_error.is_some());

        let area = Rect::new(0, 0, 80, 60);
        let mut buf = Buffer::empty(area);
        d.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Authorization"),
            "error must be surfaced without wiping the offending input:\n{text}"
        );
    }

    /// UI-R-067 — a freshly opened dialog focuses exactly one field, the first in the Tab cycle.
    /// `InputField` derives its cursor from the focus flag, and no production open path
    /// normalises the flags after construction, so a second field built focused stays focused
    /// until the first focus move — a state the existing focus-transition tests never assert,
    /// since they all move focus before looking at it.
    #[test]
    fn ut_new_dialog_focuses_only_the_name_field() {
        let dialog = OcppSetupDialog::new();
        assert!(dialog.name.state.is_focused(), "name must open focused");
        assert_eq!(
            dialog.focus,
            OcppSetupDialogFocus::Name,
            "the focus cursor must name the field carrying the flag"
        );
        assert!(
            !dialog.tls.is_focused(),
            "the nested TLS section must open unfocused"
        );

        // `focus == Name` alone would survive a reordering that moved `Name` out of first
        // position; stepping backwards from it must wrap to the last eligible field, which is
        // only true if `Name` is genuinely the cycle's first.
        let mut walked = OcppSetupDialog::new();
        walked.focus_previous();
        assert_ne!(
            walked.focus,
            OcppSetupDialogFocus::Name,
            "focus_previous from the first field must wrap away from it"
        );
        walked.focus_next();
        assert_eq!(
            walked.focus,
            OcppSetupDialogFocus::Name,
            "focus_next from the last eligible field must wrap back to Name"
        );
        for (label, focused) in [
            ("config_path", dialog.config_path.state.is_focused()),
            ("version", dialog.version.state.is_focused()),
            ("role", dialog.role.state.is_focused()),
            ("reconnect", dialog.reconnect.state.is_focused()),
            ("protocol", dialog.protocol.state.is_focused()),
            ("ip", dialog.ip.state.is_focused()),
            ("port", dialog.port.state.is_focused()),
            ("path", dialog.path.state.is_focused()),
            ("headers_table", dialog.headers_table.state.is_focused()),
            (
                "header_name_input",
                dialog.header_name_input.state.is_focused(),
            ),
            (
                "header_value_input",
                dialog.header_value_input.state.is_focused(),
            ),
            ("tls_level", dialog.tls_level.state.is_focused()),
            ("basic_auth", dialog.basic_auth.state.is_focused()),
            ("username", dialog.username.state.is_focused()),
            ("password", dialog.password.state.is_focused()),
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
    }

    /// UI-R-067 — the `:edit` open path establishes the same single-focus state as `new()`.
    /// Worth its own test because `edit` does not merely fill fields in: it replaces
    /// `headers_table` wholesale with a freshly built widget — a constructor deciding a field's
    /// focus flag, which is the shape this coverage guards against.
    #[test]
    fn ut_edit_dialog_matches_the_derive_normalised_focus_state() {
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            // An explicit-identity, verifying mTLS policy: the widest client-role shape, so
            // the TLS selector at mTLS, the credentials, and the CA-file and client cert/key
            // inputs all render (`edit()` derives the selector from this policy alone, OC-R-127
            // — the `protocol` field below plays no part in it). A `SkipVerify`/`SelfSigned`
            // policy would hide `ca_file` and the cert/key pair, and a `None` policy would hide
            // the whole cluster below the unconditional TLS + Basic Auth row — buffer equality
            // is blind to any field that is not painted.
            protocol: OcppProtocol::Wss,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: Some(false),
            security: OcppSecurityConfig {
                tls: crate::module::ocpp::config::device::OcppTlsConfig {
                    client: ClientTlsPolicy::Mutual {
                        verification: CertVerification::RootStore {
                            extra_ca_files: vec!["ca.pem".to_string()],
                        },
                        identity: CertSource::Files {
                            cert_file: "client.crt".to_string(),
                            key_file: "client.key".to_string(),
                        },
                    },
                    ..Default::default()
                },
                ..OcppSecurityConfig::default()
            },
        };
        let headers = [ferrowl_ocpp::HeaderDef {
            name: "X-Token".into(),
            value: "abc".into(),
        }];
        let area = Rect::new(0, 0, 100, 60);

        let mut as_built = OcppSetupDialog::edit(&spec, "device.toml", &headers);
        assert!(as_built.name.state.is_focused(), "name must open focused");
        assert!(
            !as_built.headers_table.state.is_focused(),
            "the rebuilt headers table must open unfocused"
        );
        assert!(
            !as_built.tls.is_focused(),
            "the nested TLS section must open unfocused"
        );
        let mut built = Buffer::empty(area);
        as_built.render(area, &mut built);

        let mut normalised = OcppSetupDialog::edit(&spec, "device.toml", &headers);
        normalised.set_focused(true);
        let mut after = Buffer::empty(area);
        normalised.render(area, &mut after);

        assert_eq!(
            built, after,
            "edit-path focus state differs from the derive's normalised state"
        );
    }

    /// UI-R-067 — the constructed focus state already equals the state `SetFocus::set_focused`
    /// normalises to, so a constructor that set a second field's flag, or that disagreed with its
    /// own focus cursor, renders differently from the normalised dialog. Needs no
    /// hand-maintained field list, so a newly added `#[focus]` field is covered the moment it
    /// renders. Three limits, the first load-bearing: a field whose `when` guard is false paints
    /// nothing, so its flag is invisible here — and `new()` is the narrowest fixture there is
    /// (`Ws`, no headers), so the security, credentials, headers-table and whole TLS-section
    /// fields are all unpainted and covered by the hand list in
    /// `ut_new_dialog_focuses_only_the_name_field` rather than by this oracle; a wrong cursor
    /// that is nonetheless eligible normalises to itself; and the comparison assumes
    /// `view_focused` is paint-neutral here, which it is.
    #[test]
    fn ut_new_dialog_matches_the_derive_normalised_focus_state() {
        let area = Rect::new(0, 0, 100, 60);

        let mut as_built = OcppSetupDialog::new();
        let mut built = Buffer::empty(area);
        as_built.render(area, &mut built);

        let mut normalised = OcppSetupDialog::new();
        normalised.set_focused(true);
        let mut after = Buffer::empty(area);
        normalised.render(area, &mut after);

        assert_eq!(
            built, after,
            "constructed focus state differs from the derive's normalised state"
        );
    }

    /// UI-R-067 — the focus flag reaches the paint. The cursor cell is the assertion because it is
    /// the only focus-derived paint that survives validation styling: `name` opens empty against
    /// `NonEmpty`, so the focused field paints its *error* border, not its focused one.
    #[test]
    fn ut_render_new_dialog_paints_one_cursor() {
        let mut dialog = OcppSetupDialog::new();
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        dialog.render(area, &mut buf);

        // Read the cursor colour from the style rather than restating its default, so a theme
        // change moves both together.
        let cursor_bg = ferrowl_ui::style::InputFieldStyle::default().cursor().bg;
        let cursors: Vec<(u16, u16)> = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| Some(buf[(x, y)].bg) == cursor_bg)
            .collect();
        assert_eq!(
            cursors.len(),
            1,
            "expected exactly one focused field cursor, found {cursors:?}:\n{}",
            buffer_text(&buf)
        );
    }
}
