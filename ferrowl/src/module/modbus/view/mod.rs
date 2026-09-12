use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_codec::{Address, Value};
use ferrowl_modbus::{Key, SlaveKey};
use ferrowl_store::{Memory, Range};
use ferrowl_ui::EventResult;
use ferrowl_ui::traits::{HandleEvents, OverlayRoute, SetFocus};
use ferrowl_ui_derive::Overlay;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::Level;
use crate::config::script::ScriptDef;
use crate::config::{DeviceConfig, ModuleSpec};
use crate::dialog::close_confirm::CloseConfirmEvent;
use crate::dialog::lua_help::ScriptContext;
use crate::dialog::scripts::ScriptDialog;
use crate::module::modbus::dialog::{EditInputDialog, EditSelectionDialog};
use crate::module::modbus::setup_dialog::SetupDialog;
use crate::module::modbus::table::{Definition, TableView, cmp_definitions};
use crate::module::view::{
    CommandDescriptor, CommandFuture, CommandResult, CommandSpec, ModuleView, RefreshFuture,
    SharedLog, parse_command,
};

use super::ModbusModule;

mod mutate;
mod overlay;
use overlay::{ModbusOverlay, PendingAction};

/// The single modal overlay over the module view (mutually exclusive by construction). The
/// derive supplies `is_active`/`close`/`take`/`route_keys`; only the setup dialog carries a
/// common-key tag (`focus_cycle`) — its `Esc`/close-confirm handling lives inside the dialog
/// itself, offered to it before `route_keys` runs (see `handle_events`). The register overlay
/// (`ModbusOverlay`, itself a nested Edit/EditSelection/Add dispatch with its own close-confirm
/// and sub-dialog precedence) and the scripts overlay route every key through their own bespoke
/// handling instead, so they carry no tags.
#[derive(Overlay)]
enum ModbusViewOverlay {
    #[overlay(none)]
    None,
    /// Register edit/add overlay (routes every key through `handle_overlay_key`).
    Register(Box<ModbusOverlay>),
    /// Module re-setup dialog.
    #[overlay(focus_cycle)]
    Setup(Box<SetupDialog>),
    /// Lua scripts editor (routes every key through its own `handle_events`).
    Scripts(Box<ScriptDialog>),
}

ferrowl_ui::impl_overlay_keys!(SetupDialog);

pub struct ModbusModuleView {
    module: ModbusModule,
    spec: ModuleSpec,
    device: DeviceConfig,
    table: TableView,
    sort: Option<(usize, bool)>,
    overlay: ModbusViewOverlay,
    pending: Option<PendingAction>,
    /// UI-R-314/UI-R-315 — a stop-bearing lifecycle command (`:stop`/`:restart`/`:reload`) that
    /// has signalled `request_stop()` and is waiting for `refresh()` to observe `poll_stop()`
    /// complete before logging its outcome (and, for `Restart`/`Reload`, running the follow-up).
    pending_lifecycle: Option<PendingLifecycle>,
    /// Whether this view (its content pane) currently has keyboard focus, set by the owning `Tab`.
    view_focused: bool,
    /// MB-R-150 — the session-wide serial-path registry attached via `set_serial_paths`, kept so
    /// a `:reload`-rebuilt `self.module` can be reattached to the same registry instead of
    /// silently falling back to a private default.
    serial_paths: super::SerialPathRegistry,
}

/// UI-R-314/UI-R-315 — the follow-up state a deferred stop-bearing lifecycle command needs once
/// its `poll_stop()` completes. `Reload` carries the config already loaded synchronously at
/// dispatch time (`handle_command`), so `refresh()` never re-reads the file.
enum PendingLifecycle {
    Stop,
    Restart,
    Reload {
        path: String,
        device: Box<DeviceConfig>,
    },
}

impl ModbusModuleView {
    pub fn new(module: ModbusModule, spec: ModuleSpec, device: DeviceConfig) -> Self {
        let definitions = module
            .registers()
            .iter()
            .map(|(name, description, register, values)| {
                Definition::new(
                    name.clone(),
                    description.clone(),
                    register.clone(),
                    values.clone(),
                )
            })
            .collect();
        Self {
            table: TableView::new(definitions),
            module,
            spec,
            device,
            sort: None,
            overlay: ModbusViewOverlay::None,
            pending: None,
            pending_lifecycle: None,
            view_focused: false,
            serial_paths: super::SerialPathRegistry::default(),
        }
    }

    fn open_edit(&mut self) {
        let Some(def) = self.table.selected().cloned() else {
            return;
        };
        let current_default = self
            .device
            .definitions
            .get(&def.name)
            .and_then(|d| d.default.as_ref());
        let unscaled = def.value.clone().unscaled().to_string();
        if def.named_values.is_empty() {
            self.overlay = ModbusViewOverlay::Register(Box::new(ModbusOverlay::Edit(
                EditInputDialog::from_register(
                    &def.name,
                    &def.description,
                    &def.register,
                    &unscaled,
                    current_default,
                    self.spec.role.client_or_server() == crate::config::ClientOrServer::Server,
                ),
            )));
        } else {
            self.overlay = ModbusViewOverlay::Register(Box::new(ModbusOverlay::EditSelection(
                EditSelectionDialog::from_register(
                    &def.name,
                    &def.description,
                    &def.register,
                    def.named_values.clone(),
                    &unscaled,
                    &def.raw_value,
                    current_default,
                    self.spec.role.client_or_server() == crate::config::ClientOrServer::Server,
                ),
            )));
        }
    }

    /// The register-edit/add overlay as a shared reference, if that's the currently active
    /// overlay variant.
    fn register_overlay(&self) -> Option<&ModbusOverlay> {
        match &self.overlay {
            ModbusViewOverlay::Register(o) => Some(o.as_ref()),
            _ => None,
        }
    }

    /// The register-edit/add overlay as a mutable reference, if that's the currently active
    /// overlay variant.
    fn register_overlay_mut(&mut self) -> Option<&mut ModbusOverlay> {
        match &mut self.overlay {
            ModbusViewOverlay::Register(o) => Some(o.as_mut()),
            _ => None,
        }
    }

    fn handle_overlay_key(&mut self, modifiers: KeyModifiers, code: KeyCode) {
        let overlay = match self.register_overlay() {
            Some(o) => o,
            None => return,
        };

        if overlay.has_confirm_delete() {
            match code {
                KeyCode::Esc => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .close_confirm_delete(),
                KeyCode::Tab => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .confirm_delete_focus_next(),
                KeyCode::BackTab => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .confirm_delete_focus_previous(),
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if self
                        .register_overlay()
                        .expect(
                            "a Register overlay was confirmed present at the top of this function",
                        )
                        .confirm_delete_is_confirmed()
                    {
                        let name = self.table.selected().map(|d| d.name.clone());
                        self.overlay.close();
                        if let Some(name) = name {
                            self.pending = Some(PendingAction::Delete(name));
                        }
                    } else {
                        self.register_overlay_mut().expect("a Register overlay was confirmed present at the top of this function").close_confirm_delete();
                    }
                }
                _ => {}
            }
            return;
        }

        if overlay.has_sub_dialog() {
            match code {
                KeyCode::Esc => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .close_add_dialog(),
                KeyCode::Enter => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .confirm_add_dialog(),
                KeyCode::Tab => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .add_dialog_focus_next(),
                KeyCode::BackTab => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .add_dialog_focus_previous(),
                _ => self
                    .register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .add_dialog_handle_events(modifiers, code),
            }
            return;
        }

        // The close-confirm popup takes precedence once open.
        if self
            .register_overlay()
            .expect("a Register overlay was confirmed present at the top of this function")
            .close_confirm_is_active()
        {
            match self
                .register_overlay_mut()
                .expect("a Register overlay was confirmed present at the top of this function")
                .close_confirm_handle_key(modifiers, code)
            {
                CloseConfirmEvent::Close => self.overlay.close(),
                CloseConfirmEvent::Dismiss | CloseConfirmEvent::Consumed => {}
            }
            return;
        }

        self.register_overlay_mut()
            .expect("a Register overlay was confirmed present at the top of this function")
            .clear_name_error();

        let confirm_button_focused = self
            .register_overlay()
            .expect("a Register overlay was confirmed present at the top of this function")
            .is_confirm_button_focused();
        let delete_button_focused = self
            .register_overlay()
            .expect("a Register overlay was confirmed present at the top of this function")
            .is_delete_register_button_focused();

        match (modifiers, code) {
            (KeyModifiers::NONE, KeyCode::Esc) => {
                self.register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .close_confirm_open();
            }
            (KeyModifiers::NONE, KeyCode::Enter) => {
                if delete_button_focused {
                    self.register_overlay_mut()
                        .expect(
                            "a Register overlay was confirmed present at the top of this function",
                        )
                        .open_confirm_delete();
                } else {
                    self.confirm_overlay();
                }
            }
            (KeyModifiers::NONE, KeyCode::Char(' ')) => {
                if confirm_button_focused {
                    self.confirm_overlay();
                } else {
                    self.register_overlay_mut()
                        .expect(
                            "a Register overlay was confirmed present at the top of this function",
                        )
                        .handle_space();
                }
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::BackTab) => {
                self.register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .focus_previous();
            }
            (KeyModifiers::NONE, KeyCode::Tab) => {
                self.register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .focus_next();
            }
            (KeyModifiers::NONE, KeyCode::Char('z')) => {
                self.table.set_compact(!self.table.compact);
            }
            _ => {
                self.register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .handle_events(modifiers, code);
            }
        }

        let new_overlay = self.register_overlay().and_then(|o| {
            o.maybe_switch_to_selection()
                .or_else(|| o.maybe_switch_to_input())
        });
        if let Some(o) = new_overlay {
            self.overlay = ModbusViewOverlay::Register(Box::new(o));
        }
    }

    fn confirm_overlay(&mut self) {
        let Some(overlay) = self.register_overlay() else {
            return;
        };
        let is_add = overlay.is_add();
        if let Some(edited) = overlay.apply() {
            let current_name = self.table.selected().map(|d| d.name.clone());
            if !is_add {
                if let Some(original) = &current_name
                    && &edited.name != original
                    && self.device.definitions.contains_key(&edited.name)
                {
                    let msg = format!("Name '{}' already in use", edited.name);
                    self.register_overlay_mut()
                        .expect(
                            "a Register overlay was confirmed present at the top of this function",
                        )
                        .set_name_error(msg);
                    return;
                }
            } else if self.device.definitions.contains_key(&edited.name) {
                let msg = format!("Name '{}' already in use", edited.name);
                self.register_overlay_mut()
                    .expect("a Register overlay was confirmed present at the top of this function")
                    .set_name_error(msg);
                return;
            }
            self.overlay.close();
            if is_add {
                self.pending = Some(PendingAction::Add(edited));
            } else {
                let Some(idx) = self.table.selected_index() else {
                    return;
                };
                let original_name = current_name.unwrap_or_default();
                self.pending = Some(PendingAction::Edit {
                    edited,
                    idx,
                    original_name,
                });
            }
        }
    }
}

impl ferrowl_ui::traits::SetFocus for ModbusModuleView {
    fn set_focused(&mut self, focus: bool) {
        self.view_focused = focus;
    }
}

impl ferrowl_ui::traits::IsFocus for ModbusModuleView {
    fn is_focused(&self) -> bool {
        self.view_focused
    }
}

impl ModuleView for ModbusModuleView {
    fn name(&self) -> String {
        self.spec.name.clone()
    }

    fn is_overlay_active(&self) -> bool {
        self.overlay.is_active()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        use ratatui::layout::{Constraint, Layout};

        let [content_area, status_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

        self.table
            .table
            .state
            .set_focused(self.view_focused && !self.overlay.is_active());
        self.table.render(content_area, frame.buffer_mut());

        // MB-R-137/153 — tri-state CONNECTED/RECONNECTING/DISCONNECTED status line.
        let status = self.module.connection_status();
        let addr = self.module.bound_addr().map(|a| a.to_string());
        crate::view::status_bar::render_status_bar(
            status,
            addr.as_deref(),
            status_area,
            frame.buffer_mut(),
        );
    }

    fn render_overlay(&mut self, frame: &mut Frame, _area: Rect) {
        let full_area = frame.area();
        match &mut self.overlay {
            ModbusViewOverlay::Scripts(scripts) => scripts.render(full_area, frame.buffer_mut()),
            ModbusViewOverlay::Setup(setup) => setup.render(full_area, frame.buffer_mut()),
            ModbusViewOverlay::Register(overlay) => overlay.render(full_area, frame.buffer_mut()),
            ModbusViewOverlay::None => {}
        }
    }

    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        if !self.overlay.is_active() {
            return if modifiers == KeyModifiers::NONE && code == KeyCode::Enter {
                self.open_edit();
                EventResult::Consumed
            } else {
                self.table.handle_events(modifiers, code)
            };
        }

        // Setup dialog: offer the key to the dialog first, so its embedded close-confirm popup
        // can consume Esc/Enter/Tab/BackTab while it is open. Only run the default Enter handling
        // below (via the `route_keys`/per-variant match) when the dialog leaves it unhandled.
        if let ModbusViewOverlay::Setup(setup) = &mut self.overlay
            && let EventResult::Consumed = setup.handle_events(modifiers, code)
        {
            if setup.take_close_request() {
                self.overlay.close();
            }
            return EventResult::Consumed;
        }

        // Common keys: `Tab`/`BackTab` cycle focus on the setup dialog (`focus_cycle`). The
        // register/scripts overlays have no common tags — their close/focus handling is bespoke
        // (register: close-confirm precedes Esc; scripts: every key routes through its own
        // handler) — so `route_keys` always returns `Unhandled` for them.
        match self.overlay.route_keys(modifiers, code) {
            OverlayRoute::Closed | OverlayRoute::Cycled => return EventResult::Consumed,
            OverlayRoute::Unhandled => {}
        }

        match &mut self.overlay {
            ModbusViewOverlay::Setup(setup) => {
                if let (KeyModifiers::NONE, KeyCode::Enter) = (modifiers, code)
                    && let Ok(resolved) = setup.resolve()
                {
                    self.pending = Some(PendingAction::ApplySetup(Box::new(resolved.values)));
                    self.overlay.close();
                }
            }
            ModbusViewOverlay::Scripts(dialog) => {
                if dialog.handle_events(modifiers, code) {
                    let ModbusViewOverlay::Scripts(dialog) = self.overlay.take() else {
                        unreachable!("just matched Scripts above")
                    };
                    let (scripts, interval) = dialog.resolve();
                    self.device.scripts = scripts;
                    self.device.script_interval = interval.as_secs_f64();
                    self.module.set_script_interval(interval);
                    self.module
                        .reload_scripts(super::registers::collect_scripts(&self.device));
                } else if let Some(script) = dialog.take_run_request() {
                    // Pulled out of the dialog borrow before touching `self.module` (UI-R-051).
                    self.module.run_script_once(script.name, script.code);
                }
            }
            ModbusViewOverlay::Register(_) => self.handle_overlay_key(modifiers, code),
            ModbusViewOverlay::None => {}
        }
        EventResult::Consumed
    }

    fn refresh<'a>(&'a mut self) -> RefreshFuture<'a> {
        Box::pin(async move {
            // UI-R-314/UI-R-315 — a deferred stop-bearing lifecycle command only signalled
            // `request_stop()`; drain its outcome (and run any follow-up) once the task actually
            // ends. A sibling of `self.pending` below, not folded into it: `PendingAction` is
            // dialog-driven and always resolves within one tick, this may span several.
            if self.pending_lifecycle.is_some()
                && let Some(stop_result) = self.module.poll_stop().await
            {
                let role = self.spec.role.to_string();
                let endpoint = self.spec.endpoint.to_string();
                match self.pending_lifecycle.take() {
                    Some(PendingLifecycle::Stop) => {
                        let (level, msg) = match stop_result {
                            Ok(()) => (Level::Info, format!("Stopped {role}")),
                            Err(e) => (Level::Error, format!("Stop {role} failed: {e}")),
                        };
                        self.log().write().await.write(level, &msg);
                    }
                    Some(PendingLifecycle::Restart) => {
                        let stop_err = stop_result.err().filter(|e| !e.is_not_running());
                        let (level, msg) = match self.module.start().await {
                            Ok(()) => match stop_err {
                                None => (Level::Info, format!("Restarted {role} on {endpoint}")),
                                Some(e) => (
                                    Level::Error,
                                    format!(
                                        "Restarted {role} on {endpoint}, but stop of previous instance failed: {e}"
                                    ),
                                ),
                            },
                            Err(e) => (Level::Error, format!("Restart {role} failed: {e}")),
                        };
                        self.log().write().await.write(level, &msg);
                    }
                    Some(PendingLifecycle::Reload { path, device }) => {
                        let stop_err = stop_result.err().filter(|e| !e.is_not_running());
                        let new_module = ModbusModule::new(&self.spec, &device);
                        self.module = new_module;
                        self.device = *device;
                        // MB-R-150 — reattach the session-wide registry (`ModbusModule::new`
                        // defaults to a private one), so an in-progress conflict survives
                        // `:reload` instead of silently clearing.
                        self.module.set_serial_paths(self.serial_paths.clone());
                        let defs: Vec<_> = self
                            .module
                            .registers()
                            .iter()
                            .map(|(n, d, r, v)| {
                                Definition::new(n.clone(), d.clone(), r.clone(), v.clone())
                            })
                            .collect();
                        self.table.set_definitions(defs);
                        self.sort = None;
                        let (level, msg) = if let Err(e) = self.module.start().await {
                            (Level::Error, format!(":reload start error: {e}"))
                        } else {
                            match stop_err {
                                None => (Level::Info, format!(":reload done — '{path}'")),
                                Some(e) => (
                                    Level::Error,
                                    format!(
                                        ":reload done — '{path}', but stop of previous instance failed: {e}"
                                    ),
                                ),
                            }
                        };
                        self.log().write().await.write(level, &msg);
                    }
                    None => unreachable!("outer condition checked pending_lifecycle.is_some()"),
                }
            }

            if let Some(pending) = self.pending.take() {
                match pending {
                    PendingAction::Add(edited) => self.apply_add(edited).await,
                    PendingAction::Edit {
                        edited,
                        idx,
                        original_name,
                    } => self.apply_edit(edited, idx, original_name).await,
                    PendingAction::Delete(name) => self.delete_register_by_name(name).await,
                    PendingAction::ApplySetup(values) => self.apply_setup(*values).await,
                }
            }

            // Acquire the (async) virtual-store guard first so the (sync) memory guard below is
            // never held across an `.await`. Scoped so both guards are dropped before the
            // `.await` further down (script-log snapshot).
            {
                let vs_arc = self.module.virtual_store();
                let virtual_values = vs_arc.read().await;
                let memory_arc = self.module.memory();
                let memory = memory_arc.read();

                let mut updated: Vec<Definition> = self
                    .table
                    .definitions()
                    .iter()
                    .cloned()
                    .map(|d| decode_definition(d, &memory, &virtual_values))
                    .collect();

                if let Some((column, descending)) = self.sort {
                    updated.sort_by(|a, b| cmp_definitions(a, b, column, descending));
                }

                self.table.set_definitions(updated);
            }

            if let ModbusViewOverlay::Scripts(dialog) = &mut self.overlay {
                let entries = crate::dialog::scripts::snapshot_log(
                    &self.module.script_log(),
                    crate::app::LOG_SIZE,
                )
                .await;
                dialog.set_log_entries(entries);
            }
        })
    }

    fn handle_command<'a>(&'a mut self, cmd: &'a str) -> CommandFuture<'a> {
        let Some(parsed) = parse_command(&MODBUS_COMMAND_SPECS, cmd) else {
            return Box::pin(std::future::ready(CommandResult::Unhandled));
        };

        match parsed {
            ModbusCmd::Start => Box::pin(async move {
                let role = self.spec.role.to_string();
                let endpoint = self.spec.endpoint.to_string();
                match self.module.start().await {
                    Ok(()) => CommandResult::Handled(Some((
                        Level::Info,
                        format!("Started {role} on {endpoint}"),
                    ))),
                    Err(e) => CommandResult::Handled(Some((
                        Level::Error,
                        format!("Start {role} failed: {e}"),
                    ))),
                }
            }),

            ModbusCmd::Stop => Box::pin(async move {
                let role = self.spec.role.to_string();
                // A stop-bearing command is already in flight (its own request_stop already
                // signalled the instance): overwrite the follow-up rather than re-requesting a
                // stop `Instance::request_stop` would reject as `NotRunning` (it's already
                // `Stopping`, not `Idle`) — that rejection must never be mistaken for "nothing to
                // stop" and drop the earlier command's outcome (UI-R-315: never discarded).
                if self.pending_lifecycle.is_some() {
                    self.pending_lifecycle = Some(PendingLifecycle::Stop);
                    return CommandResult::Handled(None);
                }
                match self.module.request_stop().await {
                    Ok(()) => {
                        self.pending_lifecycle = Some(PendingLifecycle::Stop);
                        CommandResult::Handled(None)
                    }
                    // Nothing was running (Idle): no deferred outcome to carry, and nothing for
                    // `refresh()` to ever observe (`poll_stop()` only resolves from `Stopping`) —
                    // logged, never returned as `:stop`'s own immediate result (UI-R-315).
                    Err(e) => {
                        self.log()
                            .write()
                            .await
                            .write(Level::Error, &format!("Stop {role} failed: {e}"));
                        CommandResult::Handled(None)
                    }
                }
            }),

            ModbusCmd::Restart => Box::pin(async move {
                // See `ModbusCmd::Stop` above: a stop already in flight is overwritten with the
                // new follow-up rather than re-requested.
                if self.pending_lifecycle.is_some() {
                    self.pending_lifecycle = Some(PendingLifecycle::Restart);
                    return CommandResult::Handled(None);
                }
                let role = self.spec.role.to_string();
                let endpoint = self.spec.endpoint.to_string();
                match self.module.request_stop().await {
                    Ok(()) => {
                        self.pending_lifecycle = Some(PendingLifecycle::Restart);
                        CommandResult::Handled(None)
                    }
                    // Nothing was running (Idle, MB-R-098: benign, not reported): run the
                    // follow-up start immediately — there is no in-flight task `poll_stop()`
                    // could ever resolve.
                    Err(_) => match self.module.start().await {
                        Ok(()) => CommandResult::Handled(Some((
                            Level::Info,
                            format!("Restarted {role} on {endpoint}"),
                        ))),
                        Err(e) => CommandResult::Handled(Some((
                            Level::Error,
                            format!("Restart {role} failed: {e}"),
                        ))),
                    },
                }
            }),

            ModbusCmd::Reload => Box::pin(async move {
                if self.spec.device.is_empty() {
                    return CommandResult::Handled(Some((
                        Level::Warning,
                        "No configuration file path configured. Reload aborted.".into(),
                    )));
                }
                let path = self.spec.device.clone();
                let device = match crate::config::load_device(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        return CommandResult::Handled(Some((
                            Level::Error,
                            format!(":reload failed to load '{path}': {e}"),
                        )));
                    }
                };
                // See `ModbusCmd::Stop` above: a stop already in flight is overwritten with the
                // new follow-up rather than re-requested.
                if self.pending_lifecycle.is_some() {
                    self.pending_lifecycle = Some(PendingLifecycle::Reload {
                        path,
                        device: Box::new(device),
                    });
                    return CommandResult::Handled(None);
                }
                match self.module.request_stop().await {
                    Ok(()) => {
                        self.pending_lifecycle = Some(PendingLifecycle::Reload {
                            path,
                            device: Box::new(device),
                        });
                        CommandResult::Handled(None)
                    }
                    // Nothing was running (Idle): rebuild and start immediately — there is no
                    // in-flight task `poll_stop()` could ever resolve.
                    Err(_) => {
                        let new_module = ModbusModule::new(&self.spec, &device);
                        self.module = new_module;
                        self.device = device;
                        // MB-R-150 — the fresh module's `serial_paths` defaults to a private
                        // registry (`ModbusModule::new`); reattach the session-wide one so an
                        // in-progress conflict survives `:reload` instead of silently clearing.
                        self.module.set_serial_paths(self.serial_paths.clone());
                        let defs: Vec<_> = self
                            .module
                            .registers()
                            .iter()
                            .map(|(n, d, r, v)| {
                                Definition::new(n.clone(), d.clone(), r.clone(), v.clone())
                            })
                            .collect();
                        self.table.set_definitions(defs);
                        self.sort = None;
                        if let Err(e) = self.module.start().await {
                            return CommandResult::Handled(Some((
                                Level::Error,
                                format!(":reload start error: {e}"),
                            )));
                        }
                        CommandResult::Handled(Some((
                            Level::Info,
                            format!(":reload done — '{path}'"),
                        )))
                    }
                }
            }),

            ModbusCmd::Edit => {
                let timing = ModbusModule::resolve_timing(&self.device);
                let dialog = SetupDialog::edit(
                    &self.spec.name,
                    &self.spec.device,
                    self.spec.role.client_or_server(),
                    &self.spec.endpoint,
                    timing,
                    &self.device.read_ranges,
                    Some(&self.device.tls),
                );
                self.overlay = ModbusViewOverlay::Setup(Box::new(dialog));
                Box::pin(std::future::ready(CommandResult::Handled(None)))
            }

            ModbusCmd::Add => {
                self.overlay = ModbusViewOverlay::Register(Box::new(ModbusOverlay::Add(
                    EditInputDialog::new(),
                )));
                Box::pin(std::future::ready(CommandResult::Handled(None)))
            }

            ModbusCmd::Script => {
                self.overlay = ModbusViewOverlay::Scripts(Box::new(ScriptDialog::new(
                    &self.device.scripts,
                    self.device.script_interval_duration(),
                    ScriptContext::Modbus,
                )));
                Box::pin(std::future::ready(CommandResult::Handled(None)))
            }

            ModbusCmd::Compact => {
                self.table.set_compact(!self.table.compact);
                Box::pin(std::future::ready(CommandResult::Handled(None)))
            }

            ModbusCmd::WriteDevice(None) => {
                if self.spec.device.is_empty() {
                    return Box::pin(std::future::ready(CommandResult::Handled(Some((
                        Level::Warning,
                        "No configuration file path configured.".into(),
                    )))));
                }
                let path = self.spec.device.clone();
                let result = self.save_device_to(&path);
                Box::pin(std::future::ready(result))
            }

            ModbusCmd::WriteDevice(Some(path)) => {
                let result = self.save_device_to(&path);
                Box::pin(std::future::ready(result))
            }

            // Bare `:log` is not a Modbus command (the spec gives Modbus only `:log <file>`);
            // it stays unknown, matching the pre-table behavior.
            ModbusCmd::Log(None) => Box::pin(std::future::ready(CommandResult::Unhandled)),

            ModbusCmd::Log(Some(file)) => Box::pin(async move {
                self.device.log_file = Some(file.clone());
                match self.module.set_log_base(Some(&file)) {
                    Ok(()) => CommandResult::Handled(Some((
                        Level::Info,
                        format!("Logging to files based on {file} (':wd' to persist)"),
                    ))),
                    Err(e) => CommandResult::Handled(Some((
                        Level::Error,
                        format!("Failed to open log file {file}: {e}"),
                    ))),
                }
            }),

            ModbusCmd::Set(rest) => Box::pin(async move {
                let (register, value) = parse_set_args(rest.as_deref().unwrap_or(""));
                if register.is_empty() || value.is_empty() {
                    return CommandResult::Handled(Some((
                        Level::Warning,
                        ":set requires <register> <value>".into(),
                    )));
                }
                self.set_register_value(&register, &value).await
            }),

            ModbusCmd::Order(rest) => {
                let parts: Vec<&str> = rest.as_deref().unwrap_or("").split_whitespace().collect();
                let sync_result = match parts.as_slice() {
                    [] => {
                        let original = self
                            .module
                            .registers()
                            .iter()
                            .map(|(n, d, r, v)| {
                                Definition::new(n.clone(), d.clone(), r.clone(), v.clone())
                            })
                            .collect();
                        self.sort = None;
                        self.table.set_definitions(original);
                        CommandResult::Handled(Some((Level::Info, "Order cleared".to_string())))
                    }
                    [col] | [col, "asc"] => self.apply_order(col, false),
                    [col, "desc"] => self.apply_order(col, true),
                    _ => CommandResult::Unhandled,
                };
                Box::pin(std::future::ready(sync_result))
            }
        }
    }

    fn commands(&self) -> &[CommandDescriptor] {
        static DESCRIPTORS: std::sync::OnceLock<Vec<CommandDescriptor>> =
            std::sync::OnceLock::new();
        DESCRIPTORS.get_or_init(|| MODBUS_COMMAND_SPECS.iter().map(|s| s.descriptor).collect())
    }

    fn keybinds(&self) -> &[CommandDescriptor] {
        &MODBUS_KEYBINDS
    }

    fn log(&self) -> SharedLog {
        self.module.log()
    }

    fn lifecycle_pending(&self) -> bool {
        self.pending_lifecycle.is_some()
    }

    fn session_spec(&self) -> Option<serde_json::Value> {
        let mut v = serde_json::to_value(&self.spec).ok()?;
        v.as_object_mut()?.insert("type".into(), "modbus".into());
        Some(v)
    }

    fn scripts(&self) -> Option<&[ScriptDef]> {
        Some(&self.device.scripts)
    }

    fn set_scripts(&mut self, scripts: Vec<ScriptDef>) -> bool {
        self.device.scripts = scripts;
        self.module
            .reload_scripts(super::registers::collect_scripts(&self.device));
        true
    }

    fn set_serial_paths(&mut self, registry: super::SerialPathRegistry) {
        self.module.set_serial_paths(registry.clone());
        self.serial_paths = registry;
    }

    fn module_host(&self) -> Option<std::sync::Arc<dyn ferrowl_lua::module::ModuleHost>> {
        let registers: HashMap<String, ferrowl_codec::Register> = self
            .module
            .registers()
            .iter()
            .map(|(name, _, register, _)| (name.clone(), register.clone()))
            .collect();
        let role = match self.spec.role.client_or_server() {
            crate::config::ClientOrServer::Client => "client",
            crate::config::ClientOrServer::Server => "server",
        };
        Some(std::sync::Arc::new(crate::registry::ModbusHost {
            memory: self.module.memory(),
            virtual_store: self.module.virtual_store(),
            registers: std::sync::Arc::new(registers),
            role,
        }))
    }
}

static MODBUS_KEYBINDS: [CommandDescriptor; 5] = [
    CommandDescriptor {
        name: "Enter",
        description: "edit selected register",
    },
    CommandDescriptor {
        name: "Enter (dialog)",
        description: "confirm edit",
    },
    CommandDescriptor {
        name: "Space (dialog)",
        description: "press button / toggle",
    },
    CommandDescriptor {
        name: "z (dialog)",
        description: "toggle compact table",
    },
    CommandDescriptor {
        name: "Esc (dialog)",
        description: "close dialog",
    },
];

/// The parsed form of every command this view accepts; produced by [`parse_command`] over
/// [`MODBUS_COMMAND_SPECS`]. The exhaustive `match` in `handle_command` is what guarantees a
/// table entry cannot exist without a handler.
enum ModbusCmd {
    Start,
    Stop,
    Restart,
    Reload,
    Edit,
    Add,
    Script,
    Compact,
    WriteDevice(Option<String>),
    Log(Option<String>),
    Set(Option<String>),
    Order(Option<String>),
}

/// Single source for this view's commands: aliases, help row, and parse target per entry.
static MODBUS_COMMAND_SPECS: [CommandSpec<ModbusCmd>; 12] = [
    CommandSpec {
        aliases: &["e", "edit"],
        descriptor: CommandDescriptor {
            name: ":e | :edit",
            description: "edit module setup",
        },
        build: |_| ModbusCmd::Edit,
    },
    CommandSpec {
        aliases: &["a", "add"],
        descriptor: CommandDescriptor {
            name: ":a | :add",
            description: "add register to device",
        },
        build: |_| ModbusCmd::Add,
    },
    CommandSpec {
        aliases: &["start"],
        descriptor: CommandDescriptor {
            name: ":start",
            description: "start module",
        },
        build: |_| ModbusCmd::Start,
    },
    CommandSpec {
        aliases: &["stop"],
        descriptor: CommandDescriptor {
            name: ":stop",
            description: "stop module",
        },
        build: |_| ModbusCmd::Stop,
    },
    CommandSpec {
        aliases: &["restart"],
        descriptor: CommandDescriptor {
            name: ":restart",
            description: "restart module",
        },
        build: |_| ModbusCmd::Restart,
    },
    CommandSpec {
        aliases: &["reload"],
        descriptor: CommandDescriptor {
            name: ":reload",
            description: "reload device config",
        },
        build: |_| ModbusCmd::Reload,
    },
    CommandSpec {
        aliases: &["compact"],
        descriptor: CommandDescriptor {
            name: ":compact",
            description: "toggle compact mode",
        },
        build: |_| ModbusCmd::Compact,
    },
    CommandSpec {
        aliases: &["set"],
        descriptor: CommandDescriptor {
            name: ":set <reg> <val>",
            description: "write register value",
        },
        build: |rest| ModbusCmd::Set(rest.map(str::to_string)),
    },
    CommandSpec {
        aliases: &["wd", "write-device"],
        descriptor: CommandDescriptor {
            name: ":wd | :write-device [path]",
            description: "save device config",
        },
        build: |rest| ModbusCmd::WriteDevice(rest.map(str::to_string)),
    },
    CommandSpec {
        aliases: &["log"],
        descriptor: CommandDescriptor {
            name: ":log <file>",
            description: "set log file",
        },
        build: |rest| ModbusCmd::Log(rest.map(str::to_string)),
    },
    CommandSpec {
        aliases: &["script"],
        descriptor: CommandDescriptor {
            name: ":script",
            description: "manage lua scripts",
        },
        build: |_| ModbusCmd::Script,
    },
    CommandSpec {
        aliases: &["order"],
        descriptor: CommandDescriptor {
            name: ":order [col] [asc|desc]",
            description: "sort table by column",
        },
        build: |rest| ModbusCmd::Order(rest.map(str::to_string)),
    },
];

fn decode_definition(
    mut d: Definition,
    memory: &Memory<Key<SlaveKey>>,
    virtual_values: &HashMap<String, Value>,
) -> Definition {
    let prev_raw = std::mem::take(&mut d.raw_value);
    match d.register.address() {
        Address::Fixed(addr) => {
            let width = d.register.format().width();
            let key = Key {
                id: SlaveKey {
                    slave_id: *d.register.slave_id(),
                    kind: d.register.kind().clone(),
                },
            };
            let raw = memory
                .read_unchecked(key, &Range::new(*addr as usize, width))
                .unwrap_or_else(|| vec![0; width]);
            d.value = match d.register.decode(&raw) {
                Ok(v) => v,
                Err(_) => Value::Ascii("Error".to_string()),
            };
            d.raw_value = raw_hex(&raw);
        }
        Address::Virtual => match virtual_values.get(&d.name) {
            Some(v) => {
                d.value = v.clone();
                d.raw_value = d
                    .register
                    .encode(&v.clone().unscaled().to_string())
                    .map(|raw| raw_hex(&raw))
                    .unwrap_or_default();
            }
            None => {
                d.value = Value::Ascii(String::new());
                d.raw_value.clear();
            }
        },
    }
    // The first fill-in (empty previous raw) is not a change; later differing
    // decodes stamp the highlight window (see `Definition::cell_styles`).
    if !prev_raw.is_empty() && d.raw_value != prev_raw {
        d.changed_at = Some(std::time::Instant::now());
    }
    d
}

fn raw_hex(raw: &[u16]) -> String {
    let mut out = String::with_capacity(raw.len() * 5 + 2);
    out.push('[');
    for (i, v) in raw.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out += &format!("{v:04x}");
    }
    out.push(']');
    out
}

fn parse_set_args(rest: &str) -> (String, String) {
    if let Some(after) = rest.strip_prefix('"') {
        match after.split_once('"') {
            Some((reg, val)) => (reg.to_string(), val.trim_start().to_string()),
            None => (after.to_string(), String::new()),
        }
    } else {
        match rest.split_once(char::is_whitespace) {
            Some((reg, val)) => (reg.to_string(), val.trim_start().to_string()),
            None => (rest.to_string(), String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ModbusModuleView, ModbusViewOverlay, PendingAction, decode_definition, parse_set_args,
        raw_hex,
    };
    use crate::app::Level;
    use crate::config::script::ScriptDef;
    use crate::config::{DeviceConfig, Endpoint, ModuleSpec, Role};
    use crate::module::modbus::setup_dialog::SetupValues;
    use crate::module::modbus::table::Definition;
    use crate::module::view::{CommandResult, ModuleView};
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_codec::format::{BitField, Endian, Format, Resolution, WordOrder};
    use ferrowl_codec::{Access, Address, Kind, NumericPrimitive, RegisterBuilder, Value};
    use ferrowl_modbus::UnitId;
    use ferrowl_modbus::{Key, SlaveKey};
    use ferrowl_store::{CellKind, Memory, Range};
    use ferrowl_test_support::{reserve_tcp_port, reserve_temp_dir};
    use ferrowl_ui::EventResult;
    use ratatui::Frame;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::collections::HashMap;

    fn empty_device() -> DeviceConfig {
        DeviceConfig {
            version: None,
            timeout_ms: None,
            delay_ms: None,
            interval_ms: None,
            reconnect: None,
            tls: Default::default(),
            log_file: None,
            read_ranges: Default::default(),
            definitions: Default::default(),
            script_interval: 1.0,
            scripts: Default::default(),
        }
    }

    fn tcp_server_spec() -> ModuleSpec {
        ModuleSpec {
            name: "test module".into(),
            device: String::new(),
            role: Role::Server,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 5020,
            },
        }
    }

    fn new_view() -> ModbusModuleView {
        let device = empty_device();
        let spec = tcp_server_spec();
        let module = super::super::ModbusModule::new(&spec, &device);
        ModbusModuleView::new(module, spec, device)
    }

    // --- decode_definition -------------------------------------------------

    fn fixed_def() -> Definition {
        let register = RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Fixed(0))
            .format(Format::u16(
                Endian::Big,
                WordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap();
        Definition::new("hold".to_string(), "d".to_string(), register, vec![])
    }

    fn virtual_def() -> Definition {
        let register = RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Virtual)
            .format(Format::u16(
                Endian::Big,
                WordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap();
        Definition::new("virt".to_string(), "d".to_string(), register, vec![])
    }

    #[test]
    /// UI-R-046 — a fixed register's live value decodes from its memory word for display.
    fn ut_decode_definition_fixed_reads_memory_word() {
        let def = fixed_def();
        let mut memory = Memory::<Key<SlaveKey>>::default();
        let key = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind: Kind::HoldingRegister,
            },
        };
        memory.add_ranges(
            key.clone(),
            &CellKind::read_write(ferrowl_store::CellType::Register),
            std::slice::from_ref(&Range::new(0, 1)),
        );
        memory.write_unchecked(key, &Range::new(0, 1), &[42u16]);
        let empty_vs: HashMap<String, Value> = HashMap::new();
        let decoded = decode_definition(def, &memory, &empty_vs);
        assert!(matches!(
            decoded.value,
            Value::Numeric(NumericPrimitive::U16(42), _)
        ));
        assert_eq!(decoded.raw_value, "[002a]");
    }

    #[test]
    /// UI-R-046 — a fixed register with no memory decodes to a zero display value.
    fn ut_decode_definition_fixed_missing_memory_defaults_to_zero() {
        let def = fixed_def();
        let memory = Memory::<Key<SlaveKey>>::default();
        let empty_vs: HashMap<String, Value> = HashMap::new();
        let decoded = decode_definition(def, &memory, &empty_vs);
        assert!(matches!(
            decoded.value,
            Value::Numeric(NumericPrimitive::U16(0), _)
        ));
    }

    #[test]
    /// UI-R-046 — a virtual register's live value comes from the virtual store.
    fn ut_decode_definition_virtual_uses_store_value() {
        let def = virtual_def();
        let memory = Memory::<Key<SlaveKey>>::default();
        let mut vs: HashMap<String, Value> = HashMap::new();
        vs.insert("virt".into(), Value::u16(9, Resolution(1.0)));
        let decoded = decode_definition(def, &memory, &vs);
        assert!(matches!(
            decoded.value,
            Value::Numeric(NumericPrimitive::U16(9), _)
        ));
        assert_eq!(decoded.raw_value, "[0009]");
    }

    #[test]
    /// UI-R-046 — a virtual register with no value renders blank.
    fn ut_decode_definition_virtual_missing_value_is_blank() {
        let def = virtual_def();
        let memory = Memory::<Key<SlaveKey>>::default();
        let empty_vs: HashMap<String, Value> = HashMap::new();
        let decoded = decode_definition(def, &memory, &empty_vs);
        assert!(matches!(decoded.value, Value::Ascii(ref s) if s.is_empty()));
        assert!(decoded.raw_value.is_empty());
    }

    #[test]
    /// UI-R-185 — the first value fill is not counted as a change (no highlight).
    fn ut_decode_definition_first_fill_is_not_a_change() {
        let def = fixed_def();
        let memory = Memory::<Key<SlaveKey>>::default();
        let empty_vs: HashMap<String, Value> = HashMap::new();
        let decoded = decode_definition(def, &memory, &empty_vs);
        assert!(decoded.changed_at.is_none());
        // A second identical decode is not a change either.
        let decoded = decode_definition(decoded, &memory, &empty_vs);
        assert!(decoded.changed_at.is_none());
    }

    #[test]
    /// UI-R-185 — a changed value stamps its change time to drive the highlight window.
    fn ut_decode_definition_value_change_stamps_changed_at() {
        let mut memory = Memory::<Key<SlaveKey>>::default();
        let key = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind: Kind::HoldingRegister,
            },
        };
        memory.add_ranges(
            key.clone(),
            &CellKind::read_write(ferrowl_store::CellType::Register),
            std::slice::from_ref(&Range::new(0, 1)),
        );
        memory.write_unchecked(key.clone(), &Range::new(0, 1), &[1u16]);
        let empty_vs: HashMap<String, Value> = HashMap::new();
        let decoded = decode_definition(fixed_def(), &memory, &empty_vs);
        assert!(decoded.changed_at.is_none(), "first fill must not stamp");
        memory.write_unchecked(key, &Range::new(0, 1), &[2u16]);
        let decoded = decode_definition(decoded, &memory, &empty_vs);
        assert!(decoded.changed_at.is_some(), "changed value must stamp");
    }

    // --- view construction & key handling -----------------------------------

    #[test]
    /// UI-R-021 — a new view starts with no overlay open.
    fn ut_new_view_starts_with_no_overlay() {
        let view = new_view();
        assert!(!view.is_overlay_active());
    }

    #[test]
    /// UI-R-021 — Enter on an empty table opens no overlay.
    fn ut_enter_on_empty_table_does_not_open_overlay() {
        // No registers selected -> open_edit returns early without an overlay.
        let mut view = new_view();
        let result = view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(matches!(result, EventResult::Consumed));
        assert!(!view.is_overlay_active());
    }

    #[test]
    /// UI-R-018, UI-R-094 — the `:scripts` command is handled by the view, opening the script overlay; a script created in the dialog reaches the device when the dialog is applied.
    fn ut_scripts_command_opens_overlay_and_close_applies() {
        let mut view = new_view();
        drop(view.handle_command("script"));
        assert!(view.is_overlay_active());
        // Create a script through the dialog: Tab past the interval field to the table, then to
        // the name input (the code editor is skipped while nothing is selected), type a name,
        // Enter creates it, Esc + Enter (confirm-close) closes.
        view.handle_events(KeyModifiers::NONE, KeyCode::Tab);
        view.handle_events(KeyModifiers::NONE, KeyCode::Tab);
        for c in "sim".chars() {
            view.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(!view.is_overlay_active());
        assert_eq!(view.device.scripts.len(), 1);
        assert_eq!(view.device.scripts[0].name, "sim");
        assert!(view.device.scripts[0].enabled);
    }

    #[tokio::test]
    /// UI-R-088 — an on-demand script run (`e` on the script table) leaves the scripts dialog
    /// open (unlike the `Enter` apply-and-close path) and its output — print, C_Log, and the
    /// error that ends the run — reaches the dialog's log pane.
    async fn ut_on_demand_run_keeps_the_dialog_open_and_shows_its_output() {
        let mut device = empty_device();
        // The markers are built by runtime concatenation, not written contiguously in the
        // source, so their appearance in the rendered dialog can only come from the log pane
        // (which shows executed output) and never from the code editor pane (which shows the
        // source verbatim).
        device.scripts = vec![ScriptDef {
            name: "s1".into(),
            code: r#"print("ui" .. "-r-088-out") C_Log:Info("ui" .. "-r-088-log") error("ui" .. "-r-088-boom")"#
                .into(),
            enabled: false,
        }];
        let spec = tcp_server_spec();
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);

        drop(view.handle_command("script"));
        view.handle_events(KeyModifiers::NONE, KeyCode::Tab); // interval -> table
        view.handle_events(KeyModifiers::NONE, KeyCode::Char('e'));

        assert!(
            matches!(view.overlay, ModbusViewOverlay::Scripts(_)),
            "the run does not close the dialog"
        );
        assert!(view.is_overlay_active());

        let area = Rect::new(0, 0, 220, 48);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(220, 48)).unwrap();
        let mut text = String::new();
        for _ in 0..100 {
            view.refresh().await;
            term.draw(|f: &mut Frame| view.render_overlay(f, area))
                .unwrap();
            text = buffer_text(term.backend().buffer());
            if text.contains("ui-r-088-out")
                && text.contains("ui-r-088-log")
                && text.contains("ui-r-088-boom")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            text.contains("ui-r-088-out")
                && text.contains("ui-r-088-log")
                && text.contains("ui-r-088-boom"),
            "expected all three markers in the rendered dialog:\n{text}"
        );
    }

    #[test]
    /// UI-R-018 — the `:edit` command is handled by the view, opening the setup overlay.
    fn ut_edit_command_opens_setup_overlay() {
        let mut view = new_view();
        drop(view.handle_command("edit"));
        assert!(view.is_overlay_active());
    }

    #[test]
    /// UI-R-022 — Tab cycles focus through the setup overlay fields.
    fn ut_setup_overlay_tab_cycles_focus_via_derive() {
        // `Setup` is tagged `focus_cycle`, so Tab advances the dialog's own focus once the
        // dialog itself leaves the key unhandled (no close-confirm open).
        use ferrowl_ui::traits::IsFocus;
        let mut view = new_view();
        drop(view.handle_command("edit"));
        let ModbusViewOverlay::Setup(setup) = &view.overlay else {
            panic!("expected Setup overlay");
        };
        assert!(setup.name.state.is_focused());
        assert!(!setup.config_path.state.is_focused());
        view.handle_events(KeyModifiers::NONE, KeyCode::Tab);
        let ModbusViewOverlay::Setup(setup) = &view.overlay else {
            panic!("expected Setup overlay");
        };
        assert!(!setup.name.state.is_focused());
        assert!(setup.config_path.state.is_focused());
    }

    #[test]
    /// UI-R-022 — Shift+Tab cycles the setup overlay focus in reverse.
    fn ut_setup_overlay_backtab_cycles_focus_reverse() {
        use ferrowl_ui::traits::IsFocus;
        let mut view = new_view();
        drop(view.handle_command("edit"));
        view.handle_events(KeyModifiers::NONE, KeyCode::Tab);
        view.handle_events(KeyModifiers::NONE, KeyCode::BackTab);
        let ModbusViewOverlay::Setup(setup) = &view.overlay else {
            panic!("expected Setup overlay");
        };
        // BackTab after one Tab lands back on the first field.
        assert!(setup.name.state.is_focused());
        assert!(!setup.config_path.state.is_focused());
    }

    #[test]
    /// UI-R-021 — an open register overlay consumes table-navigation keys.
    fn ut_register_overlay_swallows_table_navigation_key() {
        // While the register overlay is open, `Down` is consumed by the overlay's own dispatch
        // (untagged -> bespoke `handle_overlay_key`) and must not fall through to the underlying
        // table's selection movement.
        let mut view = view_for(device_with_defs());
        view.table.select_first();
        let before = view.table.selected().map(|d| d.name.clone());
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(view.is_overlay_active());
        view.handle_events(KeyModifiers::NONE, KeyCode::Down);
        let after = view.table.selected().map(|d| d.name.clone());
        assert_eq!(
            before, after,
            "table selection must not move while overlay is open"
        );
    }

    #[test]
    /// UI-R-022, UI-R-080 — Enter confirms the edit dialog after field routing.
    fn ut_enter_still_confirms_edit_dialog_after_offer_first_routing() {
        // The setup dialog is offered every key before the default Esc/Enter/Tab/BackTab
        // handling runs; Enter must still reach it, confirm the dialog and apply the edit.
        let mut view = new_view();
        drop(view.handle_command("edit"));
        assert!(view.is_overlay_active());
        let result = view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(matches!(result, EventResult::Consumed));
        assert!(!view.is_overlay_active());
        assert!(matches!(view.pending, Some(PendingAction::ApplySetup(_))));
    }

    #[test]
    /// UI-R-023 — Esc opens the close-confirm rather than closing the setup overlay.
    fn ut_esc_does_not_close_setup_overlay() {
        let mut view = new_view();
        drop(view.handle_command("edit"));
        assert!(view.is_overlay_active());
        // First Esc opens the close-confirm popup instead of closing the overlay outright.
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(view.is_overlay_active());
        // A second Esc dismisses the confirm popup, leaving the overlay open.
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(view.is_overlay_active());
    }

    #[test]
    /// UI-R-023 — Esc-then-Enter closes the setup overlay.
    fn ut_esc_then_enter_closes_setup_overlay() {
        let mut view = new_view();
        drop(view.handle_command("edit"));
        assert!(view.is_overlay_active());
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(view.is_overlay_active());
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(!view.is_overlay_active());
    }

    #[test]
    /// UI-R-018 — the `:add` command is handled by the view, opening the register-add overlay.
    fn ut_add_command_opens_add_overlay() {
        let mut view = new_view();
        drop(view.handle_command("add"));
        assert!(view.is_overlay_active());
        // First Esc opens the close-confirm popup; overlay stays open.
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(view.is_overlay_active());
        // Enter confirms the close-confirm popup, closing the overlay.
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(!view.is_overlay_active());
    }

    #[test]
    /// UI-R-018 — the `:compact` command toggles the table's compact flag.
    fn ut_compact_command_toggles_table_compact_flag() {
        let mut view = new_view();
        assert!(!view.table.compact);
        drop(view.handle_command("compact"));
        assert!(view.table.compact);
    }

    /// All buffer cell symbols joined into one string, for containment assertions.
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

    /// A device config with one plain fixed register ("hold") and one with named values
    /// ("named"), to drive both edit-overlay flavours.
    fn device_with_defs() -> DeviceConfig {
        use crate::config::device::{
            AccessCfg, AlignmentCfg, EndianCfg, NamedValue, RegisterDef, Scalar,
            ValueType as CfgValueType, WordOrderCfg,
        };

        let base = |address: u16, values: Vec<NamedValue>| RegisterDef {
            slave_id: 1,
            kind: Kind::HoldingRegister,
            address: Some(address),
            is_virtual: false,
            access: AccessCfg::ReadWrite,
            value_type: CfgValueType::U16,
            endian: EndianCfg::Big,
            word_order: WordOrderCfg::default(),
            resolution: 1.0,
            bitmask: None,
            length: 1,
            alignment: AlignmentCfg::Left,
            values,
            update: None,
            description: "desc".into(),
            default: None,
        };

        let mut device = empty_device();
        device.definitions.insert("hold".into(), base(0, vec![]));
        device.definitions.insert(
            "named".into(),
            base(
                1,
                vec![NamedValue {
                    name: "on".into(),
                    value: Scalar::Int(1),
                }],
            ),
        );
        device
    }

    fn view_for(device: DeviceConfig) -> ModbusModuleView {
        let spec = tcp_server_spec();
        let module = super::super::ModbusModule::new(&spec, &device);
        ModbusModuleView::new(module, spec, device)
    }

    #[test]
    /// UI-R-023 — Esc opens the close-confirm rather than closing the register overlay.
    fn ut_esc_does_not_close_register_overlay() {
        let mut view = view_for(device_with_defs());
        view.table.select_first();
        // Definitions are BTreeMap-ordered: "hold" (no named values) comes first.
        assert_eq!(view.table.selected().map(|d| d.name.as_str()), Some("hold"));
        let result = view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(matches!(result, EventResult::Consumed));
        assert!(view.is_overlay_active());
        // First Esc opens the close-confirm popup; overlay stays open.
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(view.is_overlay_active());
        assert!(view.register_overlay().unwrap().close_confirm_is_active());
        // A second Esc dismisses the confirm popup, leaving the overlay open.
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(view.is_overlay_active());
        assert!(!view.register_overlay().unwrap().close_confirm_is_active());
    }

    #[test]
    /// UI-R-023 — Esc-then-Enter closes the register overlay.
    fn ut_esc_then_enter_closes_register_overlay() {
        let mut view = view_for(device_with_defs());
        view.table.select_first();
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(view.is_overlay_active());
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(view.is_overlay_active());
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(!view.is_overlay_active());
    }

    #[test]
    /// UI-R-014 — `:` types into the value input rather than entering command mode.
    fn ut_colon_in_value_input_types() {
        let mut view = view_for(device_with_defs());
        view.table.select_first();
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(view.is_overlay_active());
        // Focus starts on the free-text Value field; `:` must be typed as ordinary text.
        view.handle_events(KeyModifiers::NONE, KeyCode::Char(':'));
        assert!(!view.register_overlay().unwrap().close_confirm_is_active());
        assert!(view.is_overlay_active());
    }

    #[test]
    /// UI-R-023 — Esc in the delete-confirm still cancels.
    fn ut_confirm_delete_esc_still_cancels() {
        let mut view = view_for(device_with_defs());
        view.table.select_first();
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(view.is_overlay_active());
        // Value -> DefaultValue -> AddButton -> ConfirmButton -> DeleteRegisterButton.
        for _ in 0..4 {
            view.handle_events(KeyModifiers::NONE, KeyCode::Tab);
        }
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter); // open confirm-delete
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc); // cancels the sub-dialog only
        assert!(view.is_overlay_active());
        assert!(view.pending.is_none());
    }

    #[test]
    /// UI-R-023 — Esc in the named-value sub-dialog still cancels.
    fn ut_named_value_subdialog_esc_still_cancels() {
        let mut view = view_for(device_with_defs());
        view.table.select_first();
        view.handle_events(KeyModifiers::NONE, KeyCode::Down); // "named"
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter); // open selection overlay
        assert!(view.is_overlay_active());
        view.handle_events(KeyModifiers::NONE, KeyCode::Tab); // Value -> AddButton
        view.handle_events(KeyModifiers::NONE, KeyCode::Char(' ')); // open add-named-value sub-dialog
        view.handle_events(KeyModifiers::NONE, KeyCode::Esc); // cancels the sub-dialog only
        assert!(view.is_overlay_active());
    }

    #[test]
    /// UI-R-021 — Enter on a named-value register opens the selection overlay.
    fn ut_enter_on_named_value_register_opens_selection_overlay() {
        let mut view = view_for(device_with_defs());
        view.table.select_first();
        // Move to "named" (second row) which carries named values -> selection dialog.
        view.handle_events(KeyModifiers::NONE, KeyCode::Down);
        assert_eq!(
            view.table.selected().map(|d| d.name.as_str()),
            Some("named")
        );
        view.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(view.is_overlay_active());
        // The selection overlay renders with the "Edit" box title and the named-value label.
        let area = Rect::new(0, 0, 80, 48);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 48)).unwrap();
        term.draw(|f: &mut Frame| view.render_overlay(f, area))
            .unwrap();
        let text = buffer_text(term.backend().buffer());
        assert!(text.contains("Edit"), "missing dialog title:\n{text}");
        assert!(text.contains("on"), "missing named value label:\n{text}");
    }

    #[test]
    fn ut_render_shows_table_and_offline_status() {
        let mut view = view_for(device_with_defs());
        let area = Rect::new(0, 0, 120, 24);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).unwrap();
        term.draw(|f: &mut Frame| view.render(f, area)).unwrap();
        let text = buffer_text(term.backend().buffer());
        // Table title, a register row, and the not-started status line are all drawn.
        assert!(text.contains("Register"), "missing table title:\n{text}");
        assert!(text.contains("hold"), "missing register row:\n{text}");
        assert!(
            text.contains("DISCONNECTED"),
            "missing status line:\n{text}"
        );
    }

    #[tokio::test]
    /// MB-R-137 — a client view against an unreachable TCP peer shows RECONNECTING (not
    /// DISCONNECTED) once its task starts backing off, distinguishing "task is running but not
    /// currently connected" from "not started/stopped".
    async fn it_modbus_client_view_shows_reconnecting_while_backing_off() {
        let mut device = empty_device();
        device.timeout_ms = Some(200);
        device.reconnect = Some(true);
        let spec = ModuleSpec {
            name: "test module".into(),
            device: String::new(),
            role: Role::Client,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: reserve_tcp_port().release(),
            },
        };
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);
        view.module
            .start()
            .await
            .expect("start must not fail synchronously");

        let area = Rect::new(0, 0, 120, 24);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).unwrap();
        let mut text = String::new();
        for _ in 0..100 {
            term.draw(|f: &mut Frame| view.render(f, area)).unwrap();
            text = buffer_text(term.backend().buffer());
            if text.contains("RECONNECTING") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            text.contains("RECONNECTING"),
            "missing status line:\n{text}"
        );

        view.module.stop().await.expect("cleanup stop");
    }

    #[tokio::test]
    /// MB-R-153 — a server view whose bind target is already occupied shows RECONNECTING (not
    /// DISCONNECTED) while its task backs off retrying the bind, per the occupier idiom
    /// established in `instance/mod.rs`'s `it_server_stop_on_backing_off_task_ends_promptly`.
    async fn it_modbus_server_view_shows_reconnecting_while_bind_backs_off() {
        let occupier = reserve_tcp_port();
        let port = occupier.port();

        let mut device = empty_device();
        device.timeout_ms = Some(200);
        let spec = ModuleSpec {
            name: "test module".into(),
            device: String::new(),
            role: Role::Server,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port,
            },
        };
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);
        view.module
            .start()
            .await
            .expect("start must not fail synchronously");

        let area = Rect::new(0, 0, 120, 24);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).unwrap();
        let mut text = String::new();
        for _ in 0..100 {
            term.draw(|f: &mut Frame| view.render(f, area)).unwrap();
            text = buffer_text(term.backend().buffer());
            if text.contains("RECONNECTING") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            text.contains("RECONNECTING"),
            "missing status line:\n{text}"
        );

        view.module.stop().await.expect("cleanup stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// UI-R-314 — `:stop` against a module whose task is genuinely alive (backing off from an
    /// occupied port, so a blocking `stop()` would need the full grace period) returns
    /// `Handled(None)` immediately instead of waiting for the task to end.
    async fn ut_stop_command_returns_without_waiting() {
        let occupier = reserve_tcp_port();
        let port = occupier.port();

        let mut device = empty_device();
        device.timeout_ms = Some(200);
        let spec = ModuleSpec {
            name: "test module".into(),
            device: String::new(),
            role: Role::Server,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port,
            },
        };
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);
        view.module
            .start()
            .await
            .expect("start must not fail synchronously");

        let before = std::time::Instant::now();
        let result = view.handle_command("stop").await;
        assert!(
            before.elapsed() < std::time::Duration::from_millis(50),
            "handle_command(\"stop\") took {:?}, expected to return immediately",
            before.elapsed()
        );
        assert!(matches!(result, CommandResult::Handled(None)));
        assert!(view.lifecycle_pending());

        // Drive the deferred stop to completion so the test doesn't leak a background task.
        for _ in 0..200 {
            if !view.lifecycle_pending() {
                break;
            }
            view.refresh().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!view.lifecycle_pending());
    }

    #[tokio::test]
    /// UI-R-315 — once `refresh()` observes the deferred stop's completion, the outcome (never
    /// carried back as `:stop`'s own immediate result) lands in the view's log at `Info`.
    async fn ut_refresh_logs_stop_outcome() {
        let mut view = new_view();
        view.module.start().await.expect("start");

        let result = view.handle_command("stop").await;
        assert!(matches!(result, CommandResult::Handled(None)));

        for _ in 0..200 {
            if !view.lifecycle_pending() {
                break;
            }
            view.refresh().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!view.lifecycle_pending());

        let lines = view
            .log()
            .read()
            .await
            .peek_n(crate::app::LOG_SIZE)
            .into_iter()
            .map(|(_, level, l)| (level, l))
            .collect::<Vec<_>>();
        assert!(
            lines
                .iter()
                .any(|(level, l)| *level == Level::Info && l == "Stopped Server"),
            "missing 'Stopped Server' Info line: {lines:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// UI-R-314/UI-R-315 — `:restart` against a module whose task is genuinely alive returns
    /// `Handled(None)` immediately, and the module is running again only once `refresh()` has
    /// settled the deferred stop and run the follow-up start.
    async fn ut_restart_defers_start_until_stop_completes() {
        let mut device = empty_device();
        device.timeout_ms = Some(200);
        let spec = ModuleSpec {
            name: "test module".into(),
            device: String::new(),
            role: Role::Server,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 0,
            },
        };
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);
        view.module.start().await.expect("start");

        let before = std::time::Instant::now();
        let result = view.handle_command("restart").await;
        assert!(
            before.elapsed() < std::time::Duration::from_millis(50),
            "handle_command(\"restart\") took {:?}, expected to return immediately",
            before.elapsed()
        );
        assert!(matches!(result, CommandResult::Handled(None)));
        assert!(view.lifecycle_pending());

        for _ in 0..200 {
            if !view.lifecycle_pending() {
                break;
            }
            view.refresh().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!view.lifecycle_pending());
        assert!(
            view.module.is_instance_active(),
            "the follow-up start must have run once the deferred stop settled"
        );

        view.module.stop().await.expect("cleanup stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// UI-R-315 — `:restart` issued while a `:stop` is already pending must not be rejected as
    /// `NotRunning` (the instance is `Stopping`, not `Idle`) and silently drop the earlier
    /// command's outcome forever: it overwrites the pending follow-up, and `refresh()` still
    /// settles it and runs the restart's start.
    async fn ut_restart_while_stop_pending_overwrites_the_follow_up() {
        let occupier = reserve_tcp_port();
        let port = occupier.port();

        let mut device = empty_device();
        device.timeout_ms = Some(200);
        let spec = ModuleSpec {
            name: "test module".into(),
            device: String::new(),
            role: Role::Server,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port,
            },
        };
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);
        view.module.start().await.expect("start");

        assert!(matches!(
            view.handle_command("stop").await,
            CommandResult::Handled(None)
        ));
        assert!(view.lifecycle_pending());

        assert!(matches!(
            view.handle_command("restart").await,
            CommandResult::Handled(None)
        ));
        assert!(
            view.lifecycle_pending(),
            "the follow-up must still be pending, not dropped"
        );

        for _ in 0..200 {
            if !view.lifecycle_pending() {
                break;
            }
            view.refresh().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            !view.lifecycle_pending(),
            "a stop overwritten with a restart must still settle, not latch forever"
        );
        assert!(
            view.module.is_instance_active(),
            "the restart's follow-up start must have run, not the stale stop's no-op"
        );

        view.module.stop().await.expect("cleanup stop");
    }

    #[test]
    fn ut_render_overlay_add_dialog_shows_box_title_and_fields() {
        let mut view = new_view();
        drop(view.handle_command("add"));
        let area = Rect::new(0, 0, 80, 52);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 52)).unwrap();
        term.draw(|f: &mut Frame| view.render_overlay(f, area))
            .unwrap();
        let text = buffer_text(term.backend().buffer());
        assert!(text.contains("Add"), "missing dialog title:\n{text}");
        assert!(text.contains("Label"), "missing label field:\n{text}");
        assert!(text.contains("CONFIRM"), "missing confirm button:\n{text}");
    }

    #[test]
    fn ut_raw_hex_formats_words_lowercase_space_separated() {
        assert_eq!(raw_hex(&[]), "[]");
        assert_eq!(raw_hex(&[0x0001]), "[0001]");
        assert_eq!(raw_hex(&[0x00a0, 0x0001]), "[00a0 0001]");
        assert_eq!(raw_hex(&[0xffff, 0x0000]), "[ffff 0000]");
    }

    #[test]
    /// UI-R-016 — `:set` argument parsing splits on the first whitespace.
    fn ut_parse_set_args_unquoted_splits_on_first_whitespace() {
        assert_eq!(parse_set_args("reg 123"), ("reg".into(), "123".into()));
        // Extra leading whitespace before the value is trimmed.
        assert_eq!(parse_set_args("reg   123"), ("reg".into(), "123".into()));
        // No value -> empty string.
        assert_eq!(parse_set_args("reg"), ("reg".into(), String::new()));
    }

    #[test]
    /// UI-R-016 — a quoted `:set` name keeps its inner spaces.
    fn ut_parse_set_args_quoted_name_keeps_inner_spaces() {
        assert_eq!(
            parse_set_args("\"my reg\" 456"),
            ("my reg".into(), "456".into())
        );
        assert_eq!(
            parse_set_args("\"my reg\" hello world"),
            ("my reg".into(), "hello world".into())
        );
        // Quoted name, no value.
        assert_eq!(
            parse_set_args("\"my reg\""),
            ("my reg".into(), String::new())
        );
    }

    #[tokio::test]
    /// Applying a server-role setup preserves the existing reconnect setting.
    async fn ut_apply_setup_server_role_preserves_existing_reconnect() {
        // Reconnect is hidden/unset (None) for Server-role dialog saves; applying it must not
        // clobber whatever the device config already had for a setting the user never saw.
        let mut view = new_view();
        view.device.reconnect = Some(false);
        let values = SetupValues {
            name: "test module".into(),
            config_path: String::new(),
            role: Role::Server,
            endpoint: Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 5020,
            },
            timeout_ms: None,
            delay_ms: None,
            interval_ms: None,
            reconnect: None,
            read_ranges: Default::default(),
            tls: Default::default(),
        };
        view.apply_setup(values).await;
        assert_eq!(view.device.reconnect, Some(false));
    }

    #[tokio::test]
    /// MB-R-098 — stopping an already-stopped instance during `:restart` is the expected
    /// no-op, not a reportable stop failure, so the restart is surfaced as Info.
    async fn ut_restart_from_stopped_does_not_report_stop_failure() {
        // Bind ephemerally so the restart's start() is deterministic. The endpoint must be
        // ephemeral *before* module construction: `ModbusModule::new` snapshots the spec, so
        // mutating `view.spec` afterwards would leave the module bound to the fixture port.
        let device = empty_device();
        let mut spec = tcp_server_spec();
        spec.endpoint = Endpoint::Tcp {
            ip: "127.0.0.1".into(),
            port: 0,
        };
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);
        // The module is not running: stop() yields NotRunning, which must be swallowed.
        let result = view.handle_command("restart").await;
        match result {
            CommandResult::Handled(Some((level, msg))) => {
                assert!(
                    matches!(level, Level::Info),
                    "benign not-running stop must not surface as Error: {msg}"
                );
                assert!(
                    !msg.contains("stop of previous instance failed"),
                    "unexpected stop-failure text: {msg}"
                );
            }
            _ => panic!("restart should be handled with a log line"),
        }
        // Tear down the server task spawned by the restart.
        let _ = view.handle_command("stop").await;
    }

    #[tokio::test]
    /// tui/api-contract §2.1 — `:write-device [path]` is the long spelling of `:wd` and saves
    /// the device config the same way.
    async fn ut_write_device_alias_saves_like_wd() {
        let mut view = new_view();
        let dir = reserve_temp_dir("ferrowl_modbus_view");
        let path = dir.join("write-device.toml");
        let p = path.to_str().expect("temp path is valid UTF-8").to_string();
        let result = view.handle_command(&format!("write-device {p}")).await;
        match result {
            CommandResult::Handled(Some((_, msg))) => {
                assert!(msg.contains("Saved device config"), "unexpected: {msg}");
            }
            _ => panic!(":write-device should be handled with a save message"),
        }
        assert!(path.exists());
    }

    #[tokio::test]
    /// MB-R-150 — `:reload` rebuilds `self.module` fresh; the session-wide serial-path registry
    /// attached via `set_serial_paths` must carry over to that fresh instance, not reset to a
    /// private default (which would silently drop an in-progress conflict on reload).
    async fn ut_view_reload_carries_serial_paths_registry_to_new_module() {
        use crate::config::{Endpoint, ModuleSpec, Role};
        use crate::module::modbus::SerialPathRegistry;

        let dir = reserve_temp_dir("ferrowl_modbus_view");
        let path = dir.join("reload-serial-paths.toml");
        let p = path.to_str().expect("temp path is valid UTF-8").to_string();

        // Produce a loadable device config file via the already-tested :write-device path.
        let mut writer = new_view();
        let _ = writer.handle_command(&format!("write-device {p}")).await;

        let serial_path = "/nonexistent/mb-r-150-reload";
        let spec = ModuleSpec {
            name: "A".into(),
            device: p.clone(),
            role: Role::Client,
            endpoint: Endpoint::Rtu {
                path: serial_path.into(),
                baud_rate: 9600,
                parity: None,
                data_bits: None,
                stop_bits: None,
            },
        };
        let device = empty_device();
        let module = super::super::ModbusModule::new(&spec, &device);
        let mut view = ModbusModuleView::new(module, spec, device);

        // Another instance ("B") already claims the same serial path in a session-wide registry.
        // `start()` claims optimistically as soon as the background task spawns (MB-R-150's OS-
        // level conflict check happens inside that task), so the registry itself — not the
        // reload's returned message — is what proves whether the fresh module used the
        // session-wide registry or fell back to a private default.
        let registry = SerialPathRegistry::new();
        registry.claim("B", serial_path);
        view.set_serial_paths(registry.clone());

        let _ = view.handle_command("reload").await;
        assert_eq!(
            registry.conflict("B", serial_path),
            Some("A".to_string()),
            "reload's fresh module lost the session-wide registry (claimed on a private default \
             instead)"
        );

        let _ = view.handle_command("stop").await;
    }

    #[tokio::test]
    /// TUI edge case 6.8 — commands match on the exact first token: `:setfoo` is unknown, not a
    /// malformed `:set`; bare `:set` still reports the usage warning.
    async fn ut_set_prefix_typo_is_unknown_bare_set_warns() {
        let mut view = new_view();
        assert!(matches!(
            view.handle_command("setfoo 1 2").await,
            CommandResult::Unhandled
        ));
        match view.handle_command("set").await {
            CommandResult::Handled(Some((level, msg))) => {
                assert!(matches!(level, Level::Warning));
                assert!(msg.contains(":set requires"), "unexpected: {msg}");
            }
            _ => panic!("bare :set should warn about usage"),
        }
    }
}
