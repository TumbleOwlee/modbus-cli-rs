//! Device and session configuration loading (TOML/JSON).

pub mod script;

pub mod device {
    pub use crate::module::modbus::config::device::*;
}
pub mod session {
    pub use crate::module::modbus::config::session::*;
}
pub mod ocpp {
    pub use crate::module::ocpp::config::device::*;
    pub use crate::module::ocpp::config::session::*;
}

pub use device::{DeviceConfig, MonitorDeviceConfig};
pub use ocpp::{OcppDeviceConfig, OcppModuleSpec, OcppSpec};
pub use session::{ClientOrServer, Endpoint, ModuleSpec, Role, Session};

use ferrowl_util::convert::{Converter, FileType};

/// Ferrowl version stamped into device/session files on save (see `DeviceConfig::version`,
/// `Session::version`) — informational only, never consulted by any load-time or migration
/// branch (CS-R-018, CS-R-022; migration keys off the legacy file shape per CS-R-040).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Sanitize a hand-edited (or dialog-typed) sim-cycle interval in seconds into a `Duration`: a
/// non-finite or non-positive value falls back to `default_secs` (instead of panicking in
/// `Duration::from_secs_f64` or busy-waiting on zero), and an otherwise-valid value is floored to
/// `min_secs` (instead of thrashing the sim thread on a near-zero interval). Pass `0.0` for
/// `min_secs` when no floor is wanted. Shared by [`Session::interval_duration`],
/// [`DeviceConfig::script_interval_duration`], and [`OcppDeviceConfig::script_interval_duration`]
/// so the NaN/negative/zero guard and floor rule stay in exactly one place.
pub(crate) fn sanitize_interval_secs(
    value: f64,
    default_secs: f64,
    min_secs: f64,
) -> std::time::Duration {
    let secs = if value.is_finite() && value > 0.0 {
        value.max(min_secs)
    } else {
        default_secs
    };
    std::time::Duration::from_secs_f64(secs)
}

/// Error type for config loading.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid file (JSON/TOML): {0}")]
    UnknownFormat(String),
    #[error("{0}")]
    Io(String),
    /// CS-R-068 — one or more retired, pre-merge flat TLS fields were found inside a `tls`
    /// subtree or an OCPP `security` table. `groups` carries the offenders split by the
    /// container they were found in, so each offender's message clause points at its own
    /// container's current block shape; no value is migrated.
    #[error("{}", render_retired_message(path, groups))]
    RetiredTlsFields {
        path: String,
        groups: Vec<RetiredTlsGroup>,
    },
}

/// CS-R-068 — the retired fields found within one [`TlsContainer`], one group per container so
/// the error can phrase one clause per container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredTlsGroup {
    pub container: TlsContainer,
    pub fields: Vec<&'static str>,
}

/// CS-R-068 — which TLS container a retired field was found in, so the error can point at that
/// container's own current block shape rather than a fixed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TlsContainer {
    /// A Modbus device/session `tls` container: `[tls.server]`/`[tls.client]`.
    Modbus,
    /// An OCPP device `security` table's `tls` sub-block: `[security.tls.server]`/
    /// `[security.tls.client]`.
    Ocpp,
}

impl TlsContainer {
    fn block_shape(self) -> &'static str {
        match self {
            TlsContainer::Modbus => "[tls.server]/[tls.client]",
            TlsContainer::Ocpp => "[security.tls.server]/[security.tls.client]",
        }
    }
}

impl std::fmt::Display for TlsContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.block_shape())
    }
}

/// CS-R-068 — renders one clause per [`TlsContainer`] group, each naming its own offenders and
/// pointing at its own current block shape.
fn render_retired_message(path: &str, groups: &[RetiredTlsGroup]) -> String {
    let clauses = groups
        .iter()
        .map(|g| {
            format!(
                "{}: a TLS block is now {} with mode = \"none\"|\"tls\"|\"mutual\", an identity sub-table (source = \"ephemeral\"|\"self-signed\"|\"files\") and a verification sub-table (verify = \"skip\"|\"root-store\"|\"ca-files\")",
                g.fields.join(", "),
                g.container
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("retired TLS fields in {path}: {clauses}; no value is migrated")
}

/// CS-R-068 — pre-merge flat TLS field names that a `tls` subtree or an OCPP `security` table
/// must reject rather than silently ignore under CS-R-052.
const RETIRED_TLS_FIELDS: &[&str] = &[
    "require_client_cert",
    "client_ca_files",
    "client_ca_file",
    "client_cert_skip_verify",
    "insecure_skip_verify",
    "client_cert_file",
    "client_key_file",
    "client_self_signed",
    "ca_file",
];
/// CS-R-068 — additionally retired when they appear outside an `identity` sub-table (inside one,
/// they are the current `CertSource::Files`/`SelfSigned` fields).
const RETIRED_OUTSIDE_IDENTITY: &[&str] = &["self_signed", "cert_file", "key_file"];

/// CS-R-068 — walks a generically-parsed document for retired TLS fields inside any `tls`
/// subtree or OCPP `security` table, tracking which container (if any) the current map sits
/// inside and whether it sits directly under a key named `identity` (where the three
/// outside-only names in [`RETIRED_OUTSIDE_IDENTITY`] are current, not retired). Descends into
/// array elements too, so a retired field nested under a list-valued key is not missed. Scope is
/// entered on any key literally named `tls`/`security` at any depth, not only the known
/// TLS-subtree positions, so an unrelated field coincidentally sharing one of those names would
/// be scanned as if it were a TLS container.
fn scan_retired_tls_fields(
    value: &serde_json::Value,
    container: Option<TlsContainer>,
    under_identity: bool,
    out: &mut Vec<(&'static str, TlsContainer)>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let scoped = match key.as_str() {
                    "security" if container.is_none() => Some(TlsContainer::Ocpp),
                    "tls" if container.is_none() => Some(TlsContainer::Modbus),
                    _ => container,
                };
                if let Some(found_in) = scoped {
                    let retired = RETIRED_TLS_FIELDS
                        .iter()
                        .find(|s| **s == key.as_str())
                        .or_else(|| {
                            (!under_identity)
                                .then(|| {
                                    RETIRED_OUTSIDE_IDENTITY
                                        .iter()
                                        .find(|s| **s == key.as_str())
                                })
                                .flatten()
                        });
                    if let Some(&name) = retired {
                        out.push((name, found_in));
                    }
                }
                scan_retired_tls_fields(child, scoped, key == "identity", out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                scan_retired_tls_fields(item, container, under_identity, out);
            }
        }
        _ => {}
    }
}

/// CS-R-068 — re-parses `path` generically (after the typed load above has already failed, via
/// the same [`Converter`] both formats already share) and looks for retired TLS fields, grouped
/// per [`TlsContainer`]. Returns `Some` with the phrased error only when at least one is found;
/// otherwise the original typed-load error should propagate unchanged.
fn retired_tls_fields_error(path: &str, ty: FileType) -> Option<ConfigError> {
    let json: serde_json::Value = Converter::load(path, ty).ok()?;
    let mut offenders: Vec<(&'static str, TlsContainer)> = Vec::new();
    scan_retired_tls_fields(&json, None, false, &mut offenders);
    if offenders.is_empty() {
        return None;
    }
    offenders.sort();
    offenders.dedup();
    let mut groups: Vec<RetiredTlsGroup> = Vec::new();
    for (field, container) in offenders {
        match groups.iter_mut().find(|g| g.container == container) {
            Some(g) => g.fields.push(field),
            None => groups.push(RetiredTlsGroup {
                container,
                fields: vec![field],
            }),
        }
    }
    Some(ConfigError::RetiredTlsFields {
        path: path.to_string(),
        groups,
    })
}

fn file_type(path: &str) -> Result<FileType, ConfigError> {
    FileType::from_path(path).ok_or_else(|| ConfigError::UnknownFormat(path.to_string()))
}

fn load<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, ConfigError> {
    let ty = file_type(path)?;
    Converter::load(path, ty).map_err(|e| {
        retired_tls_fields_error(path, ty).unwrap_or_else(|| ConfigError::Io(format!("{e:?}")))
    })
}

/// Load a device-type config file, migrating legacy per-register `update` scripts
/// into the global script list.
pub fn load_device(path: &str) -> Result<DeviceConfig, ConfigError> {
    let mut device: DeviceConfig = load(path)?;
    device.migrate_update_scripts();
    Ok(device)
}

pub fn load_ocpp_device(path: &str) -> Result<OcppDeviceConfig, ConfigError> {
    load(path)
}

/// Load a monitor device-type config file. No `migrate_update_scripts`-equivalent needed —
/// `MonitorRegisterDef` has no `update` field to migrate (MB-R-145).
///
/// `#[allow(dead_code)]`: implemented and tested here, with no app-side call site yet.
#[allow(dead_code)]
pub fn load_monitor_device(path: &str) -> Result<MonitorDeviceConfig, ConfigError> {
    load(path)
}

pub fn load_session(path: &str) -> Result<Session, ConfigError> {
    load(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_test_support::reserve_temp_dir;
    use ferrowl_util::convert::{Converter, FileType};

    #[test]
    /// CS-R-033 — a saved device/session file reloads to an equal value (envelope round-trips).
    fn ut_load_device_and_session_roundtrip() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let dpath = dir.join("device.toml").to_string_lossy().into_owned();
        Converter::save(&DeviceConfig::default(), &dpath, FileType::Toml).unwrap();
        assert_eq!(load_device(&dpath).unwrap(), DeviceConfig::default());

        let spath = dir.join("session.json").to_string_lossy().into_owned();
        Converter::save(&Session::default(), &spath, FileType::Json).unwrap();
        assert_eq!(load_session(&spath).unwrap(), Session::default());
    }

    #[test]
    /// api-contract.md §6 — a saved monitor device config file reloads to an equal value.
    fn ut_load_monitor_device_roundtrip() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("monitor_device.toml")
            .to_string_lossy()
            .into_owned();
        Converter::save(&MonitorDeviceConfig::default(), &path, FileType::Toml).unwrap();
        assert_eq!(
            load_monitor_device(&path).unwrap(),
            MonitorDeviceConfig::default()
        );
    }

    #[test]
    /// CS-R-054 — loading a device config self-heals a legacy per-register `update` snippet on every load.
    fn ut_load_device_migrates_update_scripts() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("legacy_update.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[definitions.reg]\ntype = \"U16\"\nupdate = \"C_Time:Sleep(1)\"\n",
        )
        .unwrap();
        let device = load_device(&path).unwrap();
        assert!(device.definitions["reg"].update.is_none());
        assert_eq!(device.scripts.len(), 1);
        assert_eq!(device.scripts[0].name, "reg");
        assert_eq!(device.scripts[0].code, "C_Time:Sleep(1)");
        assert!(device.scripts[0].enabled);
    }

    #[test]
    /// CS-R-003 — a path with an unknown extension fails to load with an unknown-format error.
    fn ut_load_unknown_format_errors() {
        let e = load_session("/tmp/ferrowl_cfg.bin");
        assert!(matches!(e, Err(ConfigError::UnknownFormat(_))));
    }

    #[test]
    fn ut_load_io_error() {
        let e = load_device("/no/such/ferrowl/device.toml");
        assert!(matches!(e, Err(ConfigError::Io(_))));
    }

    #[test]
    /// CS-R-052 — a field present in a file but absent from the schema is ignored on load.
    fn ut_load_ignores_unknown_field() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("unknown_field.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "bogus_unknown_field = 42\n[definitions.reg]\ntype = \"U16\"\n",
        )
        .unwrap();
        let device = load_device(&path).unwrap();
        assert!(device.definitions.contains_key("reg"));
    }
    /// CS-R-068 — a retired flat TLS field (`require_client_cert`) inside a device config's
    /// `tls` subtree fails the load, naming the retired field rather than a generic
    /// unknown-field error.
    #[test]
    fn ut_load_device_rejects_retired_require_client_cert() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_require_client_cert.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[definitions.reg]\ntype = \"U16\"\n[tls.server]\nmode = \"none\"\nrequire_client_cert = true\n",
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                assert!(
                    groups
                        .iter()
                        .any(|g| g.fields.contains(&"require_client_cert")),
                    "got: {groups:?}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-068 — a retired flat TLS field (`ca_file`) inside the OCPP `security` table fails
    /// the load, naming the retired field.
    #[test]
    fn ut_load_ocpp_device_rejects_retired_ca_file_in_security() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_ca_file.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[security]\nusername = \"cp001\"\nca_file = \"/etc/ca.pem\"\n",
        )
        .unwrap();
        let err = load_ocpp_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                assert!(
                    groups.iter().any(|g| g.fields.contains(&"ca_file")),
                    "got: {groups:?}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-068 — an error naming retired fields names every offender found, not just the
    /// first.
    #[test]
    fn ut_retired_field_error_names_every_offender() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_multiple.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[definitions.reg]\ntype = \"U16\"\n[tls.server]\nmode = \"none\"\nrequire_client_cert = true\nclient_ca_files = [\"a.pem\"]\n",
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                let fields: Vec<&str> = groups
                    .iter()
                    .flat_map(|g| g.fields.iter().copied())
                    .collect();
                assert!(fields.contains(&"require_client_cert"), "got: {fields:?}");
                assert!(fields.contains(&"client_ca_files"), "got: {fields:?}");
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-052, CS-R-072 — a stray key beside `registers` (outside any `tls`/`security` subtree) still
    /// loads silently, even though a retired-field scan now exists for TLS.
    #[test]
    fn ut_unknown_field_outside_tls_still_ignored() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("unknown_outside_tls.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "require_client_cert = true\n[definitions.reg]\ntype = \"U16\"\n",
        )
        .unwrap();
        let device = load_device(&path).unwrap();
        assert!(device.definitions.contains_key("reg"));
    }

    /// MB-R-112 — an RTU device session carrying a stray `tls.self_signed` key still loads:
    /// RTU's `Endpoint` variant defines no `tls` field, so the key is unreachable rather than
    /// rejected (docs/specs/modbus/edge-cases.md).
    #[test]
    fn ut_rtu_config_with_stray_tls_key_still_loads() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("rtu_stray_tls.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            r#"
[[modules]]
name = "m1"
device = "d1"
role = "client"

[modules.endpoint]
transport = "rtu"
path = "/dev/ttyUSB0"

[modules.endpoint.tls]
self_signed = true
"#,
        )
        .unwrap();
        let session = load_session(&path).unwrap();
        assert_eq!(session.modules.len(), 1);
    }

    /// CS-R-068 — the retired-field error for the OCPP `security` table points at its current
    /// block shape (`[security.tls.server]`/`[security.tls.client]`), not the Modbus `tls`
    /// container's shape.
    #[test]
    fn ut_retired_tls_error_names_ocpp_security_block_shape() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_ocpp_block_shape.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[security]\nusername = \"cp001\"\n[security.tls]\n[security.tls.server]\nrequire_client_cert = true\n",
        )
        .unwrap();
        let err = load_ocpp_device(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("[security.tls.server]/[security.tls.client]"),
            "got: {msg}"
        );
        assert!(!msg.contains("[tls.server]/[tls.client]"), "got: {msg}");
    }

    /// CS-R-068 — a key named `security` nested inside an already-scoped Modbus `tls` container
    /// stays in the Modbus group; it does not flip the group to the OCPP container.
    #[test]
    fn ut_security_key_nested_inside_modbus_tls_stays_modbus() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("nested_security_key_stays_modbus.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[definitions.reg]\ntype = \"U16\"\n[tls.server]\nmode = \"mutual\"\n[tls.server.security]\nrequire_client_cert = true\n",
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                assert_eq!(groups.len(), 1, "got: {groups:?}");
                assert_eq!(groups[0].container, TlsContainer::Modbus, "got: {groups:?}");
                assert!(
                    groups[0].fields.contains(&"require_client_cert"),
                    "got: {groups:?}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-068 — a document holding both a Modbus `tls` container and an OCPP `security` table
    /// with retired fields in each reports every offender under its own container's block
    /// shape, rather than collapsing all of them onto one.
    #[test]
    fn ut_mixed_modbus_and_ocpp_retired_fields_are_grouped_per_container() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_mixed_containers.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[definitions.reg]\ntype = \"U16\"\n[tls.server]\nmode = \"none\"\nrequire_client_cert = true\n\n[security]\nca_file = \"/etc/ca.pem\"\n",
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        let msg = err.to_string();
        match &err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                let modbus = groups
                    .iter()
                    .find(|g| g.container == TlsContainer::Modbus)
                    .expect("a Modbus group");
                assert_eq!(modbus.fields, vec!["require_client_cert"]);
                let ocpp = groups
                    .iter()
                    .find(|g| g.container == TlsContainer::Ocpp)
                    .expect("an OCPP group");
                assert_eq!(ocpp.fields, vec!["ca_file"]);
                assert!(msg.contains("[tls.server]/[tls.client]"), "got: {msg}");
                assert!(
                    msg.contains("[security.tls.server]/[security.tls.client]"),
                    "got: {msg}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-068 — a bare `cert_file` directly inside a `[tls.server]` block (not under its
    /// `identity` sub-table) is a retired flat field and is rejected.
    #[test]
    fn ut_bare_cert_file_outside_identity_is_retired() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("bare_cert_file.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[tls.server]\nmode = \"tls\"\ncert_file = \"s.crt\"\n",
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                assert!(
                    groups.iter().any(|g| g.fields.contains(&"cert_file")),
                    "got: {groups:?}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-068 — `cert_file`/`key_file` inside a `[tls.server.identity]` block are the current
    /// `CertSource::Files` fields, not retired ones: a load failing for an unrelated reason
    /// (an invalid `mode`) must not be replaced by a `RetiredTlsFields` error over them.
    #[test]
    fn ut_cert_file_inside_identity_block_is_not_retired() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("identity_cert_file_valid.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[tls.server]\nmode = \"bogus_mode\"\n[tls.server.identity]\nsource = \"files\"\ncert_file = \"s.crt\"\nkey_file = \"s.key\"\n",
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        assert!(
            matches!(err, ConfigError::Io(_)),
            "expected the original Io error to propagate unchanged, got: {err:?}"
        );
    }

    /// CS-R-068 — the scan reaches a retired field nested two levels below `tls`/`security`
    /// (`[tls.server.verification]`), not only a direct child.
    #[test]
    fn ut_retired_field_found_under_verification_nesting() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_under_verification.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[tls.server]\nmode = \"mutual\"\n[tls.server.verification]\nclient_ca_files = [\"a.pem\"]\n",
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                assert!(
                    groups.iter().any(|g| g.fields.contains(&"client_ca_files")),
                    "got: {groups:?}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-068 — the scan reaches a retired field nested two levels below the OCPP `security`
    /// table (`[security.tls.client]`), matching the container's own extra nesting level.
    #[test]
    fn ut_retired_field_found_under_security_tls_client_nesting() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_under_security_tls_client.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            "[security]\n[security.tls]\n[security.tls.client]\ninsecure_skip_verify = true\n",
        )
        .unwrap();
        let err = load_ocpp_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                assert!(
                    groups
                        .iter()
                        .any(|g| g.fields.contains(&"insecure_skip_verify")),
                    "got: {groups:?}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    /// CS-R-068 — the retired-field scan runs identically over a JSON config file.
    #[test]
    fn ut_load_device_rejects_retired_field_in_json() {
        let dir = reserve_temp_dir("ferrowl_cfgmod");
        let path = dir
            .join("retired_field.json")
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &path,
            r#"{"definitions":{"reg":{"type":"U16"}},"tls":{"server":{"mode":"none","require_client_cert":true}}}"#,
        )
        .unwrap();
        let err = load_device(&path).unwrap_err();
        match err {
            ConfigError::RetiredTlsFields { groups, .. } => {
                assert!(
                    groups
                        .iter()
                        .any(|g| g.fields.contains(&"require_client_cert")),
                    "got: {groups:?}"
                );
            }
            other => panic!("expected RetiredTlsFields, got: {other:?}"),
        }
    }

    #[test]
    fn ut_config_error_display() {
        assert!(
            ConfigError::UnknownFormat("p".into())
                .to_string()
                .contains("invalid file")
        );
        assert_eq!(ConfigError::Io("boom".into()).to_string(), "boom");
        let err = ConfigError::RetiredTlsFields {
            path: "d.toml".into(),
            groups: vec![RetiredTlsGroup {
                container: TlsContainer::Modbus,
                fields: vec!["require_client_cert"],
            }],
        };
        let msg = err.to_string();
        assert!(msg.contains("require_client_cert"), "got: {msg}");
        assert!(msg.contains("[tls.server]/[tls.client]"), "got: {msg}");
    }
}
