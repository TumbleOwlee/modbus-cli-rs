//! Generic OCPP **server** (CSMS) view, instantiated once per OCPP version (see the
//! [`ServerVersion`] impls in `v1_6`/`v2_0_1`). Left: a table of connected charging stations and
//! their connectors, a "Lua Scripts" button, and the CSMS action list (filtered by the selected
//! entry's level); right: the selected entry's message log over a JSON payload viewer; an
//! connection-status line for the listening socket.
//!
//! Each WebSocket connection yields a CS-level entry (no connector id) plus a connector entry for
//! every `connectorId` seen in inbound traffic. The selected entry scopes the message log and the
//! action list. A single Lua sim backs the whole module (see `server/lua.rs`): its `C_OCPP` global
//! addresses every station/connector via `ChargingStation(cs)` / `Connector(cs, id)` over a shared
//! state registry the view keeps in step with its entries.
//!
//! Split by concern: rows/types/the `ServerView` struct live here; [`mod@render`] holds frame
//! rendering + widget builders, [`mod@input`] key handling, [`mod@backend`] the sim/queue/refresh
//! glue.

mod backend;
mod input;
mod render;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::state::{ButtonState, CodeInputFieldState, SelectionState, TableState};
use ferrowl_ui::widgets::{Button, CodeInputField, Selection, Table, Widget};
use ferrowl_ui::{COLOR_SCHEME, EventResult};
use ferrowl_ui_derive::{Focus, Overlay, TableEntry, focusable};
use ratatui::style::Style;

use ferrowl_ocpp::Version;
use ferrowl_ocpp::csms::{ConnectionId, CsmsActionHandler};

use crate::app::LogRing;
use crate::config::script::ScriptDef;
use crate::dialog::scripts::ScriptDialog;
use crate::module::modbus::dialog::ConfirmDeleteDialog;
use crate::module::ocpp::action_dialog::ActionDialog;
use crate::module::ocpp::client::backend::{MsgHeader, MsgRow, OcppMessage, msg_row};
use crate::module::ocpp::client::lua_sim::OcppFields;
use crate::module::ocpp::config::device::{ConnectorRfids, OcppDeviceConfig};
use crate::module::ocpp::config::session::{OcppModuleSpec, OcppSpec};
use crate::module::ocpp::lock::{with_state, with_state_mut};
use crate::module::ocpp::server::backend::{
    EventRx, EventTx, OcppServer, RfidLists, RfidStore, Scope,
};
use crate::module::ocpp::server::detail::DetailOverlay;
use crate::module::ocpp::server::lua::{ServerActionQueue, ServerStates, SharedServerStates};
use crate::module::ocpp::setup_dialog::OcppSetupDialog;
use crate::module::view::{CommandDescriptor, CommandSpec, ModuleView, SharedLog};

/// Build the runtime RFID store from a persisted device config (CS list + per-connector lists).
fn rfid_store_from_device(device: &OcppDeviceConfig) -> RfidStore {
    let mut store = RfidStore {
        cs: device.rfids.clone(),
        ..Default::default()
    };
    for cr in &device.connector_rfids {
        let scope = Scope {
            evse: cr.evse,
            connector: cr.connector,
        };
        store.by_scope.insert(scope, cr.rfids.clone());
    }
    store
}

/// Write the runtime RFID store back into a device config for persistence, dropping empty
/// per-connector lists.
pub(super) fn fill_device_rfids(device: &mut OcppDeviceConfig, store: &RfidStore) {
    device.rfids.clone_from(&store.cs);
    let mut conns: Vec<ConnectorRfids> = store
        .by_scope
        .iter()
        .filter(|(_, list)| !list.is_empty())
        .map(|(scope, list)| ConnectorRfids {
            evse: scope.evse,
            connector: scope.connector,
            rfids: list.clone(),
        })
        .collect();
    // Stable order so saves are deterministic.
    conns.sort_by_key(|c| (c.evse, c.connector));
    device.connector_rfids = conns;
}

/// Per-entry observed state behaviour the generic view needs from each version's state types.
pub trait EntryStateT: OcppFields + Default + Send + Sync + 'static {
    /// Update the observed state from an inbound CS→CSMS request and the CSMS response (e.g.
    /// StartTransaction's transactionId is minted in the response).
    fn apply_inbound(
        &mut self,
        name: &str,
        request: &serde_json::Value,
        response: &serde_json::Value,
    );
    /// Update the observed state from a CSMS→CS request we *sent* and the CS response (e.g. mirror an
    /// accepted SetChargingProfile's limit). Default no-op; connector states override.
    fn apply_outbound(
        &mut self,
        _name: &str,
        _request: &serde_json::Value,
        _response: &serde_json::Value,
    ) {
    }
    /// Derive a complete outbound payload for `name` from observed state (e.g. `idTag` from the
    /// last RFID, the connector/EVSE id from `scope`), or `None` to fall back to the JSON editor.
    fn derive_payload(&self, name: &str, scope: Scope) -> Option<serde_json::Value>;
    /// Ordered (field, unit, value) rows describing the observed non-metering state, for the detail
    /// overlay's "State" table. `unit` is empty for non-dimensional fields.
    fn fields(&self) -> Vec<(String, String, String)>;
    /// Ordered (field, unit, value) metering rows for the detail overlay's "Metering" table (default
    /// empty; connector states override).
    fn metering(&self) -> Vec<(String, String, String)> {
        Vec::new()
    }
}

/// Everything version-specific the generic server view needs.
pub trait ServerVersion: Version + Sized + 'static {
    /// CS-level observed state (non-connector info: model/vendor/firmware).
    type Cs: EntryStateT;
    /// Per-connector observed state (metering/status/transaction).
    type Conn: EntryStateT;
    /// The inbound handler answering CS→CSMS Calls and emitting [`ServerEvent`]s.
    type Handler: CsmsActionHandler<Self>;

    /// Build the inbound handler, wiring it to the view's event channel and RFID accept-list.
    fn handler(tx: EventTx, rfids: RfidLists) -> Self::Handler;

    /// The scope an inbound request targets (CS-level/connector/EVSE), used to bucket it.
    fn inbound_connector(name: &str, request: &serde_json::Value) -> Scope;

    /// The transactionId a stop message clears (1.6 `StopTransaction`, 2.0.1 `TransactionEvent`
    /// with `eventType == "Ended"`), as a string, or `None` for non-stop messages. Used to route a
    /// stop that carries no connector/EVSE id to the connector holding that transaction.
    fn stop_tx_id(_name: &str, _request: &serde_json::Value) -> Option<String> {
        None
    }

    /// The CSMS action that retrieves configuration (`GetConfiguration` for 1.6, `GetVariables` for
    /// 2.0.1).
    fn config_action() -> &'static str;

    /// Build a config-fetch request payload for a free-form key (empty = "all" where supported).
    fn config_request(key: &str) -> serde_json::Value;

    /// Parse a config-fetch response into ordered (key, value, readonly) rows.
    fn parse_config(response: &serde_json::Value) -> Vec<(String, String, bool)>;

    /// The CSMS action that writes one configuration value (`ChangeConfiguration` for 1.6,
    /// `SetVariables` for 2.0.1).
    fn set_action() -> &'static str;

    /// Build a config-write request payload setting `key` to `value`.
    fn set_request(key: &str, value: &str) -> serde_json::Value;

    /// The per-action send-dialog spec for `name`, or `None` (raw JSON editor).
    fn action_spec(name: &str) -> Option<crate::module::ocpp::action_dialog::ActionSpec>;

    /// Dialog-reachable actions that intentionally use the raw JSON editor (no typed form yet).
    fn json_actions() -> &'static [&'static str];

    /// A handcrafted example payload prefilling the raw JSON editor for a [`Self::json_actions`]
    /// entry (falls back to the serde-`Default` skeleton when `None`).
    fn json_template(name: &str) -> Option<serde_json::Value>;

    /// Inject the target scope's connector/EVSE id into a (Lua-built) payload that does not already
    /// carry it, so e.g. `con:SetChargingProfile()` defaults to the selected connector. No-op for
    /// the CS-level scope or a non-object payload.
    fn inject_scope(_payload: &mut serde_json::Value, _scope: Scope) {}

    /// The id Lua addresses a connector entry by in `C_OCPP:Connector(cs, id)` / `GetConnectors`.
    /// 1.6 uses the connector id; 2.0.1 uses the EVSE id (connectors are always `None` there).
    /// `None` for the CS-level scope, which Lua does not address as a connector.
    fn lua_connector_id(scope: Scope) -> Option<i64> {
        scope.connector
    }

    /// Whether config keys carry a component dimension (2.0.1 `Component/Variable`), so the detail
    /// overlay's config table shows a separate "Component" column. 1.6 keys are flat.
    fn config_has_component() -> bool {
        false
    }
}

// --- Connection table ------------------------------------------------------

#[derive(Clone, Debug, TableEntry)]
#[table_entry(header = CsHeader, styles = cs_cell_styles)]
struct CsRow {
    #[column(name = "Charging Station", min = 18, max = 40)]
    name: String,
    #[column(name = "Connector", min = 9, max = 9)]
    connector: String,
    #[column(name = "State", min = 12, max = 12)]
    state: String,
}

fn cs_cell_styles(row: &CsRow) -> [Option<Style>; 3] {
    let style = match row.state.as_str() {
        "Connected" => Some(Style::default().fg(COLOR_SCHEME.success)),
        "Disconnected" => Some(Style::default().fg(COLOR_SCHEME.error)),
        _ => None,
    };
    [None, None, style]
}

// --- Entries ---------------------------------------------------------------

/// Shared observed state of one entry — a CS-level entry or a connector entry.
enum EntryState<V: ServerVersion> {
    Cs(Arc<RwLock<V::Cs>>),
    Conn(Arc<RwLock<V::Conn>>),
}

/// One row in the connection table: a charge point (CS-level) or one of its connectors.
struct Entry<V: ServerVersion> {
    /// Charge-point identity (URL-path segment, or a peer fallback).
    identity: String,
    /// The entry scope: CS-level, a 1.6 connector, or a 2.0.1 EVSE/connector.
    scope: Scope,
    /// The live connection while online.
    conn: Option<ConnectionId>,
    online: bool,
    state: EntryState<V>,
    messages: Vec<OcppMessage>,
}

impl<V: ServerVersion> Entry<V> {
    fn rows_for_state(&self) -> Vec<OcppMessage> {
        self.messages.clone()
    }

    fn apply_inbound(
        &mut self,
        name: &str,
        request: &serde_json::Value,
        response: &serde_json::Value,
    ) {
        match &self.state {
            EntryState::Cs(s) => with_state_mut(s, |s| s.apply_inbound(name, request, response)),
            EntryState::Conn(s) => with_state_mut(s, |s| s.apply_inbound(name, request, response)),
        }
    }

    fn apply_outbound(
        &mut self,
        name: &str,
        request: &serde_json::Value,
        response: &serde_json::Value,
    ) {
        match &self.state {
            EntryState::Cs(s) => with_state_mut(s, |s| s.apply_outbound(name, request, response)),
            EntryState::Conn(s) => with_state_mut(s, |s| s.apply_outbound(name, request, response)),
        }
    }

    fn derive_payload(&self, name: &str) -> Option<serde_json::Value> {
        match &self.state {
            EntryState::Cs(s) => with_state(s, |s| s.derive_payload(name, self.scope)),
            EntryState::Conn(s) => with_state(s, |s| s.derive_payload(name, self.scope)),
        }
    }

    /// Read an observed-state field as a display string, for action-dialog prefill.
    fn get_field_str(&self, name: &str) -> Option<String> {
        use crate::module::ocpp::action_dialog::value_to_string;
        let v = match &self.state {
            EntryState::Cs(s) => with_state(s, |s| s.get_field(name)),
            EntryState::Conn(s) => with_state(s, |s| s.get_field(name)),
        };
        v.map(value_to_string)
    }
}

// --- View ------------------------------------------------------------------

type CsTable = Widget<TableState<CsRow, 3>, Table<CsRow, CsHeader, 3>>;
type MsgTable = Widget<TableState<MsgRow, 5>, Table<MsgRow, MsgHeader, 5>>;

/// Results produced mid-tick and consumed by a later `refresh`: a queued module re-setup or a
/// built replacement view (version/role switch).
#[derive(Default)]
struct Deferred {
    setup: Option<(OcppSpec, String, Vec<ferrowl_ocpp::HeaderDef>)>,
    replacement: Option<Box<dyn ModuleView>>,
}

/// Simulation + liveness bookkeeping: the single Lua sim handle, its action queue, and the
/// log tee/`:log` tracking advanced each `refresh`.
#[derive(Default)]
struct SimRuntime {
    handle: Option<crate::module::ocpp::client::lua_sim::OcppSimHandle>,
    /// Actions enqueued by the Lua sim (identity + scope), drained and routed each refresh.
    lua_queue: ServerActionQueue,
    /// Highest message `seq` already teed into the persistent log, so each is logged once.
    logged_seq: u64,
    /// The `log_file` currently applied to the `SharedLog`, to detect `:log`/edit changes.
    applied_log_file: Option<String>,
}

/// The single modal overlay over the server view (mutually exclusive by construction: entering `:`
/// command mode — the only other way to open one, `setup` — is itself gated on
/// `is_overlay_active()`, see `app/keys.rs`). The derive supplies `is_active`/`take`/`close` and
/// common-key routing (`Esc` closes, `Tab`/`BackTab` cycle focus on the tagged variants); each
/// variant's `Enter`/inner dispatch stays in `handle_events`.
#[derive(Overlay)]
enum ServerOverlay {
    #[overlay(none)]
    None,
    /// Per-entry detail overlay (routes every key through its own `input()`).
    Detail(Box<DetailOverlay>),
    /// Delete-confirmation dialog for the focused CS-table entry.
    #[overlay(esc_close, focus_cycle)]
    Confirm(Box<ConfirmDeleteDialog>),
    /// Module re-setup dialog.
    #[overlay(focus_cycle)]
    Setup(Box<OcppSetupDialog>),
    /// Lua scripts editor (routes every key through its own `handle_events()`).
    Scripts(Box<ScriptDialog>),
    /// Action send dialog: target connection/scope + the dialog (routes every key via `input()`).
    Action(Box<(ConnectionId, Scope, ActionDialog)>),
}

ferrowl_ui::impl_overlay_keys!(ConfirmDeleteDialog);

#[focusable]
#[derive(Focus)]
pub struct ServerView<V: ServerVersion> {
    spec: OcppSpec,
    device_path: String,
    device: OcppDeviceConfig,
    log: SharedLog,
    /// Dedicated ring for Lua sim output (`C_Log:*`/`print()`) and sim lifecycle messages,
    /// separate from `log`'s connection/status/traffic lines.
    script_log: SharedLog,
    backend: OcppServer<V>,
    events_tx: EventTx,
    events_rx: EventRx,
    entries: Vec<Entry<V>>,
    /// conn → resolved charge-point identity, cached as events arrive.
    conn_identity: HashMap<ConnectionId, String>,
    #[focus]
    cs_table: CsTable,
    #[focus]
    scripts_button: Widget<ButtonState, Button>,
    #[focus]
    actions: Widget<SelectionState<String>, Selection<String>>,
    /// Whether the action list is currently built for a connector entry (`Some(true)`), a CS-level
    /// entry (`Some(false)`), or not yet built (`None`) — to avoid rebuilding every tick.
    actions_for_connector: Option<bool>,
    #[focus]
    msg_table: MsgTable,
    #[focus]
    code: Widget<CodeInputFieldState, CodeInputField>,
    /// The single active modal overlay (detail / delete-confirm / setup / scripts / action).
    overlay: ServerOverlay,
    /// Results produced mid-tick, consumed by a later `refresh` (re-setup / replacement).
    deferred: Deferred,
    /// Whether the listener should be running (auto-bind on open; toggled by `:start`/`:stop`).
    want_running: bool,
    /// Last content pushed into the payload viewer, so periodic refreshes don't reset its scroll.
    code_content: String,
    /// Shared RFID accept-lists (CS + per-connector) handed to each (re)built inbound handler;
    /// edited via the detail dialogs and `:rfid`.
    rfids: RfidLists,
    /// Compact table rows (no vertical margin); toggled by `:compact`.
    compact: bool,
    /// In-memory per-CS configuration rows (identity → key/value), kept across overlay open/close
    /// only while the CS is in the list; dropped when its entry is removed (delete/`:stop`/`:restart`).
    cs_configs: HashMap<String, Vec<(String, String, bool)>>,
    /// Shared registry of every entry's observed state, read by the single Lua sim.
    lua_states: SharedServerStates<V>,
    /// Simulation + liveness bookkeeping (sim handle, action queue, log tee/`:log` tracking).
    runtime: SimRuntime,
}

impl<V: ServerVersion> ServerView<V>
where
    V::Action: Clone,
{
    pub fn new(spec: OcppSpec, device_path: String, device: OcppDeviceConfig) -> Self {
        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        let rfids: RfidLists = Arc::new(parking_lot::RwLock::new(rfid_store_from_device(&device)));
        let mut view = Self {
            backend: OcppServer::new(),
            spec,
            device_path,
            device,
            log: Arc::new(tokio::sync::RwLock::new(LogRing::init())),
            script_log: Arc::new(tokio::sync::RwLock::new(LogRing::init())),
            events_tx,
            events_rx,
            entries: Vec::new(),
            conn_identity: HashMap::new(),
            cs_table: render::cs_table(),
            scripts_button: render::scripts_button(),
            actions: render::action_list(Vec::new()),
            actions_for_connector: None,
            msg_table: render::msg_table(),
            code: render::code_view(),
            overlay: ServerOverlay::None,
            deferred: Deferred::default(),
            focus: ServerViewFocus::CsTable,
            view_focused: false,
            want_running: true,
            code_content: String::new(),
            rfids,
            compact: false,
            cs_configs: HashMap::new(),
            lua_states: Arc::new(RwLock::new(ServerStates::default())),
            runtime: SimRuntime::default(),
        };
        view.start_sim();
        view
    }

    /// Enabled scripts as `(name, code)` pairs for the simulation thread.
    fn enabled_scripts(&self) -> Vec<(String, String)> {
        self.device
            .scripts
            .iter()
            .filter(|s| s.enabled)
            .map(|s| (s.name.clone(), s.code.clone()))
            .collect()
    }
}

impl<V: ServerVersion> ModuleView for ServerView<V>
where
    V::Action: Clone,
{
    fn name(&self) -> String {
        self.spec.name.clone()
    }

    fn is_overlay_active(&self) -> bool {
        self.overlay.is_active() || self.deferred.setup.is_some()
    }

    fn render(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        self.render_impl(frame, area);
    }

    fn render_overlay(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        self.render_overlay_impl(frame, area);
    }

    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        self.handle_events_impl(modifiers, code)
    }

    fn refresh<'a>(&'a mut self) -> crate::module::view::RefreshFuture<'a> {
        self.refresh_impl()
    }

    fn handle_command<'a>(&'a mut self, cmd: &'a str) -> crate::module::view::CommandFuture<'a> {
        self.handle_command_impl(cmd)
    }

    fn commands(&self) -> &[CommandDescriptor] {
        static DESCRIPTORS: std::sync::OnceLock<Vec<CommandDescriptor>> =
            std::sync::OnceLock::new();
        DESCRIPTORS.get_or_init(|| {
            OCPP_SERVER_COMMAND_SPECS
                .iter()
                .map(|s| s.descriptor)
                .chain(OCPP_SERVER_EXTRA_HELP.iter().copied())
                .collect()
        })
    }

    fn keybinds(&self) -> &[CommandDescriptor] {
        &OCPP_SERVER_KEYBINDS
    }

    fn log(&self) -> SharedLog {
        self.log.clone()
    }

    fn session_spec(&self) -> Option<serde_json::Value> {
        let module = OcppModuleSpec::from_spec(&self.spec, &self.device_path);
        let mut v = serde_json::to_value(&module).ok()?;
        v.as_object_mut()?.insert("type".into(), "ocpp".into());
        Some(v)
    }

    fn take_replacement(&mut self) -> Option<Box<dyn ModuleView>> {
        self.deferred.replacement.take()
    }

    fn scripts(&self) -> Option<&[ScriptDef]> {
        Some(&self.device.scripts)
    }

    fn set_scripts(&mut self, scripts: Vec<ScriptDef>) -> bool {
        self.device.scripts = scripts;
        self.start_sim();
        true
    }

    fn module_host(&self) -> Option<std::sync::Arc<dyn ferrowl_lua::module::ModuleHost>> {
        Some(std::sync::Arc::new(crate::registry::OcppServerEntry {
            states: self.lua_states.clone(),
            queue: self.runtime.lua_queue.clone(),
        }))
    }
}

static OCPP_SERVER_KEYBINDS: [CommandDescriptor; 3] = [
    CommandDescriptor {
        name: "Tab / Shift+Tab",
        description: "next / previous pane",
    },
    CommandDescriptor {
        name: "Enter",
        description: "open detail / scripts / trigger action",
    },
    CommandDescriptor {
        name: "d",
        description: "delete selected charging station",
    },
];

/// The parsed form of every command the CSMS server view accepts; produced by [`parse_command`]
/// over [`OCPP_SERVER_COMMAND_SPECS`]. The exhaustive `match` in `handle_command_impl` is what
/// guarantees a table entry cannot exist without a handler.
pub(super) enum OcppServerCmd {
    Start,
    Stop,
    Restart,
    Edit,
    Compact,
    WriteDevice(Option<String>),
    Log(Option<String>),
    Rfid(Option<String>),
}

/// Single source for this view's commands: aliases, help row, and parse target per entry.
pub(super) static OCPP_SERVER_COMMAND_SPECS: [CommandSpec<OcppServerCmd>; 8] = [
    CommandSpec {
        aliases: &["start"],
        descriptor: CommandDescriptor {
            name: ":start",
            description: "bind the CSMS listener",
        },
        build: |_| OcppServerCmd::Start,
    },
    CommandSpec {
        aliases: &["stop"],
        descriptor: CommandDescriptor {
            name: ":stop",
            description: "unbind the CSMS listener",
        },
        build: |_| OcppServerCmd::Stop,
    },
    CommandSpec {
        aliases: &["restart"],
        descriptor: CommandDescriptor {
            name: ":restart",
            description: "rebind the listener (clears entries)",
        },
        build: |_| OcppServerCmd::Restart,
    },
    CommandSpec {
        aliases: &["e", "edit"],
        descriptor: CommandDescriptor {
            name: ":e | :edit",
            description: "edit module setup",
        },
        build: |_| OcppServerCmd::Edit,
    },
    CommandSpec {
        aliases: &["wd", "write-device"],
        descriptor: CommandDescriptor {
            name: ":wd | :write-device [path]",
            description: "save device config",
        },
        build: |rest| OcppServerCmd::WriteDevice(rest.map(str::to_string)),
    },
    CommandSpec {
        aliases: &["compact"],
        descriptor: CommandDescriptor {
            name: ":compact",
            description: "toggle compact rows",
        },
        build: |_| OcppServerCmd::Compact,
    },
    CommandSpec {
        aliases: &["log"],
        descriptor: CommandDescriptor {
            name: ":log [file]",
            description: "set/clear log file",
        },
        build: |rest| OcppServerCmd::Log(rest.map(str::to_string)),
    },
    CommandSpec {
        aliases: &["rfid"],
        descriptor: CommandDescriptor {
            name: ":rfid [add|del <tag> | clear]",
            description: "CSMS RFID accept-list",
        },
        build: |rest| OcppServerCmd::Rfid(rest.map(str::to_string)),
    },
];

/// Display-only help rows appended after the command rows (keys, not `:` commands).
static OCPP_SERVER_EXTRA_HELP: [CommandDescriptor; 2] = [
    CommandDescriptor {
        name: "d",
        description: "delete the selected charging station / connector",
    },
    CommandDescriptor {
        name: "Enter",
        description: "open the selected entry's detail overlay",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::ocpp::config::session::{OcppProtocol, OcppRole, OcppVersion};
    use crate::module::ocpp::server::backend::ServerEvent;
    use ferrowl_ocpp::V1_6;
    use ferrowl_test_support::{reserve_tcp_port, reserve_temp_dir};

    #[test]
    fn ut_rfid_store_device_roundtrip() {
        // Device config -> runtime store -> device config preserves CS + per-connector lists.
        let device = OcppDeviceConfig {
            rfids: vec!["CS1".into()],
            connector_rfids: vec![ConnectorRfids {
                evse: Some(2),
                connector: None,
                rfids: vec!["EVSE2".into()],
            }],
            ..Default::default()
        };
        let store = rfid_store_from_device(&device);
        assert_eq!(store.cs, ["CS1"]);
        assert_eq!(store.scope_list(Scope::evse(2, None)), ["EVSE2"]);

        let mut back = OcppDeviceConfig::default();
        fill_device_rfids(&mut back, &store);
        assert_eq!(back.rfids, device.rfids);
        assert_eq!(back.connector_rfids, device.connector_rfids);
    }

    #[test]
    fn ut_empty_connector_lists_not_persisted() {
        // A connector whose list was emptied is dropped from the persisted config.
        let mut store = RfidStore::default();
        store.add(Scope::connector(1), "X".into());
        store.remove(Scope::connector(1), "X");
        let mut device = OcppDeviceConfig::default();
        fill_device_rfids(&mut device, &store);
        assert!(device.connector_rfids.is_empty());
    }

    fn server_view() -> ServerView<V1_6> {
        let spec = OcppSpec {
            name: "csms".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Server,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 0,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: Default::default(),
        };
        ServerView::<V1_6>::new(spec, String::new(), OcppDeviceConfig::default())
    }

    /// Poll until the CSMS listener has bound: `start()` binds asynchronously, retrying
    /// a failed bind with backoff (OC-R-083), so `bound_addr()` is `None` until the first
    /// successful bind lands.
    async fn poll_bound_addr(
        backend: &crate::module::ocpp::server::backend::OcppServer<V1_6>,
    ) -> Option<String> {
        for _ in 0..50 {
            if let Some(addr) = backend.bound_addr() {
                return Some(addr);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        None
    }

    // Applying a resolved `:edit` updates the view's spec — the single source the backend binds
    // from on every `start(&spec, ..)` — and stops the running listener, so the next start rebinds
    // with the edited endpoint and security. Hence the wss + Basic Auth edit below: the rebind
    // must not leave a plain unauthenticated listener bound.
    #[tokio::test]
    /// OC-R-138 — applying an edit updates the module spec and stops its listener; the listener rebinds on the next tick without an explicit `:start`.
    async fn ut_edit_apply_updates_spec_and_stops_listener() {
        let mut v = server_view();
        let mut edited = v.spec.clone();
        edited.protocol = OcppProtocol::Wss;
        edited.security.username = Some("username".into());
        edited.security.password = Some("password".into());
        v.deferred.setup = Some((edited.clone(), String::new(), Vec::new()));
        v.refresh_impl().await;
        assert_eq!(v.spec, edited);
        assert!(v.spec.csms_self_signed_fallback());
        // The same tick stops the old listener and rebinds from the edited spec (want_running
        // is on by default); OC-R-083's bind is async, so poll rather than asserting `is_online()`
        // synchronously.
        assert!(
            poll_bound_addr(&v.backend).await.is_some(),
            "edit must rebind the listener"
        );
    }

    #[tokio::test]
    /// UI-R-099 — an edited header list survives an in-place setup confirm on the server view
    /// too: `refresh_impl` must apply the dialog's `extra_headers` onto the reconfigured
    /// device (the server role ignores the value at runtime, but it must still round-trip
    /// through an edit rather than being silently dropped).
    async fn ut_edit_confirm_carries_dialog_extra_headers_onto_device() {
        let mut v = server_view();
        let edited = v.spec.clone(); // same role: in-place reconfigure, not a replacement
        let headers = vec![ferrowl_ocpp::HeaderDef::new("X-Tenant", "acme-1").unwrap()];
        v.deferred.setup = Some((edited, String::new(), headers.clone()));
        v.refresh_impl().await;
        assert_eq!(v.device.extra_headers, headers);
    }

    #[test]
    /// UI-R-049 — the server view's focus cycle includes the payload pane.
    fn focus_cycle_includes_payload_pane() {
        let mut v = server_view();
        // CsTable -> Scripts -> Actions -> Messages -> Payload -> CsTable.
        let mut seen = Vec::new();
        for _ in 0..5 {
            v.focus_next();
            seen.push(v.focus);
        }
        assert!(
            seen.contains(&ServerViewFocus::Code),
            "Payload pane not in Tab order"
        );
        assert!(
            v.focus == ServerViewFocus::CsTable,
            "focus_next did not wrap to start"
        );
        // BackTab from CsTable lands on Payload (reverse order).
        v.focus_previous();
        assert!(v.focus == ServerViewFocus::Code);
    }

    #[test]
    fn entries_keyed_by_scope() {
        let mut v = server_view();
        // Two 2.0.1 connectors sharing EVSE 1 are distinct entries; re-querying is stable.
        let c1 = v.entry_index("CP1", Scope::evse(1, Some(1)), None);
        let c2 = v.entry_index("CP1", Scope::evse(1, Some(2)), None);
        assert_ne!(
            c1, c2,
            "connectors sharing an EVSE must be distinct entries"
        );
        assert_eq!(c1, v.entry_index("CP1", Scope::evse(1, Some(1)), None));
        // 1.6-style keying (CS-level vs connector) is unchanged.
        let cs = v.entry_index("CP1", Scope::CS, None);
        let conn = v.entry_index("CP1", Scope::connector(1), None);
        assert_ne!(cs, conn);
        assert_eq!(conn, v.entry_index("CP1", Scope::connector(1), None));
    }

    #[test]
    /// UI-R-021 — opening detail builds an overlay for the selected entry.
    fn open_detail_builds_overlay_for_selected_entry() {
        let mut v = server_view();
        // Add a CS-level entry and select its row.
        v.entry_index("CP1", Scope::CS, None);
        v.cs_table.state.set_values(vec![CsRow {
            name: "CP1".into(),
            connector: String::new(),
            state: "Disconnected".into(),
        }]);
        v.cs_table.state.move_down();
        assert!(!v.is_overlay_active());
        v.open_detail();
        assert!(v.is_overlay_active(), "detail overlay should be active");
        let ServerOverlay::Detail(d) = &v.overlay else {
            panic!("detail overlay open")
        };
        assert_eq!(d.identity, "CP1");
        assert!(d.is_cs, "CS-level entry should yield a CS detail overlay");
    }

    fn get_config_event(conn: ConnectionId) -> ServerEvent {
        ServerEvent::Outbound {
            conn,
            scope: Scope::CS,
            name: "GetConfiguration".into(),
            request: serde_json::json!({}),
            response: serde_json::json!({ "configurationKey": [
                { "key": "HeartbeatInterval", "value": "30", "readonly": true }
            ]}),
            ok: true,
            context: String::new(),
        }
    }

    #[test]
    fn stop_transaction_clears_connector_tx_via_tx_id_routing() {
        let mut v = server_view();
        let conn = ConnectionId(1);
        v.conn_identity.insert(conn, "CP1".into());
        // Start a transaction on connector 1; the CSMS response mints transactionId 77.
        v.events_tx
            .send(ServerEvent::Inbound {
                conn,
                name: "StartTransaction".into(),
                request: serde_json::json!({ "connectorId": 1, "idTag": "T" }),
                response: serde_json::json!({ "transactionId": 77 }),
            })
            .unwrap();
        v.drain_events();
        let idx = v.entry_index("CP1", Scope::connector(1), Some(conn));
        assert_eq!(
            v.entries[idx].get_field_str("TransactionId").as_deref(),
            Some("77"),
            "connector 1 should hold the started transaction"
        );
        // StopTransaction carries no connectorId, only the transactionId. It buckets to CS scope but
        // must be re-routed to connector 1 (which holds tx 77) so its transaction id clears.
        v.events_tx
            .send(ServerEvent::Inbound {
                conn,
                name: "StopTransaction".into(),
                request: serde_json::json!({ "transactionId": 77 }),
                response: serde_json::Value::Null,
            })
            .unwrap();
        v.drain_events();
        let idx = v.entry_index("CP1", Scope::connector(1), Some(conn));
        assert_eq!(
            v.entries[idx].get_field_str("TransactionId").as_deref(),
            Some(""),
            "connector 1's transaction id should be cleared on stop"
        );
    }

    #[test]
    fn get_configuration_response_populates_config_table() {
        let mut v = server_view();
        let conn = ConnectionId(1);
        v.conn_identity.insert(conn, "CP1".into());
        v.entry_index("CP1", Scope::CS, Some(conn));
        v.cs_table.state.set_values(vec![CsRow {
            name: "CP1".into(),
            connector: String::new(),
            state: "Connected".into(),
        }]);
        v.cs_table.state.move_down();
        v.open_detail();
        v.events_tx.send(get_config_event(conn)).unwrap();
        v.drain_events();
        let ServerOverlay::Detail(d) = &v.overlay else {
            panic!("detail overlay open")
        };
        assert_eq!(
            d.config_rows(),
            vec![("HeartbeatInterval".into(), "30".into(), true)]
        );
    }

    #[test]
    fn get_configuration_response_persists_with_overlay_closed() {
        let mut v = server_view();
        let conn = ConnectionId(1);
        v.conn_identity.insert(conn, "CP1".into());
        v.entry_index("CP1", Scope::CS, Some(conn));
        // No overlay open when the response arrives (e.g. triggered via the action button).
        v.events_tx.send(get_config_event(conn)).unwrap();
        v.drain_events();
        assert_eq!(
            v.cs_configs.get("CP1").unwrap(),
            &vec![("HeartbeatInterval".into(), "30".into(), true)]
        );
        // Opening the detail later seeds the table from the persisted rows.
        v.cs_table.state.set_values(vec![CsRow {
            name: "CP1".into(),
            connector: String::new(),
            state: "Connected".into(),
        }]);
        v.cs_table.state.move_down();
        v.open_detail();
        let ServerOverlay::Detail(d) = &v.overlay else {
            panic!("detail overlay open")
        };
        assert_eq!(
            d.config_rows(),
            vec![("HeartbeatInterval".into(), "30".into(), true)]
        );
    }

    #[test]
    fn config_keys_persist_across_overlay_close() {
        let mut v = server_view();
        v.entry_index("CP1", Scope::CS, None);
        v.cs_table.state.set_values(vec![CsRow {
            name: "CP1".into(),
            connector: String::new(),
            state: "Disconnected".into(),
        }]);
        v.open_detail();
        // Simulate a fetched config row merged into the overlay.
        let ServerOverlay::Detail(d) = &mut v.overlay else {
            panic!("detail overlay open")
        };
        d.merge_config("HeartbeatInterval".into(), "30".into(), false);
        // Close the overlay (Esc, confirm with Enter) — keep rows in memory, not discard.
        // Qualified: ServerView now also derives `HandleEvents` via `#[derive(Focus)]`.
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);
        assert!(!v.overlay.is_active());
        assert_eq!(
            v.cs_configs.get("CP1").unwrap(),
            &vec![("HeartbeatInterval".into(), "30".into(), false)]
        );
        // Reopening seeds the overlay from the in-memory rows.
        v.open_detail();
        let ServerOverlay::Detail(d) = &v.overlay else {
            panic!("detail overlay open")
        };
        assert_eq!(
            d.config_rows(),
            vec![("HeartbeatInterval".into(), "30".into(), false)]
        );
        v.overlay = ServerOverlay::None;
        // Deleting the CS drops its stored config.
        v.delete_selected();
        assert!(!v.cs_configs.contains_key("CP1"));
    }

    // --- Esc-confirm migration: Setup overlay; Confirm keeps Esc ----------------------------

    #[test]
    /// UI-R-023 — Esc opens the close-confirm on the setup overlay; Enter closes it.
    fn setup_overlay_esc_opens_confirm_enter_closes() {
        let mut v = server_view();
        v.overlay = ServerOverlay::Setup(Box::new(
            crate::module::ocpp::setup_dialog::OcppSetupDialog::new(),
        ));
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        assert!(
            v.overlay.is_active(),
            "Esc must not close the setup dialog outright (opens close-confirm)"
        );
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);
        assert!(
            !v.overlay.is_active(),
            "Enter in close-confirm must close the setup dialog"
        );
    }

    #[test]
    /// UI-R-023 — the confirm overlay still closes on Esc.
    fn confirm_overlay_still_closes_on_esc() {
        let mut v = server_view();
        v.overlay = ServerOverlay::Confirm(Box::new(ConfirmDeleteDialog::new("CP1")));
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        assert!(
            !v.overlay.is_active(),
            "Esc must still close the delete-confirmation dialog"
        );
    }

    #[test]
    /// UI-R-013 — syncing actions preserves the table selection when nothing is selected.
    fn sync_actions_preserves_selection_when_no_entry_selected() {
        let mut v = server_view();
        // First sync builds the CS-level action list.
        v.sync_actions();
        assert!(
            v.actions.state.values().len() > 1,
            "need >1 action to exercise selection movement"
        );
        // Move the selection off the top.
        v.actions.state.move_down();
        let chosen = v.actions.state.selection();
        assert_ne!(chosen, 0);
        // A later sync with no CS entry selected must leave the list alone: rebuilding it would
        // snap the selection back to the top on every frame.
        v.sync_actions();
        assert_eq!(
            v.actions.state.selection(),
            chosen,
            "selection reset — sync_actions rebuilt the list with no selection present"
        );
    }

    // --- Module lifecycle: listener rebuild / restart ---------------------------------------

    #[tokio::test]
    /// ocpp/api-contract CSMS command table — `:write-device [path]` is the long spelling of
    /// `:wd` and saves the device config the same way.
    async fn write_device_alias_saves_like_wd() {
        use crate::module::view::CommandResult;
        let mut v = server_view();
        let dir = reserve_temp_dir("ferrowl_ocpp_server_view");
        let path = dir.join("device.toml");
        let p = path.to_str().expect("temp path is valid UTF-8").to_string();
        let CommandResult::Handled(Some((_, msg))) =
            v.handle_command_impl(&format!("write-device {p}")).await
        else {
            panic!(":write-device must be handled with a message");
        };
        assert!(msg.contains("Saved device config"), "unexpected: {msg}");
        assert!(path.exists());
    }

    #[tokio::test]
    /// OC-R-082 — the listener configuration is rebuilt from the current module spec on every
    /// start, so an edited security section takes effect on the next start without a stale copy.
    async fn each_start_rebuilds_listener_from_current_spec() {
        use crate::module::view::CommandResult;
        let mut v = server_view();

        // First start: plain ws, so the started message names no TLS fallback.
        let CommandResult::Handled(Some((_, msg))) = v.handle_command_impl("start").await else {
            panic!("start must report a message");
        };
        assert!(
            !msg.contains("self-signed"),
            "a plain listener must not report a self-signed certificate, got: {msg}"
        );
        // OC-R-083: `start()` binds the listener asynchronously — poll until it is bound.
        assert!(
            poll_bound_addr(&v.backend).await.is_some(),
            "the listener must bind promptly against a free port"
        );

        // Edit the endpoint's security to wss (no certs → self-signed fallback), then restart.
        // The rebound listener must reflect the *current* spec, not the stale plain copy.
        v.spec.protocol = OcppProtocol::Wss;
        let CommandResult::Handled(Some((_, msg))) = v.handle_command_impl("restart").await else {
            panic!("restart must report a message");
        };
        assert!(
            msg.contains("self-signed"),
            "the restart must rebuild the listener from the edited spec, got: {msg}"
        );
    }

    #[tokio::test]
    /// OC-R-084 — restarting a server stops the current instance, starts a new one from the
    /// current spec, and discards every observed charging-station entry.
    async fn restart_discards_observed_entries_and_rebinds() {
        let mut v = server_view();
        v.handle_command_impl("start").await;

        // Observe a charging station: entry_index records it.
        v.entry_index("CP1", Scope::CS, None);
        assert!(!v.entries.is_empty(), "the observed entry must be recorded");

        v.handle_command_impl("restart").await;
        assert!(
            v.entries.is_empty(),
            "restart must discard every observed charging-station entry"
        );
        // OC-R-083: the listener binds asynchronously — poll until it is bound.
        assert!(
            poll_bound_addr(&v.backend).await.is_some(),
            "restart must start a new instance"
        );
    }

    #[tokio::test]
    /// OC-R-124 — a CSMS view whose bind target is already occupied shows RECONNECTING (not
    /// DISCONNECTED) while its task keeps retrying the bind, per the occupier idiom established
    /// in `instance/mod.rs`'s `it_server_stop_on_backing_off_task_ends_promptly`.
    async fn it_csms_view_shows_reconnecting_while_bind_backs_off() {
        use crate::view::status_bar::ConnStatus;
        use ferrowl_ui::COLOR_SCHEME;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let occupier = reserve_tcp_port();

        let mut v = server_view();
        v.spec.port = occupier.port();
        v.spec.timeout_ms = Some(200);
        v.spec.reconnect = Some(true);
        v.refresh_impl().await;

        for _ in 0..100 {
            if v.backend.connection_status() == ConnStatus::Reconnecting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            v.backend.connection_status(),
            ConnStatus::Reconnecting,
            "task must be backing off, never Connected, against an occupied port"
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| v.render(frame, frame.area()))
            .unwrap();
        let last_row = terminal.backend().buffer().area.height - 1;
        let contents: String = (0..120)
            .map(|x| {
                terminal.backend().buffer()[(x, last_row)]
                    .symbol()
                    .to_string()
            })
            .collect();
        assert!(
            contents.contains("RECONNECTING"),
            "backing-off server must show RECONNECTING: {contents:?}"
        );
        assert_eq!(
            terminal.backend().buffer()[(0, last_row)].bg,
            COLOR_SCHEME.warning,
            "RECONNECTING row must use the warning background"
        );

        drop(occupier);
        v.backend.stop().await.expect("cleanup stop");
    }
}
