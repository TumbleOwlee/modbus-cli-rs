//! Shared OCPP 2.x CSMS (server) bindings, used by both 2.0.1 and 2.1.
//!
//! Like the client side, 2.1 reuses the 2.0.1 CSMS behaviour. The inbound handler builds replies as
//! JSON (decoded into the version's typed `Response` via `decode_result`/`default_response`), and
//! the `ServerVersion` glue is pure JSON/string logic, so both live here once as plain free functions
//! and each version's `impl` (in `v2_0_1/mod.rs` / `v2_1/mod.rs`, `v2_0_1/handler.rs` /
//! `v2_1/handler.rs`) delegates to them — only the inbound handler type and the action-spec module
//! actually differ per version, and those two seams stay in each version's own file. Both versions
//! share the [`crate::module::ocpp::server::v2_0_1::state`] types.
//!
//! The inbound (CS→CSMS) handler itself (`CsmsHandler`) is *not* shared here: it is typed over the
//! version's own `rust_ocpp`-derived `Action`/`Response` wire enums, so its concrete type differs
//! per version even though the decision logic is identical. It is defined once per version in
//! `v2_0_1/handler.rs` and `v2_1/handler.rs`; only the version-independent helpers it calls
//! (`craft_response`, `cs_status`, `evse_status`) live here.

use serde_json::{Value, json};

use crate::module::ocpp::client::backend::rfc3339_now;
use crate::module::ocpp::scope::{Scope, evse_id};
use crate::module::ocpp::server::backend::{RfidLists, cs_authorized, scope_authorized};

/// `"Accepted"` / `"Invalid"` for a CS-wide check (Authorize carries no EVSE).
pub(crate) fn cs_status(rfids: &RfidLists, id_token: &str) -> &'static str {
    if cs_authorized(rfids, id_token) {
        "Accepted"
    } else {
        "Invalid"
    }
}

/// `"Accepted"` / `"Invalid"` for a tag on a specific EVSE (its list ∪ the CS list).
pub(crate) fn evse_status(rfids: &RfidLists, evse_id: i64, id_token: &str) -> &'static str {
    if scope_authorized(rfids, Scope::evse(evse_id, None), id_token) {
        "Accepted"
    } else {
        "Invalid"
    }
}

/// The JSON payload a CSMS reply should carry for `name`, or `None` when the version's
/// `default_response(name)` (a plain default-accepted/empty reply) suffices. Pure JSON logic, no
/// dependency on either version's typed `Action`/`Response` wire enums.
pub(crate) fn craft_response(name: &str, request: &Value, rfids: &RfidLists) -> Option<Value> {
    match name {
        "BootNotification" => Some(json!({
            "currentTime": rfc3339_now(),
            "interval": 300,
            "status": "Accepted",
        })),
        "Heartbeat" => Some(json!({ "currentTime": rfc3339_now() })),
        // Authorize carries no EVSE, so it is checked against the CS list unioned with every
        // connector list.
        "Authorize" => {
            let tag = request["idToken"]["idToken"].as_str().unwrap_or_default();
            Some(json!({ "idTokenInfo": { "status": cs_status(rfids, tag) } }))
        }
        // A TransactionEvent carrying an idToken (a transaction start) names an EVSE, so it is
        // gated by that EVSE's list ∪ the CS list; the reply echoes the decision.
        "TransactionEvent" if request["idToken"].is_object() => {
            let tag = request["idToken"]["idToken"].as_str().unwrap_or_default();
            let evse = request["evse"]["id"].as_i64().unwrap_or_default();
            Some(json!({ "idTokenInfo": { "status": evse_status(rfids, evse, tag) } }))
        }
        _ => None,
    }
}

// Shared `ServerVersion` body (both 2.0.1 and 2.1's `impl` blocks delegate to these).

/// 2.0.1 connectors are listed and addressed by EVSE id only; a nested/top-level `connectorId` is
/// ignored for bucketing (connector kept `None`).
pub(crate) fn inbound_connector(_name: &str, request: &serde_json::Value) -> Scope {
    match evse_id(request) {
        Some(e) if e >= 1 => Scope::evse(e, None),
        _ => Scope::CS,
    }
}

/// A TransactionEvent(Ended) may omit `evse`, in which case it buckets to CS scope; route it to the
/// connector holding this transactionId instead.
pub(crate) fn stop_tx_id(name: &str, request: &serde_json::Value) -> Option<String> {
    (name == "TransactionEvent" && request["eventType"].as_str() == Some("Ended"))
        .then(|| {
            request["transactionInfo"]["transactionId"]
                .as_str()
                .map(str::to_owned)
        })
        .flatten()
}

pub(crate) fn inject_scope(payload: &mut serde_json::Value, scope: Scope) {
    if let (Some(e), Some(obj)) = (scope.evse, payload.as_object_mut()) {
        // Set the EVSE id when absent or still the `0` default the encoded request struct
        // carries; a genuine non-zero value (and later user overrides) win.
        let cur = obj.get("evseId").and_then(serde_json::Value::as_i64);
        if cur.is_none() || cur == Some(0) {
            obj.insert("evseId".into(), serde_json::json!(e));
        }
    }
}

/// 2.0.1 connectors are addressed by EVSE id (the connector dimension is always `None`).
pub(crate) fn lua_connector_id(scope: Scope) -> Option<i64> {
    scope.evse
}

/// GetVariables/SetVariables keys are `Component/Variable`; show a Component column.
pub(crate) fn config_has_component() -> bool {
    true
}

pub(crate) fn config_action() -> &'static str {
    "GetVariables"
}

/// GetVariables needs an explicit component + variable. Accept "Component/Variable"; a bare key
/// uses it for both names.
pub(crate) fn config_request(key: &str) -> serde_json::Value {
    let (component, variable) = key.split_once('/').unwrap_or((key, key));
    serde_json::json!({
        "getVariableData": [{
            "component": { "name": component },
            "variable": { "name": variable },
        }]
    })
}

pub(crate) fn parse_config(response: &serde_json::Value) -> Vec<(String, String, bool)> {
    let mut rows = Vec::new();
    let Some(results) = response["getVariableResult"].as_array() else {
        return rows;
    };
    for r in results {
        let component = r["component"]["name"].as_str().unwrap_or_default();
        let variable = r["variable"]["name"].as_str().unwrap_or_default();
        let status = r["attributeStatus"].as_str().unwrap_or("Unknown");
        let value = if status == "Accepted" {
            r["attributeValue"].as_str().unwrap_or_default().to_string()
        } else {
            format!("<{status}>")
        };
        // 2.0.1 mutability is per-attribute (not a simple bool); treat as writable.
        rows.push((format!("{component}/{variable}"), value, false));
    }
    rows
}

pub(crate) fn set_action() -> &'static str {
    "SetVariables"
}

pub(crate) fn set_request(key: &str, value: &str) -> serde_json::Value {
    let (component, variable) = key.split_once('/').unwrap_or((key, key));
    serde_json::json!({
        "setVariableData": [{
            "attributeValue": value,
            "component": { "name": component },
            "variable": { "name": variable },
        }]
    })
}
