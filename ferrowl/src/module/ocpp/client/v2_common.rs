//! Shared OCPP 2.x charging-station bindings, used by both 2.0.1 and 2.1.
//!
//! 2.1 is a strict superset of 2.0.1 and the simulator answers the same core Calls the same way, so
//! the `ClientVersion` body lives here once as plain free functions and each version's
//! `impl ClientVersion for V…` (in `v2_0_1/version.rs` / `v2_1/version.rs`) delegates to them —
//! only the action-spec module actually differs per version, and that seam stays in each version's
//! own `impl` block. Both versions share the one
//! [`crate::module::ocpp::client::v2_0_1::state::CsState`].
//!
//! The inbound (CSMS→CS) handler's *plumbing* is shared: the single generic `CsStateHandler` in
//! [`crate::module::ocpp::client::handler`] records each Call/reply, tags scope, and runs the
//! unknown-EVSE guard, then delegates to `V::respond`. The decision logic itself is fully typed and
//! **not** shared — it lives per version in `v2_0_1/inbound.rs` / `v2_1/inbound.rs` (the `Inbound`
//! impls), which read typed request fields and build typed `rust_ocpp` responses. The
//! version-independent helpers those impls and the plumbing call (`unknown_evse`, `inbound_scope`,
//! `clear_limit_by_purpose`, …) live here.

use crate::module::ocpp::client::backend::{boot_interval, rfc3339_now};
use crate::module::ocpp::client::config::ConfigKey;
use crate::module::ocpp::client::v2_0_1::state::CsState;
use crate::module::ocpp::client::view::{
    ClientState, EditField, EditKind, EditOverlay, NvRowData, PHASE_CHOICES, ResolvedEdit, choice,
    number, parse_id, text_input,
};
use crate::module::ocpp::config::device::ConnectorRef;
use crate::module::ocpp::scope::{Scope, evse_id};
use ferrowl_lua::module::ValueType;

/// Clear the per-purpose charge limit a ClearChargingProfile targets: the field matching `purpose`,
/// or every per-purpose limit when no purpose criterion is given. An unknown purpose clears nothing.
pub(crate) fn clear_limit_by_purpose(
    c: &mut crate::module::ocpp::client::v2_0_1::state::ConnectorState,
    purpose: Option<&str>,
) {
    match purpose {
        Some("TxProfile") => c.limit = None,
        Some("TxDefaultProfile") => c.default_limit = None,
        Some("ChargingStationMaxProfile") => c.max_limit = None,
        Some("ChargingStationExternalConstraints") => c.external_limit = None,
        Some(_) => {}
        None => {
            c.limit = None;
            c.default_limit = None;
            c.max_limit = None;
            c.external_limit = None;
        }
    }
}

/// The EVSE id an inbound Call targets, from a nested `evse.id` or a top-level `evseId`. A bare
/// `connectorId` is ignored: in 2.0.1 messages are addressed by EVSE only.
pub(crate) fn inbound_evse(request: &serde_json::Value) -> Option<i64> {
    evse_id(request)
}

/// Scope an inbound CSMS→CS Call belongs to, for the message log: keyed by EVSE id (connector kept
/// `None`), or CS-level when no EVSE is addressed.
pub(crate) fn inbound_scope(request: &serde_json::Value) -> Scope {
    match inbound_evse(request) {
        Some(e) => Scope::evse(e, None),
        None => Scope::CS,
    }
}

/// An addressed EVSE id this charging station does not have, if any. EVSE `0` is the charge point
/// itself and is always valid; an absent EVSE is CS-level.
pub(crate) fn unknown_evse(request: &serde_json::Value, state: &CsState) -> Option<i64> {
    let e = inbound_evse(request)?;
    if e == 0 || state.connectors.iter().any(|c| c.evse_id == e) {
        None
    } else {
        Some(e)
    }
}

// Shared concrete `ClientState` over `CsState` (defined once; both versions reuse it).

impl ClientState for CsState {
    fn connector_count(&self) -> usize {
        self.connectors.len()
    }
    fn clear_connectors(&mut self) {
        self.connectors.clear();
    }
    fn remove_connector_at(&mut self, idx: usize) {
        self.connectors.remove(idx);
    }
    fn connector_position(&self, connector_id: i64) -> Option<usize> {
        self.connectors
            .iter()
            .position(|c| c.connector_id == connector_id)
    }
    fn conn_get_field(&self, idx: usize, name: &str) -> Option<ValueType> {
        self.connectors.get(idx).and_then(|c| c.get_field(name))
    }
    fn cs_get_field_named(&self, name: &str) -> Option<ValueType> {
        self.cs_get_field(name)
    }
    fn cs_state_rows(&self) -> Vec<NvRowData> {
        CsState::cs_rows(self)
            .into_iter()
            .map(|r| NvRowData {
                name: r.name,
                unit: r.unit,
                value: r.value,
            })
            .collect()
    }
    fn conn_state_rows(&self, idx: usize) -> Vec<NvRowData> {
        self.connectors
            .get(idx)
            .map(|c| {
                c.rows()
                    .into_iter()
                    .map(|r| NvRowData {
                        name: r.name,
                        unit: r.unit,
                        value: r.value,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn config(&self) -> &[ConfigKey] {
        &self.config
    }
    fn config_mut(&mut self) -> &mut Vec<ConfigKey> {
        &mut self.config
    }
    fn heartbeat_interval_secs(&self) -> Option<u64> {
        self.heartbeat_interval_secs
    }
}

// Shared `ClientVersion` body (both 2.0.1 and 2.1's `impl` blocks delegate to these).

const STATUS_CHOICES: [&str; 5] = [
    "Available",
    "Occupied",
    "Reserved",
    "Unavailable",
    "Faulted",
];

/// State-driven real actions: built straight from state, no dialog.
pub(crate) const STATE_DRIVEN: [&str; 5] = [
    "Authorize",
    "BootNotification",
    "Heartbeat",
    "MeterValues",
    "StatusNotification",
];

pub(crate) fn config_title() -> &'static str {
    "Variables"
}

pub(crate) fn add_connector_placeholder() -> &'static str {
    "Add evse/connector"
}

pub(crate) fn has_tx_shortcuts() -> bool {
    true
}

pub(crate) fn scope_of(s: &CsState, idx: usize) -> Scope {
    Scope::evse(s.connectors[idx].evse_id, None)
}

/// Resolve the connector index targeted by `scope` (the connector on its EVSE, else the first).
pub(crate) fn connector_index(s: &CsState, scope: Scope) -> Option<usize> {
    scope
        .evse
        .and_then(|e| s.connectors.iter().position(|c| c.evse_id == e))
        .or((!s.connectors.is_empty()).then_some(0))
}

pub(crate) fn connector_index_for_state(s: &CsState, scope: Scope) -> Option<usize> {
    scope
        .evse
        .and_then(|e| s.connectors.iter().position(|c| c.evse_id == e))
}

pub(crate) fn add_connector(s: &mut CsState, raw: &str) -> Option<i64> {
    let (evse, connector) = match raw.split_once('/') {
        Some((e, c)) => (parse_id(e).unwrap_or(1), parse_id(c)),
        None => (1, parse_id(raw)),
    };
    let connector = connector?;
    s.add_connector(evse, connector).then_some(connector)
}

pub(crate) fn seed_connector(s: &mut CsState, c: &ConnectorRef) {
    s.add_connector(c.evse.unwrap_or(1), c.connector);
}

pub(crate) fn connector_ref(s: &CsState, idx: usize) -> ConnectorRef {
    let c = &s.connectors[idx];
    ConnectorRef {
        evse: Some(c.evse_id),
        connector: c.connector_id,
    }
}

/// Map a connector state-table row (see `ConnectorState::rows`). Charge Limit (row 15) is
/// read-only.
pub(crate) fn conn_edit_field(row: usize) -> Option<EditField> {
    Some(match row {
        0 => EditField::EvseId,
        1 => EditField::ConnectorId,
        2 => EditField::Phases,
        3 => EditField::Voltage,
        4 => EditField::Current(0),
        5 => EditField::Current(1),
        6 => EditField::Current(2),
        7 => EditField::Power,
        8 => EditField::Frequency,
        9 => EditField::TotalEnergy,
        10 => EditField::SessionEnergy,
        11 => EditField::Soc,
        12 => EditField::Temperature,
        13 => EditField::Status,
        14 => EditField::Rfid,
        _ => return None,
    })
}

pub(crate) fn edit_kind(s: &CsState, scope: Scope, cs: bool, field: EditField) -> Option<EditKind> {
    let evse = if cs { None } else { scope.evse };
    let conn = evse
        .and_then(|e| s.connector_by_evse(e))
        .or_else(|| s.connectors.first());
    Some(match field {
        EditField::Phases => EditKind::Choice(choice(
            &PHASE_CHOICES,
            conn.map_or("", |c| c.phases.as_str()),
        )),
        EditField::Status => EditKind::Choice(choice(
            &STATUS_CHOICES,
            conn.map_or("", |c| c.status.as_str()),
        )),
        EditField::EvseId => EditKind::Number(number(conn.map_or(1.0, |c| c.evse_id as f64))),
        EditField::ConnectorId => {
            EditKind::Number(number(conn.map_or(0.0, |c| c.connector_id as f64)))
        }
        EditField::Voltage => EditKind::Number(number(conn.map_or(0.0, |c| c.voltage))),
        EditField::Current(i) => EditKind::Number(number(conn.map_or(0.0, |c| c.current[i]))),
        EditField::Power => EditKind::Number(number(conn.map_or(0.0, |c| c.power))),
        EditField::Frequency => EditKind::Number(number(conn.map_or(0.0, |c| c.frequency))),
        EditField::TotalEnergy => EditKind::Number(number(conn.map_or(0.0, |c| c.total_energy))),
        EditField::SessionEnergy => {
            EditKind::Number(number(conn.map_or(0.0, |c| c.session_energy)))
        }
        EditField::Soc => EditKind::Number(number(conn.map_or(0.0, |c| c.soc))),
        EditField::Temperature => EditKind::Number(number(conn.map_or(0.0, |c| c.temperature))),
        EditField::Rfid => EditKind::Text(text_input(conn.map_or("", |c| c.rfid.as_str()))),
        EditField::Model => EditKind::Text(text_input(&s.model)),
        EditField::Vendor => EditKind::Text(text_input(&s.vendor)),
        EditField::FirmwareVersion => EditKind::Text(text_input(&s.firmware_version)),
        EditField::SerialNumber => EditKind::Text(text_input(&s.serial_number)),
        // OC-R-104's meter/modem identity fields are 1.6-only; the row map never produces them
        // for 2.0.1/2.1.
        EditField::Iccid
        | EditField::Imsi
        | EditField::MeterSerialNumber
        | EditField::MeterType => {
            return None;
        }
    })
}

pub(crate) fn apply_edit(s: &mut CsState, edit: &EditOverlay, value: ResolvedEdit) {
    // Resolve the targeted connector for connector-level fields.
    let conn_idx = edit
        .scope
        .evse
        .and_then(|e| s.connectors.iter().position(|c| c.evse_id == e))
        .or((!s.connectors.is_empty()).then_some(0));
    match value {
        ResolvedEdit::Choice(value) => {
            if let Some(i) = conn_idx {
                let c = &mut s.connectors[i];
                match edit.field {
                    EditField::Phases => c.phases = value,
                    EditField::Status => c.status = value,
                    _ => {}
                }
            }
        }
        ResolvedEdit::Number(value) => {
            if let Some(i) = conn_idx {
                let c = &mut s.connectors[i];
                match edit.field {
                    EditField::EvseId => c.evse_id = value as i64,
                    EditField::ConnectorId => c.connector_id = value as i64,
                    EditField::Voltage => c.voltage = value,
                    EditField::Current(j) => c.current[j] = value,
                    EditField::Power => c.power = value,
                    EditField::Frequency => c.frequency = value,
                    EditField::TotalEnergy => c.total_energy = value,
                    EditField::SessionEnergy => c.session_energy = value,
                    EditField::Soc => c.soc = value,
                    EditField::Temperature => c.temperature = value,
                    _ => {}
                }
            }
        }
        ResolvedEdit::Text(value) => match edit.field {
            EditField::Rfid => {
                if let Some(i) = conn_idx {
                    s.connectors[i].rfid = value;
                }
            }
            EditField::Model => s.model = value,
            EditField::Vendor => s.vendor = value,
            EditField::FirmwareVersion => s.firmware_version = value,
            EditField::SerialNumber => s.serial_number = value,
            _ => {}
        },
    }
}

pub(crate) fn state_payload(s: &CsState, name: &str, scope: Scope) -> serde_json::Value {
    let conn = scope
        .evse
        .and_then(|e| s.connector_by_evse(e))
        .or_else(|| s.connectors.first());
    let evse = conn.map_or(1, |c| c.evse_id);
    let cid = conn.map_or(1, |c| c.connector_id);
    let rfid = conn.map(|c| c.rfid.clone()).unwrap_or_default();
    match name {
        "Authorize" => serde_json::json!({
            "idToken": { "idToken": rfid, "type": "Central" },
        }),
        "BootNotification" => serde_json::json!({
            "reason": "PowerUp",
            "chargingStation": {
                "model": s.model,
                "vendorName": s.vendor,
                "serialNumber": s.serial_number,
                "firmwareVersion": s.firmware_version,
            },
        }),
        "Heartbeat" => serde_json::json!({}),
        "MeterValues" => serde_json::json!({
            "evseId": evse,
            "meterValue": conn.map(super::v2_0_1::state::ConnectorState::meter_value_json).unwrap_or(serde_json::json!([])),
        }),
        "StatusNotification" => serde_json::json!({
            "timestamp": rfc3339_now(),
            "connectorStatus": conn.map(|c| c.status.clone()).unwrap_or_default(),
            "evseId": evse,
            "connectorId": cid,
        }),
        _ => serde_json::json!({}),
    }
}

/// Build a `TransactionEvent(Started)` for the connector resolved from `scope`, minting a tx id.
pub(crate) fn start_event(s: &mut CsState, scope: Scope) -> serde_json::Value {
    let idx = connector_index(s, scope);
    let Some(i) = idx else {
        return serde_json::json!({});
    };
    let tx = s.connectors[i].start_tx();
    let seq = s.connectors[i].next_seq();
    s.connectors[i].status = "Occupied".to_string();
    s.connectors[i].session_energy = 0.0;
    let c = &s.connectors[i];
    serde_json::json!({
        "eventType": "Started",
        "timestamp": rfc3339_now(),
        "triggerReason": "Authorized",
        "seqNo": seq,
        "transactionInfo": { "transactionId": tx },
        "idToken": { "idToken": c.rfid, "type": "Central" },
        "evse": { "id": c.evse_id, "connectorId": c.connector_id },
    })
}

/// Build a `TransactionEvent(Ended)` for the connector resolved from `scope`, or `None` if idle.
pub(crate) fn stop_event(s: &mut CsState, scope: Scope) -> Option<serde_json::Value> {
    let i = connector_index(s, scope)?;
    let tx = s.connectors[i].transaction_id.clone()?;
    let seq = s.connectors[i].next_seq();
    s.connectors[i].status = "Available".to_string();
    s.connectors[i].transaction_id = None;
    s.connectors[i].limit = None;
    s.connectors[i].tx_confirmed = false;
    let c = &s.connectors[i];
    Some(serde_json::json!({
        "eventType": "Ended",
        "timestamp": rfc3339_now(),
        "triggerReason": "StopAuthorized",
        "seqNo": seq,
        "transactionInfo": { "transactionId": tx },
        "idToken": { "idToken": c.rfid, "type": "Central" },
    }))
}

pub(crate) fn apply_post_send(
    s: &mut CsState,
    name: &str,
    scope: Scope,
    started_tx: Option<&str>,
    response: &serde_json::Value,
) {
    if name == "BootNotification" {
        s.heartbeat_interval_secs = boot_interval(response);
    }
    if let Some(tx_id) = started_tx
        && let Some(c) = scope.evse.and_then(|e| s.connector_mut_by_evse(e))
        && c.transaction_id.as_deref() == Some(tx_id)
    {
        c.tx_confirmed = true;
    }
}

pub(crate) fn rollback_tx(s: &mut CsState, scope: Scope, started_tx: Option<&str>) {
    if let Some(tx_id) = started_tx
        && let Some(c) = scope.evse.and_then(|e| s.connector_mut_by_evse(e))
        && c.transaction_id.as_deref() == Some(tx_id)
    {
        c.transaction_id = None;
        c.limit = None;
        c.tx_confirmed = false;
        c.status = "Available".to_string();
    }
}

pub(crate) fn active_meter_scopes(s: &CsState) -> Vec<Scope> {
    s.connectors
        .iter()
        .filter(|c| c.transaction_id.is_some() && c.tx_confirmed)
        .map(|c| Scope::evse(c.evse_id, None))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::ocpp::client::v2_0_1::state::ConnectorState;

    /// A CS with the given EVSE ids seeded via `add_connector` at connector ids 1..=n.
    fn state_with(evses: &[i64]) -> CsState {
        let mut s = CsState::default();
        s.connectors.clear();
        for (i, &e) in evses.iter().enumerate() {
            assert!(s.add_connector(e, i as i64 + 1));
        }
        s
    }

    #[test]
    /// OC-R-068 — clearing a per-purpose limit erases only the matching field; the rest persist.
    fn ut_clear_limit_by_purpose_matches_only_named_field() {
        let mut c = ConnectorState::new(1, 1);
        c.limit = Some(1.0);
        c.default_limit = Some(2.0);
        c.max_limit = Some(3.0);
        c.external_limit = Some(4.0);
        clear_limit_by_purpose(&mut c, Some("TxProfile"));
        assert_eq!(c.limit, None);
        assert_eq!(c.default_limit, Some(2.0));
        assert_eq!(c.max_limit, Some(3.0));
        assert_eq!(c.external_limit, Some(4.0));
    }

    #[test]
    fn ut_clear_limit_by_purpose_unknown_clears_nothing() {
        let mut c = ConnectorState::new(1, 1);
        c.limit = Some(1.0);
        c.default_limit = Some(2.0);
        clear_limit_by_purpose(&mut c, Some("Nonsense"));
        assert_eq!(c.limit, Some(1.0));
        assert_eq!(c.default_limit, Some(2.0));
    }

    #[test]
    fn ut_clear_limit_by_purpose_none_clears_all() {
        let mut c = ConnectorState::new(1, 1);
        c.limit = Some(1.0);
        c.default_limit = Some(2.0);
        c.max_limit = Some(3.0);
        c.external_limit = Some(4.0);
        clear_limit_by_purpose(&mut c, None);
        assert!(c.limit.is_none() && c.default_limit.is_none());
        assert!(c.max_limit.is_none() && c.external_limit.is_none());
    }

    #[test]
    fn ut_inbound_evse_reads_nested_and_top_level() {
        assert_eq!(
            inbound_evse(&serde_json::json!({ "evse": { "id": 7 } })),
            Some(7)
        );
        assert_eq!(inbound_evse(&serde_json::json!({ "evseId": 4 })), Some(4));
        assert_eq!(inbound_evse(&serde_json::json!({ "connectorId": 2 })), None);
    }

    #[test]
    fn ut_inbound_scope_keys_by_evse_else_cs() {
        assert_eq!(
            inbound_scope(&serde_json::json!({ "evseId": 3 })),
            Scope::evse(3, None)
        );
        assert_eq!(inbound_scope(&serde_json::json!({})), Scope::CS);
    }

    #[test]
    /// OC-R-063 — an addressed EVSE the station lacks is reported; id 0 and absent are always valid.
    fn ut_unknown_evse_flags_missing_only() {
        let s = state_with(&[1, 2]);
        assert_eq!(
            unknown_evse(&serde_json::json!({ "evseId": 9 }), &s),
            Some(9)
        );
        assert_eq!(unknown_evse(&serde_json::json!({ "evseId": 2 }), &s), None);
        assert_eq!(unknown_evse(&serde_json::json!({ "evseId": 0 }), &s), None);
        assert_eq!(unknown_evse(&serde_json::json!({}), &s), None);
    }

    #[test]
    /// OC-R-058 — the generic view sees each connector's own state through `ClientState`.
    fn ut_client_state_surface_over_connectors() {
        let mut s = state_with(&[1, 2]);
        assert_eq!(s.connector_count(), 2);
        assert_eq!(s.connector_position(2), Some(1));
        assert_eq!(s.connector_position(99), None);
        assert!(!s.cs_state_rows().is_empty());
        assert!(!s.conn_state_rows(0).is_empty());
        assert!(s.conn_state_rows(9).is_empty());
        assert!(!s.config().is_empty());
        s.remove_connector_at(0);
        assert_eq!(s.connector_count(), 1);
        s.clear_connectors();
        assert_eq!(s.connector_count(), 0);
    }

    #[test]
    /// OC-R-059 — the 2.x state-driven action set (built from state, no dialog).
    fn ut_state_driven_set_and_flags() {
        assert!(STATE_DRIVEN.contains(&"BootNotification"));
        assert!(STATE_DRIVEN.contains(&"Heartbeat"));
        assert!(!STATE_DRIVEN.contains(&"RequestStartTransaction"));
        assert!(has_tx_shortcuts());
        assert_eq!(config_title(), "Variables");
        assert_eq!(add_connector_placeholder(), "Add evse/connector");
    }

    #[test]
    fn ut_connector_index_resolves_scope_or_first() {
        let s = state_with(&[1, 5]);
        assert_eq!(connector_index(&s, Scope::evse(5, None)), Some(1));
        assert_eq!(connector_index(&s, Scope::CS), Some(0));
        assert_eq!(connector_index_for_state(&s, Scope::CS), None);
        assert_eq!(connector_index(&state_with(&[]), Scope::CS), None);
        assert_eq!(scope_of(&s, 1), Scope::evse(5, None));
    }

    #[test]
    fn ut_add_connector_parses_evse_slash_connector() {
        let mut s = state_with(&[]);
        assert_eq!(add_connector(&mut s, "2/3"), Some(3));
        assert_eq!(add_connector(&mut s, "7"), Some(7));
        assert_eq!(add_connector(&mut s, "bad"), None);
        assert_eq!(add_connector(&mut s, "2/3"), None); // duplicate connector id
        // Connectors sort by (evse, connector): (1,7) then (2,3).
        let r = connector_ref(&s, 1);
        assert_eq!((r.evse, r.connector), (Some(2), 3));
    }

    #[test]
    fn ut_seed_connector_defaults_evse_to_one() {
        let mut s = state_with(&[]);
        seed_connector(
            &mut s,
            &ConnectorRef {
                evse: None,
                connector: 4,
            },
        );
        assert_eq!(s.connectors[0].evse_id, 1);
        assert_eq!(s.connectors[0].connector_id, 4);
    }

    #[test]
    fn ut_conn_edit_field_maps_rows_and_bounds() {
        assert!(matches!(conn_edit_field(0), Some(EditField::EvseId)));
        assert!(matches!(conn_edit_field(4), Some(EditField::Current(0))));
        assert!(matches!(conn_edit_field(13), Some(EditField::Status)));
        assert!(matches!(conn_edit_field(14), Some(EditField::Rfid)));
        assert!(conn_edit_field(15).is_none()); // Charge Limit row is read-only
    }

    #[test]
    fn ut_edit_kind_picks_widget_per_field() {
        let s = state_with(&[1]);
        let sc = Scope::evse(1, None);
        assert!(matches!(
            edit_kind(&s, sc, false, EditField::Status),
            Some(EditKind::Choice(_))
        ));
        assert!(matches!(
            edit_kind(&s, sc, false, EditField::Voltage),
            Some(EditKind::Number(_))
        ));
        assert!(matches!(
            edit_kind(&s, sc, true, EditField::Model),
            Some(EditKind::Text(_))
        ));
    }

    #[test]
    /// OC-R-059 — state-driven request payloads are assembled entirely from observed state.
    fn ut_state_payload_built_from_state() {
        let s = state_with(&[1]);
        let sc = Scope::evse(1, None);
        assert_eq!(
            state_payload(&s, "BootNotification", sc)["chargingStation"]["model"],
            "Ferrowl-EVSE"
        );
        assert_eq!(state_payload(&s, "Heartbeat", sc), serde_json::json!({}));
        assert_eq!(state_payload(&s, "MeterValues", sc)["evseId"], 1);
        assert_eq!(
            state_payload(&s, "StatusNotification", sc)["connectorStatus"],
            "Available"
        );
        assert_eq!(
            state_payload(&s, "Authorize", sc)["idToken"]["idToken"],
            "DEADBEEF"
        );
        assert_eq!(state_payload(&s, "Unknown", sc), serde_json::json!({}));
    }

    #[test]
    /// OC-R-070 — a started transaction mints an id and puts the connector into a charging state.
    fn ut_start_event_mints_tx_and_occupies() {
        let mut s = state_with(&[1]);
        let ev = start_event(&mut s, Scope::evse(1, None));
        assert_eq!(ev["eventType"], "Started");
        assert!(ev["transactionInfo"]["transactionId"].is_string());
        assert_eq!(s.connectors[0].status, "Occupied");
        assert_eq!(s.connectors[0].session_energy, 0.0);
        assert!(s.connectors[0].transaction_id.is_some());
    }

    #[test]
    /// OC-R-072 — ending a transaction clears the tx-scoped limit only; the default limit persists.
    fn ut_stop_event_clears_tx_and_tx_scoped_limit() {
        let mut s = state_with(&[1]);
        start_event(&mut s, Scope::evse(1, None));
        s.connectors[0].limit = Some(16.0);
        s.connectors[0].default_limit = Some(32.0);
        let ev = stop_event(&mut s, Scope::evse(1, None)).unwrap();
        assert_eq!(ev["eventType"], "Ended");
        assert_eq!(s.connectors[0].status, "Available");
        assert_eq!(s.connectors[0].transaction_id, None);
        assert_eq!(s.connectors[0].limit, None);
        assert_eq!(s.connectors[0].default_limit, Some(32.0));
    }

    #[test]
    fn ut_stop_event_none_when_idle() {
        let mut s = state_with(&[1]);
        assert!(stop_event(&mut s, Scope::evse(1, None)).is_none());
    }

    #[test]
    /// OC-R-060 — the heartbeat cadence is taken from the BootNotification response's interval.
    fn ut_apply_post_send_boot_sets_heartbeat_interval() {
        let mut s = state_with(&[1]);
        apply_post_send(
            &mut s,
            "BootNotification",
            Scope::CS,
            None,
            &serde_json::json!({ "interval": 90 }),
        );
        assert_eq!(s.heartbeat_interval_secs, Some(90));
    }

    #[test]
    fn ut_apply_post_send_confirms_started_tx() {
        let mut s = state_with(&[1]);
        let tx = s.connectors[0].start_tx();
        apply_post_send(
            &mut s,
            "TransactionEvent",
            Scope::evse(1, None),
            Some(&tx),
            &serde_json::json!({}),
        );
        assert!(s.connectors[0].tx_confirmed);
    }

    #[test]
    /// OC-R-070 — a failed start rolls the connector back to available with no transaction.
    fn ut_rollback_tx_reverts_started_connector() {
        let mut s = state_with(&[1]);
        let tx = s.connectors[0].start_tx();
        s.connectors[0].limit = Some(16.0);
        s.connectors[0].status = "Occupied".to_string();
        rollback_tx(&mut s, Scope::evse(1, None), Some(&tx));
        assert_eq!(s.connectors[0].transaction_id, None);
        assert_eq!(s.connectors[0].limit, None);
        assert!(!s.connectors[0].tx_confirmed);
        assert_eq!(s.connectors[0].status, "Available");
    }

    #[test]
    /// OC-R-061 — auto MeterValues target only connectors with a live, confirmed transaction.
    fn ut_active_meter_scopes_only_confirmed_tx() {
        let mut s = state_with(&[1, 2]);
        s.connectors[0].start_tx();
        s.connectors[0].tx_confirmed = true;
        s.connectors[1].start_tx(); // unconfirmed
        assert_eq!(active_meter_scopes(&s), vec![Scope::evse(1, None)]);
    }
}
