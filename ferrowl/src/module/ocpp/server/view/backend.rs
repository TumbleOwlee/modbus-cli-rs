//! Simulation + backend glue: the Lua sim lifecycle, entry bookkeeping (create/update/sort/delete),
//! the per-tick `refresh` (deferred setup, backend + Lua-queued events, log tee) and `:` command
//! execution.

use std::sync::Arc;

use parking_lot::RwLock;

use ferrowl_ocpp::ConnectorScope;
use ferrowl_ocpp::csms::ConnectionId;

use crate::app::Level;
use crate::module::ocpp::client::build_client_view;
use crate::module::ocpp::client::lua_sim::merge_overrides;
use crate::module::ocpp::config::device::OcppDeviceConfig;
use crate::module::ocpp::config::session::OcppRole;
use crate::module::ocpp::lock::{with_state, with_state_mut};
use crate::module::ocpp::server::backend::{
    Scope, ServerEvent, TlsBinding, inbound_messages, with_rfids, with_rfids_mut,
};
use crate::module::ocpp::server::build_server_view;
use crate::module::ocpp::server::lua::{run_server_script_once, run_server_sim};
use crate::module::view::{CommandFuture, CommandResult, RefreshFuture, parse_command};

use super::{
    Entry, EntryState, EntryStateT, OCPP_SERVER_COMMAND_SPECS, OcppServerCmd, PendingLifecycle,
    ServerOverlay, ServerVersion, ServerView, fill_device_rfids,
};

/// The start/restart log line, built from the TLS mode the backend reports it actually bound
/// with — no re-derivation from the spec that could drift out of sync with the listener.
fn started_message(binding: TlsBinding, verb: &str) -> String {
    match binding {
        TlsBinding::SelfSigned => format!(
            "CSMS server {verb} (wss without certificates: using an ephemeral self-signed certificate)"
        ),
        TlsBinding::Plain | TlsBinding::Certificates => format!("CSMS server {verb}"),
    }
}

/// Encodes `V::default_action(name)` to JSON, for a Lua-queued action with no observed entry state
/// to derive its payload from. An encode failure here would otherwise silently degrade to an empty
/// payload with no trace; this logs it to stderr (the crate's existing error-reporting channel, see
/// `main.rs`/`headless.rs`) before falling back to `{}` so callers keep their existing behavior.
fn default_action_payload<V: ServerVersion>(name: &str) -> serde_json::Value {
    V::default_action(name)
        .and_then(|a| match V::encode_action(&a) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("ocpp: failed to encode default action '{name}': {e}");
                None
            }
        })
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Sort key ordering entries by station, then CS-level before its connectors, then EVSE/connector.
fn entry_sort_key<V: ServerVersion>(e: &Entry<V>) -> (String, bool, i64, i64) {
    (
        e.identity.clone(),
        e.scope.is_connector(),
        e.scope.evse.unwrap_or(0),
        e.scope.connector.unwrap_or(0),
    )
}

impl<V: ServerVersion> ServerView<V>
where
    V::Action: Clone,
{
    /// (Re)start the single Lua sim over the shared state registry (no-op if no enabled scripts).
    pub(super) fn start_sim(&mut self) {
        if let Some(mut sim) = self.runtime.handle.take() {
            sim.stop();
        }
        self.runtime.handle = run_server_sim(
            self.lua_states.clone(),
            self.runtime.lua_queue.clone(),
            self.enabled_scripts(),
            self.device.script_interval_duration(),
            self.script_log.clone(),
        );
    }

    /// Execute one script once against the shared state registry, outside the sim (SC-R-035). The
    /// sim thread is left alone, and the script runs whether or not it is enabled.
    pub(super) fn run_script_once(&mut self, name: String, code: String) {
        run_server_script_once(
            self.lua_states.clone(),
            self.runtime.lua_queue.clone(),
            name,
            code,
            self.script_log.clone(),
        );
    }

    /// Forget every station's registered state (after the entry set is cleared).
    fn clear_lua_states(&mut self) {
        with_state_mut(&self.lua_states, |s| s.stations.clear());
    }

    fn set_compact(&mut self, compact: bool) {
        self.compact = compact;
        let margin = ratatui::layout::Margin {
            vertical: if compact { 0 } else { 1 },
            horizontal: 0,
        };
        self.cs_table.widget.set_row_margin(margin);
        self.msg_table.widget.set_row_margin(margin);
    }

    /// Delete the selected entry. A CS-level entry takes its connector entries with it.
    pub(super) fn delete_selected(&mut self) {
        let Some(idx) = self.selected() else { return };
        let entry = &self.entries[idx];
        let identity = entry.identity.clone();
        let scope = entry.scope;
        if !scope.is_connector() {
            self.entries.retain(|e| e.identity != identity);
            self.cs_configs.remove(&identity);
            with_state_mut(&self.lua_states, |s| {
                s.stations.remove(&identity);
            });
        } else {
            self.entries.remove(idx);
            with_state_mut(&self.lua_states, |s| {
                if let Some(st) = s.stations.get_mut(&identity) {
                    st.conns.retain(|(sc, _)| *sc != scope);
                }
            });
        }
    }

    /// The live connection for a charge-point identity, if any entry of it is online.
    pub(super) fn conn_for(&self, identity: &str) -> Option<ConnectionId> {
        self.entries
            .iter()
            .find(|e| e.identity == identity && e.conn.is_some())
            .and_then(|e| e.conn)
    }

    /// Feed the open detail overlay live state/metering rows from its target entry.
    pub(super) fn refresh_detail(&mut self) {
        let ServerOverlay::Detail(detail) = &self.overlay else {
            return;
        };
        let (identity, scope, is_cs) = (detail.identity.clone(), detail.scope, detail.is_cs);
        let Some(entry) = self
            .entries
            .iter()
            .find(|e| e.identity == identity && e.scope == scope)
        else {
            return;
        };
        let (fields, metering) = match &entry.state {
            EntryState::Cs(s) => with_state(s, |g| (g.fields(), g.metering())),
            EntryState::Conn(s) => with_state(s, |g| (g.fields(), g.metering())),
        };
        // RFID lists for this entry: a CS entry owns the CS list (nothing inherited); a connector
        // owns its own list and inherits the CS list (shown read-only).
        let (own, inherited) = with_rfids(&self.rfids, |store| {
            if is_cs {
                (store.cs.clone(), Vec::new())
            } else {
                (store.scope_list(scope).to_vec(), store.cs.clone())
            }
        });
        let ServerOverlay::Detail(detail) = &mut self.overlay else {
            unreachable!()
        };
        detail.set_state_rows(fields);
        if !is_cs {
            detail.set_metering_rows(metering);
        }
        detail.set_rfids(own, inherited);
    }

    /// Resolve (and cache) a connection's charge-point identity.
    fn identity_of(&mut self, conn: ConnectionId) -> String {
        if let Some(id) = self.conn_identity.get(&conn) {
            return id.clone();
        }
        let id = self
            .backend
            .identity(conn)
            .unwrap_or_else(|| conn.to_string());
        self.conn_identity.insert(conn, id.clone());
        id
    }

    /// The scope of the connector entry (of `identity`) whose observed transactionId equals `tx`.
    fn connector_scope_for_tx(&self, identity: &str, tx: &str) -> Option<Scope> {
        self.entries
            .iter()
            .find(|e| {
                e.identity == identity
                    && e.scope.is_connector()
                    && e.get_field_str("TransactionId").as_deref() == Some(tx)
            })
            .map(|e| e.scope)
    }

    /// Find an entry by (identity, connector), creating it if missing. Returns its index.
    pub(super) fn entry_index(
        &mut self,
        identity: &str,
        scope: Scope,
        conn: Option<ConnectionId>,
    ) -> usize {
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.identity == identity && e.scope == scope)
        {
            return i;
        }
        // The Lua sim's shared state registration is keyed by identity + scope.
        let state = if scope.is_connector() {
            let arc = Arc::new(RwLock::new(V::Conn::default()));
            with_state_mut(&self.lua_states, |reg| {
                reg.stations
                    .entry(identity.to_string())
                    .or_default()
                    .conns
                    .push((scope, arc.clone()));
            });
            EntryState::Conn(arc)
        } else {
            let arc = Arc::new(RwLock::new(V::Cs::default()));
            with_state_mut(&self.lua_states, |reg| {
                reg.stations.entry(identity.to_string()).or_default().cs = Some(arc.clone());
            });
            EntryState::Cs(arc)
        };
        self.entries.push(Entry {
            identity: identity.to_string(),
            scope,
            conn,
            online: conn.is_some(),
            state,
            messages: Vec::new(),
        });
        self.entries.len() - 1
    }

    /// Drain backend events into entries (create/update state, append to per-entry logs).
    pub(super) fn drain_events(&mut self) {
        let mut events = Vec::new();
        while let Ok(ev) = self.events_rx.try_recv() {
            events.push(ev);
        }
        let had_events = !events.is_empty();
        for ev in events {
            match ev {
                ServerEvent::Connected { conn } => {
                    let identity = self.identity_of(conn);
                    // Ensure the CS-level entry exists and bring every entry of this CS online.
                    self.entry_index(&identity, Scope::CS, Some(conn));
                    for e in self.entries.iter_mut().filter(|e| e.identity == identity) {
                        e.online = true;
                        e.conn = Some(conn);
                    }
                }
                ServerEvent::Disconnected { conn } => {
                    for e in self.entries.iter_mut().filter(|e| e.conn == Some(conn)) {
                        e.online = false;
                        e.conn = None;
                    }
                }
                ServerEvent::Inbound {
                    conn,
                    name,
                    request,
                    response,
                } => {
                    let identity = self.identity_of(conn);
                    let mut scope = V::inbound_connector(&name, &request);
                    // Always make sure the CS-level entry exists for this connection.
                    self.entry_index(&identity, Scope::CS, Some(conn));
                    // A stop carrying no connector/EVSE id buckets to CS scope; re-route it to the
                    // connector holding the stopping transaction so its tx id (and limit) clear.
                    if scope == Scope::CS
                        && let Some(tx) = V::stop_tx_id(&name, &request)
                        && let Some(conn_scope) = self.connector_scope_for_tx(&identity, &tx)
                    {
                        scope = conn_scope;
                    }
                    let idx = self.entry_index(&identity, scope, Some(conn));
                    let entry = &mut self.entries[idx];
                    entry.online = true;
                    entry.conn = Some(conn);
                    entry.apply_inbound(&name, &request, &response);
                    for m in inbound_messages(&name, request, response) {
                        crate::module::ocpp::client::backend::push_capped(&mut entry.messages, m);
                    }
                }
                ServerEvent::Outbound {
                    conn,
                    scope,
                    name,
                    request,
                    response,
                    ok,
                    context,
                } => {
                    let identity = self.identity_of(conn);
                    // Persist a config-fetch response for this CS so it is available whether or not
                    // the detail overlay is open, and live-merge it into an open matching overlay.
                    if ok && name == V::config_action() {
                        let rows = V::parse_config(&response);
                        if !rows.is_empty() {
                            let store = self.cs_configs.entry(identity.clone()).or_default();
                            for (k, v, ro) in rows {
                                match store.iter_mut().find(|(ek, _, _)| *ek == k) {
                                    Some(r) => {
                                        r.1 = v;
                                        r.2 = ro;
                                    }
                                    None => store.push((k, v, ro)),
                                }
                            }
                            if let ServerOverlay::Detail(d) = &mut self.overlay
                                && d.is_cs
                                && d.identity == identity
                            {
                                d.set_config(self.cs_configs[&identity].clone());
                            }
                        }
                    }
                    let idx = self.entry_index(&identity, scope, Some(conn));
                    let entry = &mut self.entries[idx];
                    // Mirror state we successfully pushed to the station (e.g. an accepted
                    // SetChargingProfile's per-purpose limit).
                    if ok {
                        entry.apply_outbound(&name, &request, &response);
                    }
                    crate::module::ocpp::client::backend::push_capped(
                        &mut entry.messages,
                        crate::module::ocpp::client::backend::OcppMessage::new(
                            crate::module::ocpp::client::backend::Dir::Out,
                            name.clone(),
                            request,
                            None,
                            "outbound call",
                        ),
                    );
                    crate::module::ocpp::client::backend::push_capped(
                        &mut entry.messages,
                        crate::module::ocpp::client::backend::OcppMessage::new(
                            crate::module::ocpp::client::backend::Dir::In,
                            name,
                            response,
                            Some(ok),
                            context,
                        ),
                    );
                }
            }
        }
        // Keep the table sorted (CS → connector, grouped by station) without losing the selection.
        if had_events {
            self.sort_entries();
        }
    }

    /// Sort entries by `(identity, CS-before-connector, evse, connector)`, preserving the selected
    /// entry across the reorder.
    fn sort_entries(&mut self) {
        let selected_key = self
            .cs_table
            .state
            .table_state()
            .selected()
            .and_then(|i| self.entries.get(i))
            .map(|e| (e.identity.clone(), e.scope));
        self.entries.sort_by_key(entry_sort_key);
        if let Some(key) = selected_key
            && let Some(idx) = self
                .entries
                .iter()
                .position(|e| e.identity == key.0 && e.scope == key.1)
        {
            self.cs_table.state.select_index(idx);
        }
    }

    /// Spawn an outbound Call to `conn` and post its result back as an [`ServerEvent::Outbound`].
    pub(super) fn send_to(
        &self,
        conn: ConnectionId,
        scope: Scope,
        name: &str,
        payload: serde_json::Value,
    ) {
        let Some(sender) = self.backend.sender() else {
            return;
        };
        let tx = self.events_tx.clone();
        let name = name.to_string();
        let action = match V::decode_call(&name, payload.clone()) {
            Ok(a) => a,
            Err(e) => {
                let _ = tx.send(ServerEvent::Outbound {
                    conn,
                    scope,
                    name,
                    request: payload,
                    response: serde_json::Value::Null,
                    ok: false,
                    context: format!("invalid payload: {e}"),
                });
                return;
            }
        };
        let request = payload;
        tokio::spawn(async move {
            let (response, ok, context) = match sender.call(conn, action).await {
                Ok(resp) => (resp, true, String::new()),
                Err(e) => (serde_json::Value::Null, false, e.to_string()),
            };
            let _ = tx.send(ServerEvent::Outbound {
                conn,
                scope,
                name,
                request,
                response,
                ok,
                context,
            });
        });
    }

    /// Selected entry index, if any.
    pub(super) fn selected(&self) -> Option<usize> {
        let i = self.cs_table.state.table_state().selected()?;
        (i < self.entries.len()).then_some(i)
    }

    /// Drain the single Lua action queue and route each action to its station/scope. Each item is
    /// `(identity, scope, action, overrides)`: the payload is derived from the matching entry's
    /// observed state (falling back to the action default), then overrides are merged.
    fn drain_lua_actions(&mut self) {
        let queued: Vec<(String, Scope, String, serde_json::Value)> =
            self.runtime.lua_queue.lock().drain(..).collect();
        let mut sends: Vec<(ConnectionId, Scope, String, serde_json::Value)> = Vec::new();
        for (identity, scope, name, overrides) in queued {
            let Some(conn) = self.conn_for(&identity) else {
                continue;
            };
            let mut payload = self
                .entries
                .iter()
                .find(|e| e.identity == identity && e.scope == scope)
                .and_then(|e| e.derive_payload(&name))
                .unwrap_or_else(|| default_action_payload::<V>(&name));
            // Default any connector/EVSE id from the entry's scope before user overrides win.
            V::inject_scope(&mut payload, scope);
            merge_overrides(&mut payload, overrides);
            sends.push((conn, scope, name, payload));
        }
        for (conn, scope, name, payload) in sends {
            self.send_to(conn, scope, &name, payload);
        }
    }

    /// Rebuild the action list for the selected entry's level, if it changed.
    pub(super) fn sync_actions(&mut self) {
        let is_connector = self
            .selected()
            .map(|i| self.entries[i].scope.is_connector());
        let want = is_connector.unwrap_or(false);
        // Rebuild only when the level (CS vs connector) actually changes. Gating on a live
        // selection rebuilt the list every frame while the table was empty, resetting the
        // selection so it could never move.
        if self.actions_for_connector == Some(want) {
            return;
        }
        self.actions_for_connector = Some(want);
        let values: Vec<String> = V::csms_actions()
            .iter()
            .filter(|(_, scope)| {
                matches!(
                    (want, scope),
                    (true, ConnectorScope::Required | ConnectorScope::Optional)
                        | (false, ConnectorScope::None | ConnectorScope::Optional)
                )
            })
            .map(|(n, _)| n.to_string())
            .collect();
        self.actions = super::render::action_list(values);
    }

    /// Load the selected message's payload into the read-only viewer.
    pub(super) fn sync_code(&mut self) {
        let content = self
            .selected()
            .and_then(|i| {
                let sel = self.msg_table.state.table_state().selected()?;
                self.entries[i].messages.get(sel).cloned()
            })
            .map(|m| serde_json::to_string_pretty(&m.payload).unwrap_or_default())
            .unwrap_or_default();
        // Only reset the viewer when the selected payload actually changes; otherwise the periodic
        // refresh would snap its scroll position back to the top every tick.
        if content != self.code_content {
            self.code.state.set_content(&content);
            self.code_content = content;
        }
    }

    fn save_device_to(&self, path: &str) -> CommandResult {
        use ferrowl_util::convert::{Converter, FileType};
        let Some(ty) = FileType::from_path(path) else {
            return CommandResult::Handled(Some((
                Level::Warning,
                format!("unknown format for '{path}' (use .toml or .json)"),
            )));
        };
        let mut device = OcppDeviceConfig::from_spec(&self.spec, self.device.scripts.clone());
        device.version = Some(crate::config::VERSION.to_string());
        device.log_file.clone_from(&self.device.log_file);
        with_rfids(&self.rfids, |store| fill_device_rfids(&mut device, store));
        match Converter::save(&device, path, ty) {
            Ok(()) => CommandResult::Handled(Some((
                Level::Info,
                format!("Saved device config to {path}"),
            ))),
            Err(e) => CommandResult::Handled(Some((Level::Error, format!("Save failed: {e:?}")))),
        }
    }

    pub(super) fn refresh_impl<'a>(&'a mut self) -> RefreshFuture<'a> {
        Box::pin(async move {
            // UI-R-314/UI-R-315 — a deferred stop-bearing lifecycle command only signalled
            // `request_stop()`; drain its outcome (and run any follow-up) once the task actually
            // ends. Row/state housekeeping stays here, not the command handler, so a CSMS that is
            // still unbinding does not lose its rows before the stop lands.
            if self.pending_lifecycle.is_some()
                && let Some(stop_result) = self.backend.poll_stop().await
            {
                self.entries.clear();
                self.conn_identity.clear();
                self.cs_configs.clear();
                self.clear_lua_states();
                match self.pending_lifecycle.take() {
                    Some(PendingLifecycle::Stop) => {
                        let (level, msg) = match stop_result {
                            Ok(()) => (Level::Info, "CSMS server stopped".to_string()),
                            // OC-R-102 — a stop failure logs at Error.
                            Err(e) => (Level::Error, format!("CSMS server stop failed: {e}")),
                        };
                        self.log.write().await.write(level, &msg);
                    }
                    Some(PendingLifecycle::Restart) => {
                        if let Err(e) = stop_result {
                            self.log
                                .write()
                                .await
                                .write(Level::Error, &format!("Restart: stop failed: {e}"));
                        }
                        self.want_running = true;
                        let handler = V::handler(self.events_tx.clone(), self.rfids.clone());
                        let (level, msg) = match self.backend.start(&self.spec, handler).await {
                            Ok(binding) => (Level::Info, started_message(binding, "restarted")),
                            Err(e) => (Level::Error, format!("listen failed: {e}")),
                        };
                        self.log.write().await.write(level, &msg);
                    }
                    None => unreachable!("outer condition checked pending_lifecycle.is_some()"),
                }
            }

            // Apply a resolved `:edit`.
            if let Some((spec, path, extra_headers)) = self.deferred.setup.take() {
                let mut device = OcppDeviceConfig::from_spec(&spec, self.device.scripts.clone());
                device.log_file = self.device.log_file.clone();
                with_rfids(&self.rfids, |store| fill_device_rfids(&mut device, store));
                // Not carried forward from `self.device`: the setup dialog's headers table is
                // authoritative for a confirmed edit (server role ignores it, but the value is
                // still threaded uniformly — see `HeaderDef`'s doc comment).
                device.extra_headers = extra_headers;
                if spec.role == OcppRole::Client {
                    // Stop the listener first: dropping `Server<V>` only detaches its accept task,
                    // leaving the port bound, so the swapped-in view could never rebind.
                    if let Err(e) = self.backend.stop().await {
                        self.log.write().await.write(
                            Level::Error,
                            &format!("Stop before role switch failed: {e}"),
                        );
                    }
                    self.deferred.replacement = Some(build_client_view(spec, path, device));
                    return;
                }
                if spec.version != self.spec.version {
                    // A version change must swap the whole view: `ServerView<V>`/`OcppServer<V>` are
                    // generic over the *old* version and would rebind with the old subprotocol,
                    // rejecting the (now-different-version) client handshake with a 400.
                    if let Err(e) = self.backend.stop().await {
                        self.log.write().await.write(
                            Level::Error,
                            &format!("Stop before version switch failed: {e}"),
                        );
                    }
                    self.deferred.replacement = Some(build_server_view(spec, path, device));
                    return;
                }
                // Rebind on the (possibly changed) endpoint: the backend builds its listener
                // config from the spec passed into `start`, so updating `self.spec` is all an
                // edit needs.
                if let Err(e) = self.backend.stop().await {
                    self.log.write().await.write(
                        Level::Error,
                        &format!("Stop for settings update failed: {e}"),
                    );
                }
                self.spec = spec;
                self.device = device;
                self.device_path = path;
                self.entries.clear();
                self.conn_identity.clear();
                self.cs_configs.clear();
                self.clear_lua_states();
                self.log
                    .write()
                    .await
                    .write(Level::Info, "Settings updated");
            }

            // Auto-bind / honour `:start`.
            if self.want_running && !self.backend.is_online() {
                let handler = V::handler(self.events_tx.clone(), self.rfids.clone());
                if let Err(e) = self.backend.start(&self.spec, handler).await {
                    self.log
                        .write()
                        .await
                        .write(Level::Error, &format!("listen failed: {e}"));
                    self.want_running = false;
                }
            }

            self.drain_events();
            self.drain_lua_actions();

            // Apply a pending `:log` change (or device-config log file) to the persistent sink.
            if self.runtime.applied_log_file != self.device.log_file {
                let name = self.spec.name.clone();
                self.log
                    .write()
                    .await
                    .set_log_file(self.device.log_file.as_deref(), &name);
                self.runtime
                    .applied_log_file
                    .clone_from(&self.device.log_file);
            }

            // Tee new protocol messages (across all entries) into the persistent log. Each entry's
            // log is filtered separately on screen, but the persistent log is the whole CSMS.
            let mut max_seq = self.runtime.logged_seq;
            let mut new: Vec<(u64, String)> = Vec::new();
            for entry in &self.entries {
                for m in entry
                    .messages
                    .iter()
                    .filter(|m| m.seq > self.runtime.logged_seq)
                {
                    max_seq = max_seq.max(m.seq);
                    new.push((m.seq, m.log_line()));
                }
            }
            if !new.is_empty() {
                new.sort_by_key(|(seq, _)| *seq);
                let mut log = self.log.write().await;
                for (_, line) in new {
                    log.write(Level::Info, &line);
                }
                self.runtime.logged_seq = max_seq;
            }

            if let super::ServerOverlay::Scripts(dialog) = &mut self.overlay {
                let entries =
                    crate::dialog::scripts::snapshot_log(&self.script_log, crate::app::LOG_SIZE)
                        .await;
                dialog.set_log_entries(entries);
            }
        })
    }

    pub(super) fn handle_command_impl<'a>(&'a mut self, cmd: &'a str) -> CommandFuture<'a> {
        Box::pin(async move {
            let Some(parsed) = parse_command(&OCPP_SERVER_COMMAND_SPECS, cmd) else {
                return CommandResult::Unhandled;
            };
            match parsed {
                OcppServerCmd::Start => {
                    self.want_running = true;
                    let handler = V::handler(self.events_tx.clone(), self.rfids.clone());
                    match self.backend.start(&self.spec, handler).await {
                        Ok(binding) => CommandResult::Handled(Some((
                            Level::Info,
                            started_message(binding, "started"),
                        ))),
                        Err(e) => CommandResult::Handled(Some((
                            Level::Error,
                            format!("listen failed: {e}"),
                        ))),
                    }
                }
                OcppServerCmd::Stop => {
                    // UI-R-314/UI-R-315 — signal and return; `refresh_impl` drains `poll_stop()`
                    // and logs the outcome. A stop already pending overwrites the follow-up in
                    // place instead of re-requesting one against a backend already `Stopping`.
                    self.want_running = false;
                    if self.pending_lifecycle.is_some() {
                        self.pending_lifecycle = Some(PendingLifecycle::Stop);
                        return CommandResult::Handled(None);
                    }
                    match self.backend.request_stop().await {
                        Ok(()) => {
                            self.pending_lifecycle = Some(PendingLifecycle::Stop);
                            CommandResult::Handled(None)
                        }
                        Err(e) => {
                            self.log
                                .write()
                                .await
                                .write(Level::Error, &format!("CSMS server stop failed: {e}"));
                            CommandResult::Handled(None)
                        }
                    }
                }
                OcppServerCmd::Restart => {
                    // See `OcppServerCmd::Stop` above.
                    if self.pending_lifecycle.is_some() {
                        self.pending_lifecycle = Some(PendingLifecycle::Restart);
                        return CommandResult::Handled(None);
                    }
                    match self.backend.request_stop().await {
                        Ok(()) => {
                            self.pending_lifecycle = Some(PendingLifecycle::Restart);
                            CommandResult::Handled(None)
                        }
                        Err(ferrowl_ocpp::Error::NotRunning) => {
                            self.entries.clear();
                            self.conn_identity.clear();
                            self.cs_configs.clear();
                            self.clear_lua_states();
                            self.want_running = true;
                            let handler = V::handler(self.events_tx.clone(), self.rfids.clone());
                            match self.backend.start(&self.spec, handler).await {
                                Ok(binding) => CommandResult::Handled(Some((
                                    Level::Info,
                                    started_message(binding, "restarted"),
                                ))),
                                Err(e) => CommandResult::Handled(Some((
                                    Level::Error,
                                    format!("listen failed: {e}"),
                                ))),
                            }
                        }
                        Err(e) => CommandResult::Handled(Some((
                            Level::Error,
                            format!("Restart failed: {e}"),
                        ))),
                    }
                }
                OcppServerCmd::Edit => {
                    self.overlay = ServerOverlay::Setup(Box::new(
                        crate::module::ocpp::setup_dialog::OcppSetupDialog::edit(
                            &self.spec,
                            &self.device_path,
                            &self.device.extra_headers,
                        ),
                    ));
                    CommandResult::Handled(None)
                }
                OcppServerCmd::WriteDevice(None) => {
                    if self.device_path.is_empty() {
                        CommandResult::Handled(Some((
                            Level::Warning,
                            "No configuration file path configured.".into(),
                        )))
                    } else {
                        self.save_device_to(&self.device_path.clone())
                    }
                }
                OcppServerCmd::WriteDevice(Some(path)) => self.save_device_to(&path),
                OcppServerCmd::Compact => {
                    self.set_compact(!self.compact);
                    CommandResult::Handled(None)
                }
                OcppServerCmd::Log(None) => {
                    self.device.log_file = None;
                    CommandResult::Handled(Some((Level::Info, "File logging disabled".into())))
                }
                OcppServerCmd::Log(Some(path)) => {
                    self.device.log_file = Some(path.clone());
                    CommandResult::Handled(Some((Level::Info, format!("Logging to {path}"))))
                }
                OcppServerCmd::Rfid(sub) => self.handle_rfid(sub.as_deref()),
            }
        })
    }

    /// The `:rfid` subcommands: bare lists the accept-list, `clear` empties it, `add <tag>` /
    /// `del <tag>` mutate it. Anything else stays unhandled (an unknown command).
    fn handle_rfid(&mut self, sub: Option<&str>) -> CommandResult {
        match sub {
            None => {
                let msg = with_rfids(&self.rfids, |s| {
                    if s.cs.is_empty() {
                        "CS RFID accept-list empty (all tags accepted)".to_string()
                    } else {
                        format!("Accepted CS RFIDs: {}", s.cs.join(", "))
                    }
                });
                CommandResult::Handled(Some((Level::Info, msg)))
            }
            Some("clear") => {
                with_rfids_mut(&self.rfids, |s| s.cs.clear());
                CommandResult::Handled(Some((
                    Level::Info,
                    "CS RFID accept-list cleared (all accepted)".into(),
                )))
            }
            Some(rest) if rest.starts_with("add ") => {
                let tag = rest["add ".len()..].trim().to_string();
                if tag.is_empty() {
                    return CommandResult::Handled(Some((
                        Level::Warning,
                        "Usage: :rfid add <tag>".into(),
                    )));
                }
                if with_rfids_mut(&self.rfids, |s| s.add(Scope::CS, tag.clone())) {
                    CommandResult::Handled(Some((Level::Info, format!("Added CS RFID {tag}"))))
                } else {
                    CommandResult::Handled(Some((
                        Level::Warning,
                        format!("{tag} already in accept-list"),
                    )))
                }
            }
            Some(rest) if rest.starts_with("del ") => {
                let tag = rest["del ".len()..].trim().to_string();
                if with_rfids_mut(&self.rfids, |s| s.remove(Scope::CS, &tag)) {
                    CommandResult::Handled(Some((Level::Info, format!("Removed CS RFID {tag}"))))
                } else {
                    CommandResult::Handled(Some((
                        Level::Warning,
                        format!("{tag} not in accept-list"),
                    )))
                }
            }
            Some(_) => CommandResult::Unhandled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::ocpp::config::session::{OcppProtocol, OcppRole, OcppSpec, OcppVersion};
    use crate::module::view::ModuleView;
    use ferrowl_ocpp::V1_6;

    fn server_view(port: u16) -> ServerView<V1_6> {
        let spec = OcppSpec {
            name: "csms".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Server,
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: Default::default(),
        };
        ServerView::<V1_6>::new(spec, String::new(), OcppDeviceConfig::default())
    }

    /// Poll until the CSMS listener has bound (`start` retries the bind in the background).
    async fn wait_bound(v: &ServerView<V1_6>) {
        for _ in 0..100 {
            if v.backend.bound_addr().is_some() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("CSMS listener never bound");
    }

    /// UI-R-314 — `:stop` against a CSMS whose listener task is genuinely alive signals
    /// termination and returns without waiting for the accept loop (and every connection) to end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_stop_command_returns_without_waiting() {
        let port = ferrowl_test_support::reserve_tcp_port().release();

        let mut v = server_view(port);
        v.handle_command("start").await;
        wait_bound(&v).await;

        let before = std::time::Instant::now();
        let result = v.handle_command("stop").await;
        assert!(
            before.elapsed() < std::time::Duration::from_millis(50),
            "handle_command(\"stop\") took {:?}, expected to return immediately",
            before.elapsed()
        );
        assert!(matches!(result, CommandResult::Handled(None)));
        assert!(v.lifecycle_pending());

        for _ in 0..200 {
            if !v.lifecycle_pending() {
                break;
            }
            v.refresh().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!v.lifecycle_pending());
    }

    /// UI-R-315 — once `refresh()` settles a deferred stop, the outcome lands in the log as an
    /// `Info "CSMS server stopped"` line rather than riding the command's own immediate result;
    /// observed-station rows survive until the stop lands.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_refresh_logs_stop_outcome() {
        let port = ferrowl_test_support::reserve_tcp_port().release();

        let mut v = server_view(port);
        v.handle_command("start").await;
        wait_bound(&v).await;
        v.entry_index("cp-1", Scope::CS, None);

        let result = v.handle_command("stop").await;
        assert!(matches!(result, CommandResult::Handled(None)));
        assert_eq!(v.entries.len(), 1, "row must survive until the stop lands");

        for _ in 0..200 {
            if !v.lifecycle_pending() {
                break;
            }
            v.refresh().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!v.lifecycle_pending());
        assert!(
            v.entries.is_empty(),
            "row must be cleared once the stop lands"
        );

        let lines = v
            .log
            .read()
            .await
            .peek_n(crate::app::LOG_SIZE)
            .into_iter()
            .map(|(_, level, l)| (level, l))
            .collect::<Vec<_>>();
        assert!(
            lines
                .iter()
                .any(|(level, l)| *level == Level::Info && l == "CSMS server stopped"),
            "missing 'CSMS server stopped' Info line: {lines:?}"
        );
    }

    /// UI-R-314/UI-R-315 — a `:restart` issued while a `:stop` is still pending overwrites the
    /// follow-up in place rather than re-requesting `request_stop()` against a backend already
    /// `Stopping`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_restart_while_stop_pending_overwrites_the_follow_up() {
        let port = ferrowl_test_support::reserve_tcp_port().release();

        let mut v = server_view(port);
        v.handle_command("start").await;
        wait_bound(&v).await;

        assert!(matches!(
            v.handle_command("stop").await,
            CommandResult::Handled(None)
        ));
        assert!(v.lifecycle_pending());

        assert!(matches!(
            v.handle_command("restart").await,
            CommandResult::Handled(None)
        ));
        assert!(
            v.lifecycle_pending(),
            "the follow-up must still be pending, not dropped"
        );

        for _ in 0..200 {
            if !v.lifecycle_pending() {
                break;
            }
            v.refresh().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            !v.lifecycle_pending(),
            "a stop overwritten with a restart must still settle, not latch forever"
        );
        wait_bound(&v).await;
    }

    #[test]
    fn ut_default_action_payload_unknown_name_falls_back_to_empty_object() {
        assert_eq!(
            default_action_payload::<V1_6>("NotARealAction"),
            serde_json::json!({})
        );
    }

    #[test]
    fn ut_default_action_payload_known_name_encodes_default_action() {
        // "Reset" is a real CSMS-originated V1_6 action; its Default-derived skeleton must encode
        // to a non-null JSON object, exercising the success path through `V::encode_action` rather
        // than the `unwrap_or_else` fallback.
        let payload = default_action_payload::<V1_6>("Reset");
        assert!(payload.is_object());
    }
}
