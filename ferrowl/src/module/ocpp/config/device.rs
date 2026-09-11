//! OCPP device-type config: the per-device-type settings for an OCPP charging station — the
//! OCPP version it speaks, its role (charging station / management system), the reply timeout,
//! and its Lua simulation scripts. One file = one device type (no ip/port — those are the
//! per-instance endpoint, set via the setup dialog / session like Modbus).
//!
//! The OCPP version lives here (not in the session) because the Lua scripts call version-specific
//! `C_OCPP:<Action>` methods, so a device file is version-locked.

use serde::{Deserialize, Serialize};

pub use crate::config::script::ScriptDef;

use super::session::{OcppRole, OcppSpec, OcppVersion};

/// The two-role TLS container for an OCPP device config (OC-R-126), the same shape as
/// `ferrowl_modbus::tcp::ModbusTlsConfig` one level deeper: serialized `[security.tls.server]`/
/// `[security.tls.client]`. Each policy independently defaults to its own `None` variant, so an
/// absent `tls` block, an empty one, and one whose two policies are both `mode = "none"` denote
/// the same state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct OcppTlsConfig {
    pub server: ferrowl_util::tls::ServerTlsPolicy,
    pub client: ferrowl_util::tls::ClientTlsPolicy,
}

impl OcppTlsConfig {
    pub fn is_none(&self) -> bool {
        matches!(self.server, ferrowl_util::tls::ServerTlsPolicy::None {})
            && matches!(self.client, ferrowl_util::tls::ClientTlsPolicy::None {})
    }
}

/// Optional websocket transport security for an OCPP instance: HTTP Basic Auth (Security Profile
/// one) and TLS/mTLS (Security Profiles two and three, held per role in `tls`, OC-R-126). A role
/// irrelevant to the instance's [`OcppRole`] is simply inert (same convention as the role-specific
/// fields elsewhere in [`OcppDeviceConfig`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct OcppSecurityConfig {
    /// Basic Auth username. Client role: sent on connect. Server role: required to accept a
    /// connection (together with `password`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Basic Auth password. Never logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// The two-role TLS container (OC-R-126). Always present on the wire, both policies
    /// defaulting to `None`, so a role toggle doesn't lose the other role's settings.
    #[serde(default, skip_serializing_if = "OcppTlsConfig::is_none")]
    pub tls: OcppTlsConfig,
}

impl OcppSecurityConfig {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Basic Auth credentials, if both `username` and `password` are set.
    pub fn basic_auth(&self) -> Option<ferrowl_ocpp::BasicAuth> {
        match (&self.username, &self.password) {
            (Some(username), Some(password)) => Some(ferrowl_ocpp::BasicAuth {
                username: username.clone(),
                password: password.clone(),
            }),
            _ => None,
        }
    }

    /// CS-side TLS policy (OC-R-126): a CS decides whether TLS is configured by matching its own
    /// role's policy variant, never by comparing the whole security block against a baseline.
    pub fn cs_tls(&self) -> ferrowl_util::tls::ClientTlsPolicy {
        self.tls.client.clone()
    }

    /// CSMS-side TLS policy (OC-R-126).
    pub fn csms_tls(&self) -> ferrowl_util::tls::ServerTlsPolicy {
        self.tls.server.clone()
    }
}

/// A persisted connector entry for a charging-station (client) device type. `evse` is `None` for
/// OCPP 1.6 (connector-only) and `Some` for 2.0.1; `connector` is the connector id. The CS-level
/// entry is implicit (always present in the view) and is not stored here. Maps to a runtime
/// `Scope` when the view is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse: Option<i64>,
    pub connector: i64,
}

/// A persisted per-connector CSMS RFID accept-list (server role). The connector is identified the
/// same way as [`ConnectorRef`] (`evse` is `None` for 1.6, `Some` for 2.0.1); `rfids` are the tags
/// accepted for that connector *in addition to* the inherited charge-point-wide list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRfids {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<i64>,
    pub rfids: Vec<String>,
}

/// A persisted configuration key for a charging-station (client) device type: a name/value pair and
/// its read-only flag, seeded into the client's config store (GetConfiguration / GetVariables) on
/// load and written by `:wd`. Server (CSMS) config is per-connected-station and transient, so it is
/// never persisted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigKeyDef {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub readonly: bool,
}

/// An OCPP device-type configuration file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct OcppDeviceConfig {
    /// Ferrowl version that wrote this file, stamped on save. Enables future compatibility shims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// OCPP protocol version this device type speaks.
    #[serde(default)]
    pub ocpp_version: OcppVersion,
    /// Whether the module acts as a charging station (client) or management system (server).
    #[serde(default)]
    pub role: OcppRole,
    /// Awaited-reply timeout (ms); `None` uses the crate default (30_000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Automatically reconnect (with backoff) instead of ending the module task on failure:
    /// client redial on a lost or refused connection (OC-R-048), or server listener bind retry
    /// (OC-R-083). `None` falls back to the default (on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect: Option<bool>,
    /// Lua simulation scripts (run every ~100ms while enabled; client role only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptDef>,
    /// Script sim cycle interval in seconds — the period `refresh_all` is called on the sim
    /// thread. Older device config files without this field load as the default (1.0s).
    #[serde(default = "default_script_interval")]
    pub script_interval: f64,
    /// Persistent log-file base set via `:log <file>`; `None` disables file logging. The actual
    /// file is `<stem>.<tab-name>.<ext>` next to this path (see `module_log_path`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
    /// Charge-point-wide CSMS RFID accept-list (server role): id tags accepted for Authorize /
    /// transaction starts, inherited by every connector. Empty (together with all connector lists)
    /// = accept every tag (the default-accept behaviour). Ignored for the client role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rfids: Vec<String>,
    /// Per-connector CSMS RFID accept-lists (server role), each unioned with [`rfids`](Self::rfids)
    /// when gating that connector's transaction starts. Ignored for the client role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_rfids: Vec<ConnectorRfids>,
    /// Connector entries for the charging-station (client) view, seeded into its connector table on
    /// load and written by `:wd`. Empty = CS-level only. Ignored for the server role (connectors
    /// there are discovered from connected stations).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connectors: Vec<ConnectorRef>,
    /// Persisted configuration keys for the charging-station (client) view, seeded into its config
    /// store on load and written by `:wd`. Empty = use the built-in defaults. Ignored for the server
    /// role (CSMS config is per-connected-station and transient).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config: Vec<ConfigKeyDef>,
    /// Extra headers sent on the WebSocket upgrade request in addition to the client's own
    /// (OC-R-117). Client-only device config field: never exposed via `--ocpp` (OC-R-119).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_headers: Vec<ferrowl_ocpp::HeaderDef>,
    /// CS boot identity model, seeded into state on load and written by `:wd` (OC-R-103). Unset =
    /// keep the built-in default. Ignored for the server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// CS boot identity vendor (OC-R-103). Unset = keep the built-in default. Ignored for the
    /// server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// CS boot identity firmware version (OC-R-103). Unset = keep the built-in default. Ignored
    /// for the server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    /// CS boot identity serial number (OC-R-103). Unset = keep the built-in default. Ignored for
    /// the server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// SIM ICCID (OC-R-104). Unset = keep the built-in default (empty). OCPP 1.6 only; ignored
    /// for 2.0.1/2.1 and the server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iccid: Option<String>,
    /// SIM IMSI (OC-R-104). OCPP 1.6 only; ignored for 2.0.1/2.1 and the server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imsi: Option<String>,
    /// Installed meter's serial number (OC-R-104). OCPP 1.6 only; ignored for 2.0.1/2.1 and the
    /// server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_serial_number: Option<String>,
    /// Installed meter's type/model (OC-R-104). OCPP 1.6 only; ignored for 2.0.1/2.1 and the
    /// server role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_type: Option<String>,
    /// Websocket transport security: Basic Auth and/or TLS/mTLS. Default (all `None`/`false`) is
    /// the pre-existing plain `ws://` behaviour.
    #[serde(default, skip_serializing_if = "OcppSecurityConfig::is_empty")]
    pub security: OcppSecurityConfig,
}

fn default_script_interval() -> f64 {
    1.0
}

/// Floor for the script sim cycle: below this, a Lua script would busy-loop the sim thread with
/// no benefit (register I/O and Lua execution themselves take real time). Well under the old
/// fixed 1s device-poll-derived floor this replaces, so genuinely fast cycles are still possible.
const MIN_SCRIPT_INTERVAL_SECS: f64 = 0.05;

impl OcppDeviceConfig {
    /// Assemble a device config from a runtime spec, carrying the given scripts. Used when a
    /// setup/edit dialog supplies version/role/timeout and the scripts are preserved separately.
    pub fn from_spec(spec: &OcppSpec, scripts: Vec<ScriptDef>) -> Self {
        Self {
            version: None,
            ocpp_version: spec.version,
            role: spec.role,
            timeout_ms: spec.timeout_ms,
            reconnect: spec.reconnect,
            scripts,
            script_interval: default_script_interval(),
            log_file: None,
            rfids: Vec::new(),
            connector_rfids: Vec::new(),
            connectors: Vec::new(),
            config: Vec::new(),
            extra_headers: Vec::new(),
            model: None,
            vendor: None,
            firmware_version: None,
            serial_number: None,
            iccid: None,
            imsi: None,
            meter_serial_number: None,
            meter_type: None,
            security: spec.security.clone(),
        }
    }

    /// The script sim cycle interval as a `Duration`; see
    /// [`crate::config::sanitize_interval_secs`] for the sanitization rule.
    pub fn script_interval_duration(&self) -> std::time::Duration {
        crate::config::sanitize_interval_secs(
            self.script_interval,
            default_script_interval(),
            MIN_SCRIPT_INTERVAL_SECS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_test_support::reserve_temp_dir;
    use ferrowl_util::convert::{Converter, FileType};

    #[test]
    /// SC-R-022 — a script entry with no `enabled` flag defaults to enabled.
    fn ut_script_enabled_defaults_true() {
        // A file entry without an `enabled` flag deserializes as active.
        let s: ScriptDef = serde_json::from_str(r#"{"name":"a","code":"x = 1"}"#).unwrap();
        assert!(s.enabled);
    }

    #[test]
    /// CS-R-004 — an OCPP device config round-trips through TOML and JSON.
    fn ut_device_config_roundtrip() {
        let cfg = OcppDeviceConfig {
            version: Some("0.1.0".into()),
            ocpp_version: OcppVersion::V2_0_1,
            role: OcppRole::Client,
            timeout_ms: Some(5000),
            reconnect: Some(false),
            scripts: vec![ScriptDef {
                name: "boot".into(),
                code: "C_OCPP:Set(\"Power\", 11000)".into(),
                enabled: false,
            }],
            script_interval: 2.5,
            log_file: Some("/tmp/ferrowl.log".into()),
            rfids: vec!["DEADBEEF".into(), "CAFE1234".into()],
            connector_rfids: vec![ConnectorRfids {
                evse: Some(1),
                connector: Some(2),
                rfids: vec!["CONN2TAG".into()],
            }],
            connectors: vec![
                ConnectorRef {
                    evse: None,
                    connector: 1,
                },
                ConnectorRef {
                    evse: Some(1),
                    connector: 2,
                },
            ],
            config: vec![
                ConfigKeyDef {
                    key: "HeartbeatInterval".into(),
                    value: "30".into(),
                    readonly: false,
                },
                ConfigKeyDef {
                    key: "NumberOfConnectors".into(),
                    value: "2".into(),
                    readonly: true,
                },
            ],
            extra_headers: Vec::new(),
            model: Some("Ferrowl-EVSE-Pro".into()),
            vendor: Some("Acme".into()),
            firmware_version: Some("2.3.1".into()),
            serial_number: Some("SN-0042".into()),
            iccid: Some("8912345678901234567".into()),
            imsi: Some("290123456789012".into()),
            meter_serial_number: Some("MTR-0042".into()),
            meter_type: Some("MT-X".into()),
            security: OcppSecurityConfig {
                username: Some("cp001".into()),
                password: Some("s3cret".into()),
                tls: OcppTlsConfig {
                    client: ferrowl_util::tls::ClientTlsPolicy::Tls {
                        verification: ferrowl_util::tls::CertVerification::RootStore {
                            extra_ca_files: vec!["/tmp/ca.pem".into()],
                        },
                    },
                    ..Default::default()
                },
            },
        };
        let dir = reserve_temp_dir("ferrowl_ocpp_device");
        for (ty, ext) in [(FileType::Toml, "toml"), (FileType::Json, "json")] {
            let path = dir.join(format!("device.{ext}"));
            let path = path.to_str().unwrap();
            Converter::save(&cfg, path, ty).expect("save");
            let back: OcppDeviceConfig = Converter::load(path, ty).expect("load");
            assert_eq!(cfg, back);
        }
    }

    #[test]
    /// CS-R-023 — an OCPP device config predating the security section loads with its default.
    fn ut_device_config_without_security_section_still_parses() {
        // Pre-existing config files (written before Security Profiles were added) have no
        // `security` table/key at all; `#[serde(default)]` must fill it in as the all-`None`
        // default rather than failing to parse.
        let json = serde_json::json!({
            "ocpp_version": "1.6",
            "role": "client",
            "timeout_ms": 5000,
        });
        let cfg: OcppDeviceConfig = serde_json::from_value(json).expect("old-style config parses");
        assert_eq!(cfg.security, OcppSecurityConfig::default());
        assert!(cfg.security.basic_auth().is_none());
        assert!(matches!(
            cfg.security.cs_tls(),
            ferrowl_util::tls::ClientTlsPolicy::None {}
        ));
        assert!(matches!(
            cfg.security.csms_tls(),
            ferrowl_util::tls::ServerTlsPolicy::None {}
        ));
    }

    #[test]
    /// OC-R-157 — `OcppSecurityConfig`'s TLS container defaults to both policies `None`, and an
    /// absent `[security.tls]` block is exactly that state.
    fn ut_ocpp_security_config_defaults() {
        let cfg = OcppSecurityConfig::default();
        assert!(cfg.tls.is_none());
    }

    #[test]
    /// MB-R-168, OC-R-126 — the two-role container serializes at `[security.tls.server]`/
    /// `[security.tls.client]` and round-trips both roles together through TOML.
    fn ut_security_tls_block_roundtrips_both_roles() {
        let cfg = OcppSecurityConfig {
            username: Some("cs001".into()),
            password: Some("hunter2".into()),
            tls: OcppTlsConfig {
                server: ferrowl_util::tls::ServerTlsPolicy::Mutual {
                    identity: ferrowl_util::tls::CertSource::Files {
                        cert_file: "csms.crt".into(),
                        key_file: "csms.key".into(),
                    },
                    verification: ferrowl_util::tls::CertVerification::CaFiles {
                        ca_files: vec!["fleet-ca.pem".into()],
                    },
                },
                client: ferrowl_util::tls::ClientTlsPolicy::Tls {
                    verification: ferrowl_util::tls::CertVerification::RootStore {
                        extra_ca_files: vec!["private-ca.pem".into()],
                    },
                },
            },
        };
        let dir = reserve_temp_dir("ferrowl_ocpp_device");
        let path = dir.join("security_both_roles.toml");
        let path = path.to_str().unwrap();
        Converter::save(&cfg, path, FileType::Toml).expect("save");
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(
            raw.contains("[tls.server]") && raw.contains("[tls.client]"),
            "expected [security.tls.server]/[security.tls.client] key paths, got:\n{raw}"
        );
        let back: OcppSecurityConfig = Converter::load(path, FileType::Toml).expect("load");
        assert_eq!(cfg, back);
    }

    #[test]
    /// OC-R-158 — a CS decides whether TLS is configured by matching only its own role's
    /// policy variant: a non-`None` server policy does not affect `cs_tls()`, and vice versa.
    fn ut_cs_tls_reads_only_the_client_half() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ferrowl_util::tls::ServerTlsPolicy::Mutual {
                    identity: ferrowl_util::tls::CertSource::SelfSigned {},
                    verification: ferrowl_util::tls::CertVerification::CaFiles {
                        ca_files: vec!["fleet-ca.pem".into()],
                    },
                },
                client: ferrowl_util::tls::ClientTlsPolicy::None {},
            },
            ..Default::default()
        };
        assert!(matches!(
            cfg.cs_tls(),
            ferrowl_util::tls::ClientTlsPolicy::None {}
        ));
        assert!(!matches!(
            cfg.csms_tls(),
            ferrowl_util::tls::ServerTlsPolicy::None {}
        ));
    }

    #[test]
    /// OC-R-112 — `cert_file` set alone (no `key_file`) inside a `[tls.server.identity]` block
    /// fails to deserialize an `OcppSecurityConfig`: `CertSource::Files` requires both fields.
    fn ut_device_config_cert_file_alone_fails_to_load() {
        let json = serde_json::json!({
            "tls": {
                "server": {
                    "mode": "tls",
                    "identity": {"source": "files", "cert_file": "s.crt"}
                }
            }
        });
        let result: Result<OcppSecurityConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    /// OC-R-157 — `csms_tls()` returns `ServerTlsPolicy::None {}` when the container's server
    /// policy is `None`.
    fn ut_csms_tls_none_by_default() {
        let cfg = OcppSecurityConfig::default();
        assert!(matches!(
            cfg.csms_tls(),
            ferrowl_util::tls::ServerTlsPolicy::None {}
        ));
    }

    // An old-format device config file (predating `script_interval`) must still load, with
    // `script_interval` defaulting to 1.0.
    #[test]
    /// SC-R-016 — an absent script_interval resolves to the 1.0s default.
    fn ut_device_config_loads_without_script_interval_field() {
        let json = serde_json::json!({
            "ocpp_version": "1.6",
            "role": "client",
        });
        let cfg: OcppDeviceConfig = serde_json::from_value(json).expect("old-style config parses");
        assert_eq!(cfg.script_interval, 1.0);
    }

    // A hand-edited `script_interval` that is NaN, negative, or zero must fall back to the
    // 1.0s default instead of panicking or busy-waiting; a valid value converts as-is.
    #[test]
    /// SC-R-016 — a non-finite or non-positive script_interval falls back to the 1.0s default.
    fn ut_device_config_script_interval_duration_sanitized() {
        let mut cfg = OcppDeviceConfig::default();
        assert_eq!(
            cfg.script_interval_duration(),
            std::time::Duration::from_secs(1)
        );
        cfg.script_interval = 0.25;
        assert_eq!(
            cfg.script_interval_duration(),
            std::time::Duration::from_millis(250)
        );
        for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
            cfg.script_interval = bad;
            assert_eq!(
                cfg.script_interval_duration(),
                std::time::Duration::from_secs(1)
            );
        }
    }

    #[test]
    /// SC-R-045 — a per-module script_interval is floored to 0.05s.
    fn ut_device_config_script_interval_duration_floored() {
        let cfg = OcppDeviceConfig {
            script_interval: 0.0001,
            ..Default::default()
        };
        assert_eq!(
            cfg.script_interval_duration(),
            std::time::Duration::from_millis(50)
        );
    }

    #[test]
    /// CS-R-004 — new security fields round-trip through TOML and JSON.
    fn ut_security_config_new_fields_round_trip() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ferrowl_util::tls::ServerTlsPolicy::Tls {
                    identity: ferrowl_util::tls::CertSource::SelfSigned {},
                },
                client: ferrowl_util::tls::ClientTlsPolicy::Tls {
                    verification: ferrowl_util::tls::CertVerification::Skip {},
                },
            },
            ..Default::default()
        };
        let dir = reserve_temp_dir("ferrowl_ocpp_device");
        for (ty, ext) in [(FileType::Toml, "toml"), (FileType::Json, "json")] {
            let path = dir.join(format!("security.{ext}"));
            let path = path.to_str().unwrap();
            Converter::save(&cfg, path, ty).expect("save");
            let back: OcppSecurityConfig = Converter::load(path, ty).expect("load");
            assert_eq!(cfg, back);
        }
    }

    #[test]
    /// OC-R-096 — a wss CSMS with no server cert/key files uses `self_signed` for its TLS material.
    fn ut_csms_tls_self_signed_without_cert_files() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ferrowl_util::tls::ServerTlsPolicy::Tls {
                    identity: ferrowl_util::tls::CertSource::SelfSigned {},
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            cfg.csms_tls(),
            ferrowl_util::tls::ServerTlsPolicy::Tls {
                identity: ferrowl_util::tls::CertSource::SelfSigned {},
            }
        ));
    }

    #[test]
    /// OC-R-096 — `CertSource::Files` is carried through for a wss CSMS unchanged.
    fn ut_csms_tls_explicit_maps_to_files() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ferrowl_util::tls::ServerTlsPolicy::Tls {
                    identity: ferrowl_util::tls::CertSource::Files {
                        cert_file: "s.crt".into(),
                        key_file: "s.key".into(),
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            cfg.csms_tls(),
            ferrowl_util::tls::ServerTlsPolicy::Tls {
                identity: ferrowl_util::tls::CertSource::Files { .. },
            }
        ));
    }

    #[test]
    /// OC-R-036 — a CS TLS configuration carries `CertVerification::Skip`.
    fn ut_cs_tls_carries_skip_verify() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                client: ferrowl_util::tls::ClientTlsPolicy::Tls {
                    verification: ferrowl_util::tls::CertVerification::Skip {},
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            cfg.cs_tls(),
            ferrowl_util::tls::ClientTlsPolicy::Tls {
                verification: ferrowl_util::tls::CertVerification::Skip {},
            }
        );
    }

    #[test]
    /// OC-R-081 — the device config carries the module's Lua scripts (not the session entry).
    fn ut_from_spec_carries_scripts() {
        let spec = OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Server,
            protocol: super::super::session::OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: Some(1000),
            reconnect: Some(false),
            security: OcppSecurityConfig::default(),
        };
        let scripts = vec![ScriptDef {
            name: "s".into(),
            code: "".into(),
            enabled: true,
        }];
        let cfg = OcppDeviceConfig::from_spec(&spec, scripts.clone());
        assert_eq!(cfg.ocpp_version, OcppVersion::V1_6);
        assert_eq!(cfg.role, OcppRole::Server);
        assert_eq!(cfg.timeout_ms, Some(1000));
        assert_eq!(cfg.reconnect, Some(false));
        assert_eq!(cfg.scripts, scripts);
        assert_eq!(cfg.version, None);
    }

    #[test]
    /// OC-R-107 — a device config predating the `reconnect` field loads with its default (unset,
    /// falling back to reconnect-enabled at the point of use), mirroring Modbus's own
    /// `DeviceConfig::reconnect` compatibility shim.
    fn ut_device_config_loads_without_reconnect_field() {
        let json = serde_json::json!({
            "ocpp_version": "1.6",
            "role": "client",
        });
        let cfg: OcppDeviceConfig = serde_json::from_value(json).expect("old-style config parses");
        assert_eq!(cfg.reconnect, None);
    }

    #[test]
    /// OC-R-117 — extra_headers round-trips through TOML and JSON.
    fn ut_device_config_extra_headers_round_trip() {
        let cfg = OcppDeviceConfig {
            extra_headers: vec![ferrowl_ocpp::HeaderDef::new("X-Tenant", "acme-1").unwrap()],
            ..Default::default()
        };
        let dir = reserve_temp_dir("ferrowl_ocpp_device");
        for (ty, ext) in [(FileType::Toml, "toml"), (FileType::Json, "json")] {
            let path = dir.join(format!("extra_headers.{ext}"));
            let path = path.to_str().unwrap();
            Converter::save(&cfg, path, ty).expect("save");
            let back: OcppDeviceConfig = Converter::load(path, ty).expect("load");
            assert_eq!(cfg, back);
        }
    }

    #[test]
    /// OC-R-117 — a device config predating extra_headers loads with an empty list.
    fn ut_device_config_loads_without_extra_headers_field() {
        let json = serde_json::json!({
            "ocpp_version": "1.6",
            "role": "client",
        });
        let cfg: OcppDeviceConfig = serde_json::from_value(json).expect("old-style config parses");
        assert_eq!(cfg.extra_headers, Vec::new());
    }

    /// CS-R-055 — a retired flat field beside the `security` table's defined members
    /// (`username`, `password`, `tls`) fails the load rather than being silently ignored.
    #[test]
    fn ut_security_table_rejects_unknown_field() {
        let json = serde_json::json!({
            "username": "cp001",
            "password": "s3cret",
            "require_client_cert": true,
        });
        let err = serde_json::from_value::<OcppSecurityConfig>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("unknown field `require_client_cert`"),
            "got: {err}"
        );
    }

    /// CS-R-071, OC-R-156 — `username`/`password` remain defined members of `security` and are unaffected
    /// by the strictness that governs the rest of the table.
    #[test]
    fn ut_security_table_accepts_username_password_beside_tls() {
        let json = serde_json::json!({
            "username": "cp001",
            "password": "s3cret",
            "tls": {
                "server": {"mode": "none"},
                "client": {"mode": "none"},
            },
        });
        let cfg: OcppSecurityConfig = serde_json::from_value(json).expect("defined fields load");
        assert_eq!(cfg.username, Some("cp001".into()));
        assert_eq!(cfg.password, Some("s3cret".into()));
    }
}
