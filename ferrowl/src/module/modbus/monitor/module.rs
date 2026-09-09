//! `ModbusMonitorModule` (MB-R-076, MB-R-140–145): a monitor's construction/start/stop
//! lifecycle. Unlike [`super::super::module::ModbusModule`] there is no `Instance<T>`, no
//! operations list, no virtual store, and no Lua sim surface — a monitor owns exactly one log
//! and one observed-value table, receive-only.

use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::app::LogRing;
use crate::config::device::MonitorRegisterDef;
use crate::config::{Endpoint, ModuleSpec, MonitorDeviceConfig};
use ferrowl_modbus::LogFn;
use ferrowl_modbus::ServerCommand;
use ferrowl_modbus::UnitId;
use ferrowl_modbus::monitor::{RecordLog, SharedObservedTable, SharedRecordLog};

use super::super::log::{FileSink, open_sink};
use super::super::serial_paths::SerialPathRegistry;
use super::build::{MonitorNetConfig, MonitorTransportError, endpoint_to_monitor_config};

pub type ModuleLog = Arc<RwLock<LogRing>>;

/// Errors from a monitor's own lifecycle: either the role/transport compatibility check
/// (MB-R-140) or a network error surfaced from the running receive task.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Network error: {0}")]
    Net(#[from] ferrowl_modbus::Error),
    #[error(transparent)]
    Transport(#[from] MonitorTransportError),
}

/// One monitor instance: a receive-only observer of an `Rtu`/`Ascii` bus, its register
/// interpretations, observed-value table, and log — no operations list, no memory store, no
/// Lua sim (MB-R-076).
pub struct ModbusMonitorModule {
    name: String,
    endpoint: Endpoint,
    reconnect: bool,
    interpretations: std::collections::BTreeMap<UnitId, Vec<(String, MonitorRegisterDef)>>,
    table: SharedObservedTable,
    records: SharedRecordLog,
    log: ModuleLog,
    file_sink: FileSink,
    command_tx: Option<Sender<ServerCommand>>,
    task: Option<JoinHandle<Result<(), ferrowl_modbus::Error>>>,
    /// MB-R-152 — the "serial port open" signal, mirroring `ServerHandle::open`
    /// (`instance/handle.rs`) but read directly off the builder's returned
    /// `ConnectedCell` rather than routed through an `Instance`.
    open: ferrowl_modbus::ConnectedCell,
    /// MB-R-150 — the session-wide registry this instance's path-conflict check consults.
    /// Defaults to a private, unshared registry until `set_serial_paths` attaches the real
    /// session registry.
    serial_paths: SerialPathRegistry,
}

impl ModbusMonitorModule {
    /// Build a monitor module from an instance spec and its device-type config. The
    /// endpoint/transport is validated later, at [`start`](Self::start) — construction never
    /// fails.
    pub fn new(spec: &ModuleSpec, device: &MonitorDeviceConfig) -> Self {
        let mut interpretations: std::collections::BTreeMap<
            UnitId,
            Vec<(String, MonitorRegisterDef)>,
        > = std::collections::BTreeMap::new();
        for def in device.definitions.iter() {
            interpretations
                .entry(UnitId(def.slave_id))
                .or_default()
                .push((def.name.clone(), def.clone()));
        }

        let file_sink: FileSink = Arc::new(std::sync::Mutex::new(None));
        let _ = open_sink(&file_sink, device.log_file.as_deref(), &spec.name);

        Self {
            name: spec.name.clone(),
            endpoint: spec.endpoint.clone(),
            reconnect: device
                .reconnect
                .unwrap_or(crate::config::device::DEFAULT_RECONNECT),
            interpretations,
            table: Arc::new(parking_lot::RwLock::new(
                ferrowl_modbus::monitor::ObservedTable::default(),
            )),
            records: Arc::new(parking_lot::RwLock::new(RecordLog::default())),
            log: Arc::new(RwLock::new(LogRing::init())),
            file_sink,
            command_tx: None,
            task: None,
            open: ferrowl_modbus::ConnectedCell::default(),
            serial_paths: SerialPathRegistry::default(),
        }
    }

    /// MB-R-150 — attach this session's live Rtu/Ascii path-conflict registry. A monitor that
    /// never receives this call keeps the default, unshared registry (never conflicts with
    /// anything) — the pre-feature behavior, still correct for standalone/test use.
    pub fn set_serial_paths(&mut self, registry: SerialPathRegistry) {
        self.serial_paths = registry;
    }

    /// Reconfigure an existing monitor instance from a
    /// (possibly edited) spec/device without resetting its accumulated observed state: the
    /// counterpart to `ModbusModule::reconfigure` (`module/modbus/module.rs`), but consuming
    /// `self` and returning a new `Self` rather than mutating in place, since (unlike the full
    /// client/server module) a monitor's edit-confirm path is not `async` and so cannot
    /// gracefully `.await` a stop/restart choreography here.
    ///
    /// `table`/`records`/`log`/`interpretations` carry over unchanged (the same `Arc`-shared
    /// instances, not fresh empty ones) — the whole point of this method over calling `new()`
    /// again. `name`/`endpoint`/`reconnect`/`file_sink` rebuild from `spec`/`device`, exactly as
    /// `new()` would.
    ///
    /// Any previously running connection (`command_tx`/`task`) is dropped rather than carried
    /// over: the sender is simply dropped (closing the channel), and any still-running task is
    /// `abort()`ed outright (the same fallback `stop()` itself uses when a graceful
    /// `Terminate`-then-wait doesn't finish in time) since there is no `async` context here to
    /// await a graceful stop. The caller is expected to `:start` again afterwards, exactly as it
    /// already had to after a fresh `new()`'d module.
    pub fn reconfigure(self, spec: &ModuleSpec, device: &MonitorDeviceConfig) -> Self {
        // MB-R-150 — release any claim this instance held before rebuilding: "recovers…once the
        // conflicting instance stops" applies just as much to a reconfigure as to a stop.
        self.serial_paths.release(&self.name);
        if let Some(task) = self.task {
            task.abort();
        }

        let file_sink: FileSink = Arc::new(std::sync::Mutex::new(None));
        let _ = open_sink(&file_sink, device.log_file.as_deref(), &spec.name);

        Self {
            name: spec.name.clone(),
            endpoint: spec.endpoint.clone(),
            reconnect: device
                .reconnect
                .unwrap_or(crate::config::device::DEFAULT_RECONNECT),
            interpretations: self.interpretations,
            table: self.table,
            records: self.records,
            log: self.log,
            file_sink,
            command_tx: None,
            task: None,
            open: ferrowl_modbus::ConnectedCell::default(),
            serial_paths: self.serial_paths,
        }
    }

    pub fn table(&self) -> SharedObservedTable {
        self.table.clone()
    }

    pub fn records(&self) -> SharedRecordLog {
        self.records.clone()
    }

    pub fn log(&self) -> ModuleLog {
        self.log.clone()
    }

    /// Whether the monitor's receive task is currently running (started and not yet stopped),
    /// the connection-state signal the view's status line drives off of,
    /// mirroring `ModbusModule::bound_addr` for the full
    /// client/server module (a monitor is RTU/ASCII-only, so there is no bound TCP address to
    /// report instead).
    pub fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// MB-R-152 — see `Instance::connection_status` (`instance/mod.rs`) for the shared
    /// derivation this mirrors.
    pub fn connection_status(&self) -> crate::view::status_bar::ConnStatus {
        use crate::view::status_bar::ConnStatus;
        if !self.is_running() {
            return ConnStatus::Disconnected;
        }
        if self.open.get() {
            ConnStatus::Connected
        } else {
            ConnStatus::Reconnecting
        }
    }

    /// #219 — rebuild the on-disk `definitions` list (`MonitorDeviceConfig::definitions`) from
    /// the live in-memory interpretations, each entry carrying its own `name`/`slave_id` (kept in
    /// sync by `add_interpretation`/`edit_interpretation`). A list, not a name-keyed map: two
    /// units may legitimately hold a same-named interpretation (MB-R-148 scopes edit/remove to
    /// one slave id's set), and collapsing them by name here would silently lose one on save. The
    /// caller assigns this into `self.device.definitions` after any interpretation add/edit/
    /// delete so `:write` (and any path that reconstructs from `self.device`) never silently
    /// drops a runtime change — mirroring `ModbusModule`'s own `apply_add`/`delete_register_by_name`
    /// (`module/modbus/view/mutate.rs`), which keep `self.device.definitions` in sync at each
    /// mutation site rather than only at save time.
    pub fn definitions(&self) -> Vec<MonitorRegisterDef> {
        self.interpretations
            .values()
            .flatten()
            .map(|(name, def)| MonitorRegisterDef {
                name: name.clone(),
                ..def.clone()
            })
            .collect()
    }

    /// Interpretations for one unit id, empty (not absent/panicking) for a unit id with none.
    pub fn interpretations_for(&self, unit: UnitId) -> &[(String, MonitorRegisterDef)] {
        self.interpretations.get(&unit).map_or(&[], Vec::as_slice)
    }

    /// Append a brand-new interpretation to `unit`'s cached list, forcing `def.slave_id` to
    /// match `unit` so the map key and the def's own field never drift.
    pub fn add_interpretation(&mut self, unit: UnitId, name: String, mut def: MonitorRegisterDef) {
        def.slave_id = unit.0;
        self.interpretations
            .entry(unit)
            .or_default()
            .push((name, def));
    }

    /// MB-R-148 — edit an existing interpretation in place, possibly under a new name.
    /// No-op (returns `false`) if `unit`/`old_name` isn't found.
    pub fn edit_interpretation(
        &mut self,
        unit: UnitId,
        old_name: &str,
        new_name: String,
        mut def: MonitorRegisterDef,
    ) -> bool {
        def.slave_id = unit.0;
        let Some(list) = self.interpretations.get_mut(&unit) else {
            return false;
        };
        let Some(entry) = list.iter_mut().find(|(n, _)| n == old_name) else {
            return false;
        };
        *entry = (new_name, def);
        true
    }

    /// MB-R-148 — remove an interpretation from `unit`'s cached list by name (no-op if absent).
    pub fn remove_interpretation(&mut self, unit: UnitId, name: &str) {
        if let Some(list) = self.interpretations.get_mut(&unit) {
            list.retain(|(n, _)| n != name);
        }
    }

    /// Start the monitor: validate the endpoint/transport (MB-R-140), spawn the receive-only
    /// task, and route its log/status lines into the ring log and (if configured) the
    /// per-module log file.
    pub async fn start<L, S>(&mut self, log: L, status: S) -> Result<(), Error>
    where
        L: LogFn + Clone,
        S: LogFn + Clone,
    {
        let net_config = endpoint_to_monitor_config(&self.endpoint, self.reconnect)?;

        let (tx, rx) = tokio::sync::mpsc::channel::<ServerCommand>(10);

        let log_ring = self.log.clone();
        let log_sink = self.file_sink.clone();
        let log_cb = move |s: String| {
            let log_ring = log_ring.clone();
            let log_sink = log_sink.clone();
            async move {
                log_ring.write().await.write(network_log_level(&s), &s);
                super::super::log::append(&log_sink, &s);
            }
        };

        let status_ring = self.log.clone();
        let status_sink = self.file_sink.clone();
        let status_cb = move |s: String| {
            let status_ring = status_ring.clone();
            let status_sink = status_sink.clone();
            async move {
                let line = format!("[status] {s}");
                status_ring
                    .write()
                    .await
                    .write(network_log_level(&line), &line);
                super::super::log::append(&status_sink, &line);
            }
        };

        // MB-R-150 — the checker consulted before every connect attempt, via the freshly-built
        // builder's `PathConflictCell` (attached before `spawn()`, below).
        let path_conflict_check: ferrowl_modbus::PathConflictCheck = {
            let registry = self.serial_paths.clone();
            let name = self.name.clone();
            Arc::new(move |path: &str| registry.conflict(&name, path))
        };

        let table = self.table.clone();
        let records = self.records.clone();
        let (handle, open) = match net_config {
            MonitorNetConfig::Rtu(cfg) => {
                let builder = ferrowl_modbus::rtu::MonitorBuilder::new(
                    Arc::new(RwLock::new(cfg)),
                    table,
                    records,
                );
                builder.path_conflict().set(path_conflict_check);
                builder.spawn(rx, log_cb, status_cb).await?
            }
            MonitorNetConfig::Ascii(cfg) => {
                let builder = ferrowl_modbus::ascii::MonitorBuilder::new(
                    Arc::new(RwLock::new(cfg)),
                    table,
                    records,
                );
                builder.path_conflict().set(path_conflict_check);
                builder.spawn(rx, log_cb, status_cb).await?
            }
        };

        let _ = log.invoke(format!("Monitor '{}' started", self.name)).await;
        let _ = status;
        self.command_tx = Some(tx);
        self.task = Some(handle);
        self.open = open;
        // MB-R-150 — claim this instance's serial path so the registry can report it as a
        // conflict to any other instance that shares it.
        if let Some(path) = crate::module::modbus::build::endpoint_serial_path(&self.endpoint) {
            self.serial_paths.claim(&self.name, &path);
        }
        Ok(())
    }

    /// Stop the running task: send `Terminate` then, after a grace period, abort if it is still
    /// alive. Mirrors `Instance::stop`'s grace-period-then-abort shape.
    pub async fn stop(&mut self) -> Result<(), Error> {
        let sent_terminate = if let Some(tx) = &self.command_tx {
            tx.send(ServerCommand::Terminate).await.is_ok()
        } else {
            false
        };
        if sent_terminate {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        self.command_tx = None;

        if let Some(handle) = self.task.take() {
            if handle.is_finished() {
                let _ = handle.await;
            } else {
                handle.abort();
                let _ = handle.await;
            }
        }
        // MB-R-150 — release unconditionally, mirroring `ModbusModule::stop`.
        self.serial_paths.release(&self.name);
        // MB-R-152 — a stopped monitor must never read back a stale `true`.
        self.open = ferrowl_modbus::ConnectedCell::default();
        Ok(())
    }
}

/// Classifies a monitor's network/status line for the log ring: reuses
/// [`super::super::module::network_log_level`]'s classification plus a monitor-specific Warning
/// branch for a discarded, malformed frame (MB-R-142) — the decode/match state machine phrases
/// its discarded-frame log line to contain this exact substring.
pub(crate) fn network_log_level(s: &str) -> crate::app::Level {
    if s.to_lowercase().contains("malformed frame") {
        crate::app::Level::Warning
    } else {
        super::super::module::network_log_level(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Role;
    use ferrowl_test_support::reserve_temp_dir;

    fn spec(endpoint: Endpoint) -> ModuleSpec {
        ModuleSpec {
            name: "mon1".to_string(),
            device: "monitor.toml".to_string(),
            role: Role::Monitor,
            endpoint,
        }
    }

    fn device_with_defs() -> MonitorDeviceConfig {
        let definitions = vec![MonitorRegisterDef {
            name: "power".to_string(),
            slave_id: 1,
            kind: ferrowl_codec::Kind::HoldingRegister,
            address: Some(10),
            is_virtual: false,
            value_type: crate::config::device::ValueType::U16,
            endian: Default::default(),
            word_order: Default::default(),
            resolution: 1.0,
            bitmask: None,
            length: 1,
            alignment: Default::default(),
            values: vec![],
            description: String::new(),
            default: None,
        }];
        MonitorDeviceConfig {
            version: None,
            reconnect: Some(true),
            log_file: None,
            definitions,
        }
    }

    fn bad_rtu_endpoint() -> Endpoint {
        Endpoint::Rtu {
            path: "/dev/does-not-exist-ferrowl-monitor-test".to_string(),
            baud_rate: 9600,
            parity: None,
            data_bits: None,
            stop_bits: None,
        }
    }

    /// MB-R-076/145 — a monitor module buckets its interpretations by their own `slave_id` at
    /// construction, and starts with an empty observed-value table, no register set.
    #[test]
    fn ut_monitor_module_new_buckets_interpretations_by_slave_id() {
        let mut device = device_with_defs();
        let mut current = device
            .definitions
            .iter()
            .find(|d| d.name == "power")
            .expect("power def present")
            .clone();
        current.slave_id = 3;
        device.definitions.push(MonitorRegisterDef {
            name: "voltage".to_string(),
            slave_id: 7,
            address: Some(20),
            ..current.clone()
        });
        device.definitions.retain(|d| d.name != "power");
        device.definitions.push(current);

        let module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);
        let unit3 = module.interpretations_for(ferrowl_modbus::UnitId(3));
        let unit7 = module.interpretations_for(ferrowl_modbus::UnitId(7));
        assert_eq!(unit3.len(), 1);
        assert_eq!(unit3[0].0, "power");
        assert_eq!(unit7.len(), 1);
        assert_eq!(unit7[0].0, "voltage");
        assert!(module.table().read().unit_ids().is_empty());
    }

    /// MB-R-148 — editing an interpretation replaces it in place, under a new name if given, and
    /// the module keeps `MonitorRegisterDef::slave_id` in sync with its map key.
    #[test]
    fn ut_edit_interpretation_replaces_in_place_under_new_name() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let def = module.interpretations_for(ferrowl_modbus::UnitId(1))[0]
            .1
            .clone();
        module.add_interpretation(ferrowl_modbus::UnitId(3), "power".to_string(), def.clone());

        let mut edited = def.clone();
        edited.address = Some(99);
        let replaced = module.edit_interpretation(
            ferrowl_modbus::UnitId(3),
            "power",
            "power2".to_string(),
            edited,
        );
        assert!(replaced);

        let unit3 = module.interpretations_for(ferrowl_modbus::UnitId(3));
        assert_eq!(unit3.len(), 1);
        assert_eq!(unit3[0].0, "power2");
        assert_eq!(unit3[0].1.address, Some(99));
        assert_eq!(unit3[0].1.slave_id, 3);
    }

    /// MB-R-148 — editing a name/unit that doesn't exist is a no-op, not a panic.
    #[test]
    fn ut_edit_interpretation_noop_if_absent() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let def = module.interpretations_for(ferrowl_modbus::UnitId(1))[0]
            .1
            .clone();
        let replaced = module.edit_interpretation(
            ferrowl_modbus::UnitId(9),
            "nope",
            "still-nope".to_string(),
            def,
        );
        assert!(!replaced);
    }

    /// MB-R-148 — removing an interpretation deletes it outright, without touching any value
    /// already written into the observed table.
    #[test]
    fn ut_remove_interpretation_deletes_without_touching_table() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let def = module.interpretations_for(ferrowl_modbus::UnitId(1))[0]
            .1
            .clone();
        module.add_interpretation(ferrowl_modbus::UnitId(3), "power".to_string(), def);

        let key = ferrowl_modbus::Key::new(ferrowl_modbus::SlaveKey {
            slave_id: ferrowl_modbus::UnitId(3),
            kind: ferrowl_codec::Kind::HoldingRegister,
        });
        module.table.write().write_words(key.clone(), 10, &[42]);

        module.remove_interpretation(ferrowl_modbus::UnitId(3), "power");
        assert!(
            module
                .interpretations_for(ferrowl_modbus::UnitId(3))
                .is_empty()
        );
        assert_eq!(
            module.table().read().read_words(&key, 10, 1),
            Some(vec![42])
        );
    }

    /// Per-unit-id isolation: editing/removing on one unit id never touches another's set, even
    /// when both hold an interpretation of the same name.
    #[test]
    fn ut_edit_and_remove_scoped_to_their_own_unit_id() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let def = module.interpretations_for(ferrowl_modbus::UnitId(1))[0]
            .1
            .clone();
        module.add_interpretation(ferrowl_modbus::UnitId(3), "power".to_string(), def.clone());
        module.add_interpretation(ferrowl_modbus::UnitId(5), "power".to_string(), def);

        module.remove_interpretation(ferrowl_modbus::UnitId(3), "power");
        assert!(
            module
                .interpretations_for(ferrowl_modbus::UnitId(3))
                .is_empty()
        );
        assert_eq!(
            module.interpretations_for(ferrowl_modbus::UnitId(5)).len(),
            1
        );

        let mut edited = module.interpretations_for(ferrowl_modbus::UnitId(5))[0]
            .1
            .clone();
        edited.address = Some(77);
        module.edit_interpretation(
            ferrowl_modbus::UnitId(5),
            "power",
            "power".to_string(),
            edited,
        );
        assert_eq!(
            module.interpretations_for(ferrowl_modbus::UnitId(5))[0]
                .1
                .address,
            Some(77)
        );
        assert!(
            module
                .interpretations_for(ferrowl_modbus::UnitId(3))
                .is_empty()
        );
    }

    /// `records()` mirrors `table()`'s own accessor shape and starts empty.
    #[test]
    fn ut_monitor_module_records_accessor_starts_empty() {
        let module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        assert!(
            module
                .records()
                .read()
                .records_for(ferrowl_modbus::UnitId(1))
                .is_empty()
        );
    }

    /// MB-R-152 — `is_running()` must not trust a stale `command_tx` alone once the task backing
    /// it has already finished, mirroring `instance/mod.rs`'s own
    /// `send_command_on_server_is_invalid_operation` fixture (a hand-built handle wrapping an
    /// already-finished `tokio::spawn`).
    #[tokio::test]
    async fn ut_monitor_is_running_false_after_task_ends_even_if_command_tx_set() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let task = tokio::spawn(async { Ok(()) });
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        // Give the freshly spawned task a moment to actually finish before asserting.
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        module.command_tx = Some(sender);
        module.task = Some(task);
        assert!(
            !module.is_running(),
            "a finished task must not report running just because command_tx is still Some"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_is_running_tracks_start_stop() {
        let mut device = device_with_defs();
        device.reconnect = Some(false);
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);
        assert!(!module.is_running(), "not running before start()");
        module
            .start(|_: String| async {}, |_: String| async {})
            .await
            .expect("start always succeeds for a valid transport");
        assert!(module.is_running(), "running once start() succeeds");
        module.stop().await.expect("stop");
        assert!(!module.is_running(), "not running once stop() completes");
    }

    /// MB-R-191 — a monitor's start() rejects a non-serial endpoint with the role/transport
    /// compatibility error, the enforcement point nothing can bypass (a hand-edited session
    /// file skips the setup dialog's own check).
    #[tokio::test]
    async fn ut_monitor_module_start_with_tcp_endpoint_fails_with_transport_error() {
        let endpoint = Endpoint::Tcp {
            ip: "127.0.0.1".to_string(),
            port: 502,
        };
        let mut module = ModbusMonitorModule::new(&spec(endpoint), &device_with_defs());
        let err = module
            .start(|_: String| async {}, |_: String| async {})
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Transport(_)));
    }

    /// MB-R-192 — `reconnect: true` against a bad serial path keeps the task retrying (still
    /// running after a short wait); `reconnect: false` ends the task promptly.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_monitor_module_start_stop_lifecycle() {
        let mut device = device_with_defs();
        device.reconnect = Some(true);
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);
        module
            .start(|_: String| async {}, |_: String| async {})
            .await
            .expect("start always succeeds for a valid transport");
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        assert!(
            module.task.as_ref().is_some_and(|h| !h.is_finished()),
            "an open failure with reconnect enabled must keep retrying"
        );
        module.stop().await.expect("stop");

        let mut device = device_with_defs();
        device.reconnect = Some(false);
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);
        module
            .start(|_: String| async {}, |_: String| async {})
            .await
            .expect("start always succeeds for a valid transport");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while module.task.as_ref().is_some_and(|h| !h.is_finished()) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("task should end promptly, not retry, with reconnect disabled");
    }

    /// Editing a running monitor's setup must not reset its accumulated
    /// `table`/`records`/`log`/`interpretations`: `reconfigure` carries the same `Arc`-shared
    /// instances over (`Arc::ptr_eq`, not just equal-by-value) rather than building fresh ones.
    #[test]
    fn ut_reconfigure_preserves_shared_state_arcs() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        module.add_interpretation(
            ferrowl_modbus::UnitId(9),
            "extra".to_string(),
            device_with_defs()
                .definitions
                .iter()
                .find(|d| d.name == "power")
                .unwrap()
                .clone(),
        );
        let table_before = module.table();
        let records_before = module.records();
        let log_before = module.log();

        let new_spec = spec(bad_rtu_endpoint());
        let reconfigured = module.reconfigure(&new_spec, &device_with_defs());

        assert!(
            Arc::ptr_eq(&table_before, &reconfigured.table()),
            "table must be the same shared instance, not a fresh empty one"
        );
        assert!(
            Arc::ptr_eq(&records_before, &reconfigured.records()),
            "records must be the same shared instance, not a fresh empty one"
        );
        assert!(
            Arc::ptr_eq(&log_before, &reconfigured.log()),
            "log must be the same shared instance, not a fresh empty one"
        );
        assert_eq!(
            reconfigured
                .interpretations_for(ferrowl_modbus::UnitId(9))
                .len(),
            1,
            "interpretations added at runtime must survive a reconfigure"
        );
    }

    /// `reconfigure` rebuilds identity/connection fields (name, endpoint,
    /// reconnect) from the new spec/device, same as a fresh `new()`.
    #[test]
    fn ut_reconfigure_rebuilds_identity_fields_from_new_spec_and_device() {
        let module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());

        let new_endpoint = Endpoint::Rtu {
            path: "/dev/ttyUSB9".to_string(),
            baud_rate: 9600,
            parity: None,
            data_bits: None,
            stop_bits: None,
        };
        let mut new_spec = spec(new_endpoint.clone());
        new_spec.name = "mon2".to_string();
        let mut new_device = device_with_defs();
        new_device.reconnect = Some(false);

        let reconfigured = module.reconfigure(&new_spec, &new_device);
        assert_eq!(reconfigured.name, "mon2");
        assert_eq!(reconfigured.endpoint, new_endpoint);
        assert!(!reconfigured.reconnect);
    }

    /// `reconfigure` never leaves a previously running task's `command_tx`/
    /// `task` handle behind (would leak a detached background task); the reconfigured instance
    /// always starts in the same not-yet-started state a fresh `new()` would.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_reconfigure_drops_any_previously_running_connection() {
        let mut device = device_with_defs();
        device.reconnect = Some(true);
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);
        module
            .start(|_: String| async {}, |_: String| async {})
            .await
            .expect("start always succeeds for a valid transport");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let reconfigured = module.reconfigure(&spec(bad_rtu_endpoint()), &device);
        assert!(reconfigured.command_tx.is_none());
        assert!(reconfigured.task.is_none());
    }

    /// MB-R-150 — starting a monitor claims its serial path in the attached registry (visible to
    /// another instance sharing the registry as a conflict); stopping releases it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_monitor_start_claims_serial_path_stop_releases() {
        use crate::module::modbus::SerialPathRegistry;

        let mut device = device_with_defs();
        device.reconnect = Some(false);
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);
        let registry = SerialPathRegistry::new();
        module.set_serial_paths(registry.clone());

        let path = crate::module::modbus::build::endpoint_serial_path(&bad_rtu_endpoint())
            .expect("Rtu endpoint has a serial path");
        let _ = module
            .start(|_: String| async {}, |_: String| async {})
            .await;
        assert_eq!(registry.conflict("B", &path), Some("mon1".to_string()));

        module.stop().await.expect("stop");
        assert_eq!(registry.conflict("B", &path), None);
    }

    /// MB-R-150 — `reconfigure()` releases the previous claim and carries the same shared
    /// registry forward, so a subsequent `start()` on the reconfigured instance still
    /// participates in the same session-wide conflict check.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_monitor_reconfigure_releases_old_claim_and_carries_registry() {
        use crate::module::modbus::SerialPathRegistry;

        let mut device = device_with_defs();
        device.reconnect = Some(false);
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);
        let registry = SerialPathRegistry::new();
        module.set_serial_paths(registry.clone());

        let path = crate::module::modbus::build::endpoint_serial_path(&bad_rtu_endpoint())
            .expect("Rtu endpoint has a serial path");
        let _ = module
            .start(|_: String| async {}, |_: String| async {})
            .await;
        assert_eq!(registry.conflict("B", &path), Some("mon1".to_string()));

        let mut reconfigured = module.reconfigure(&spec(bad_rtu_endpoint()), &device);
        // The old claim must be released by reconfigure itself, before any restart.
        assert_eq!(registry.conflict("B", &path), None);

        let _ = reconfigured
            .start(|_: String| async {}, |_: String| async {})
            .await;
        assert_eq!(registry.conflict("B", &path), Some("mon1".to_string()));
    }

    /// MB-R-150 — a monitor that never receives `set_serial_paths` keeps its own private,
    /// default registry: its claim never lands in any other (e.g. a session-wide) registry.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_monitor_new_without_set_serial_paths_never_conflicts() {
        use crate::module::modbus::SerialPathRegistry;

        let mut device = device_with_defs();
        device.reconnect = Some(false);
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device);

        let path = crate::module::modbus::build::endpoint_serial_path(&bad_rtu_endpoint())
            .expect("Rtu endpoint has a serial path");
        let _ = module
            .start(|_: String| async {}, |_: String| async {})
            .await;

        let unrelated_registry = SerialPathRegistry::new();
        assert_eq!(unrelated_registry.conflict("B", &path), None);
    }

    /// network_log_level's monitor-specific Warning branch: a discarded malformed-frame line
    /// classifies as Warning, matching the decode/match state machine's log wording.
    #[test]
    fn ut_network_log_level_classifies_malformed_frame_as_warning() {
        let line = "Discarding malformed frame (checksum mismatch): 01 02 03";
        assert_eq!(network_log_level(line), crate::app::Level::Warning);
    }

    /// `definitions()` rebuilds the flat on-disk map from whatever is
    /// currently in the in-memory interpretations map, including entries added purely at
    /// runtime (never present in the `MonitorDeviceConfig` the module was constructed from).
    #[test]
    fn ut_definitions_rebuilds_flat_map_including_runtime_additions() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let def = module.interpretations_for(ferrowl_modbus::UnitId(1))[0]
            .1
            .clone();
        module.add_interpretation(ferrowl_modbus::UnitId(9), "extra".to_string(), def);

        let defs = module.definitions();
        assert!(
            defs.iter().any(|d| d.name == "power"),
            "original def must survive"
        );
        let extra = defs
            .iter()
            .find(|d| d.name == "extra")
            .expect("runtime-added def must be included, not silently dropped");
        assert_eq!(extra.slave_id, 9);
    }

    /// #219 — two same-named interpretations on different unit ids must both survive
    /// `definitions()` (the on-disk-shaped snapshot `:wd` persists), not collapse last-wins.
    #[test]
    fn ut_definitions_keeps_same_name_distinct_across_units() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let def = module.interpretations_for(ferrowl_modbus::UnitId(1))[0]
            .1
            .clone();
        module.add_interpretation(ferrowl_modbus::UnitId(9), "power".to_string(), def);

        let defs = module.definitions();
        assert_eq!(
            defs.len(),
            2,
            "both unit 1's and unit 9's \"power\" interpretation must survive distinctly"
        );
    }

    /// #219 — the exact reported scenario: two same-named interpretations on different unit ids
    /// must survive a full `:wd`-equivalent save/reload round trip (`definitions()` -> TOML file
    /// -> `load_monitor_device` -> a fresh `ModbusMonitorModule::new`), not just an in-process
    /// snapshot.
    #[test]
    fn ut_same_name_across_units_survives_save_reload_round_trip() {
        let mut module = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &device_with_defs());
        let def = module.interpretations_for(ferrowl_modbus::UnitId(1))[0]
            .1
            .clone();
        module.add_interpretation(ferrowl_modbus::UnitId(9), "power".to_string(), def);

        let device = MonitorDeviceConfig {
            version: None,
            reconnect: Some(true),
            log_file: None,
            definitions: module.definitions(),
        };
        let dir = reserve_temp_dir("ferrowl_modbus_monitor_module");
        let path = dir
            .join("same-name-roundtrip.toml")
            .to_string_lossy()
            .into_owned();
        ferrowl_util::convert::Converter::save(
            &device,
            &path,
            ferrowl_util::convert::FileType::Toml,
        )
        .expect("save");
        let reloaded = crate::config::load_monitor_device(&path).expect("load");

        let reconstructed = ModbusMonitorModule::new(&spec(bad_rtu_endpoint()), &reloaded);
        assert_eq!(
            reconstructed
                .interpretations_for(ferrowl_modbus::UnitId(1))
                .len(),
            1,
            "unit 1's \"power\" must survive the round trip"
        );
        assert_eq!(
            reconstructed
                .interpretations_for(ferrowl_modbus::UnitId(9))
                .len(),
            1,
            "unit 9's \"power\" must survive the round trip, not be collapsed into unit 1's"
        );
    }
}
