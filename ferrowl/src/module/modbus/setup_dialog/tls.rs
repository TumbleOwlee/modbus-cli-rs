//! TLS domain for the Modbus setup dialog: the transport security level and its toggles, plus
//! the mapping between raw field text and a [`ModbusTlsConfig`] (`from_config` infers a level,
//! `build_config` resolves one, `validate_tls` checks files). Mirrors
//! `ferrowl::module::ocpp::setup_dialog::security` exactly, minus Basic Auth (Modbus/TCP TLS has
//! no credential level, only Off/TLS/mTLS) — see `MB-R-104`..`MB-R-112`, `MB-R-136`, `MB-R-139`.

use ferrowl_ui::traits::ToLabel;

use crate::config::ClientOrServer;
use crate::dialog::tls_section::EffectiveTlsLevel;
use ferrowl_modbus::tcp::ModbusTlsConfig;
use ferrowl_util::tls::{CertSource, CertVerification, ClientTlsPolicy, ServerTlsPolicy};

/// Modbus/TCP TLS level. Cumulative: `MutualTls`'s fields are shown in addition to `Tls`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TlsLevel {
    Off,
    Tls,
    MutualTls,
}

impl ToLabel for TlsLevel {
    fn to_label(&self) -> String {
        match self {
            TlsLevel::Off => "Off",
            TlsLevel::Tls => "TLS",
            TlsLevel::MutualTls => "mTLS",
        }
        .to_string()
    }
}

/// Modbus's TLS level has no credential tier (OCPP's Basic Authentication is a separate,
/// independent selection, not a level of its own `TlsLevel`), so this mapping is a plain 1:1
/// rename rather than a collapse.
impl From<TlsLevel> for EffectiveTlsLevel {
    fn from(level: TlsLevel) -> Self {
        match level {
            TlsLevel::Off => EffectiveTlsLevel::Off,
            TlsLevel::Tls => EffectiveTlsLevel::Tls,
            TlsLevel::MutualTls => EffectiveTlsLevel::MutualTls,
        }
    }
}

/// Raw text of every TLS path field, plus every toggle, passed by name so the many look-alike
/// path fields cannot be transposed at a call site. `ca_files` is the shared add/remove/edit
/// list widget's current entries (MB-R-136/MB-R-156) — already individual paths, no further
/// parsing.
pub struct TlsInputs<'a> {
    pub cert_file: &'a str,
    pub key_file: &'a str,
    pub client_cert_file: &'a str,
    pub client_key_file: &'a str,
    pub ca_files: &'a [String],
    /// Server: server certificate is self-signed (MB-R-106). Client, at `MutualTls` only: the
    /// client's own mTLS identity is self-signed (MB-R-138/139).
    pub self_signed: bool,
    /// Client: accept any server certificate.
    pub skip_verify: bool,
    /// Server, at `MutualTls` only: accept any client certificate (MB-R-136).
    pub client_cert_skip_verify: bool,
    /// Client-only Root Store toggle (MB-R-156): On resolves `ca_files` as `CertVerification::
    /// RootStore`'s `extra_ca_files`; Off, as `CertVerification::CaFiles`'s `ca_files`.
    pub root_store: bool,
}

impl TlsLevel {
    /// Infer the level an existing [`ModbusTlsConfig`] represents, by role, from that role's own
    /// nested policy (`cfg.server`/`cfg.client` — the other role's half is always present on the
    /// wire too, per `device.rs`'s doc comment, but never consulted here).
    pub fn from_config(cfg: &ModbusTlsConfig, role: ClientOrServer) -> TlsLevel {
        match role {
            ClientOrServer::Client => match &cfg.client {
                ClientTlsPolicy::Mutual { .. } => TlsLevel::MutualTls,
                ClientTlsPolicy::Tls { .. } => TlsLevel::Tls,
                ClientTlsPolicy::None {} => TlsLevel::Off,
            },
            ClientOrServer::Server => match &cfg.server {
                ServerTlsPolicy::Mutual { .. } => TlsLevel::MutualTls,
                ServerTlsPolicy::Tls { .. } => TlsLevel::Tls,
                ServerTlsPolicy::None {} => TlsLevel::Off,
            },
        }
    }

    /// Build the active role's resolved policy from raw field text and toggle state
    /// (MB-R-135/136/139), producing a full [`ModbusTlsConfig`] whose *inactive* role's half is
    /// left at [`ModbusTlsConfig::default`]'s placeholder — the caller (`SetupDialog::resolve`)
    /// overwrites that half from the original config, if any, so a role toggle preserves the
    /// other role's previously-saved settings.
    pub fn build_config(
        self,
        role: ClientOrServer,
        inputs: TlsInputs<'_>,
    ) -> Result<ModbusTlsConfig, String> {
        let TlsInputs {
            cert_file,
            key_file,
            client_cert_file,
            client_key_file,
            ca_files,
            self_signed,
            skip_verify,
            client_cert_skip_verify,
            root_store,
        } = inputs;
        let opt = |s: &str| {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        };
        let mtls = self == TlsLevel::MutualTls;

        let mut cfg = ModbusTlsConfig::default();
        match role {
            ClientOrServer::Server => {
                let identity = if self_signed {
                    CertSource::SelfSigned {}
                } else {
                    match (opt(cert_file), opt(key_file)) {
                        (Some(c), Some(k)) => CertSource::Files {
                            cert_file: c,
                            key_file: k,
                        },
                        (None, None) => CertSource::Ephemeral {},
                        _ => {
                            return Err(
                                "cert_file and key_file must both be set, or neither".to_string()
                            );
                        }
                    }
                };
                cfg.server = if mtls {
                    let ca_files = ca_files.to_vec();
                    if !client_cert_skip_verify && ca_files.is_empty() {
                        return Err(
                            "Client CA list is required for mTLS (or enable Skip Verify)."
                                .to_string(),
                        );
                    }
                    let verification = if client_cert_skip_verify {
                        CertVerification::Skip {}
                    } else {
                        CertVerification::CaFiles { ca_files }
                    };
                    ServerTlsPolicy::Mutual {
                        identity,
                        verification,
                    }
                } else {
                    ServerTlsPolicy::Tls { identity }
                };
            }
            ClientOrServer::Client => {
                let verification = if skip_verify {
                    CertVerification::Skip {}
                } else if root_store {
                    CertVerification::RootStore {
                        extra_ca_files: ca_files.to_vec(),
                    }
                } else if ca_files.is_empty() {
                    return Err(
                        "Server CA list is required when Root Store is Off (or enable Root Store / Skip Verify)."
                            .to_string(),
                    );
                } else {
                    CertVerification::CaFiles {
                        ca_files: ca_files.to_vec(),
                    }
                };
                cfg.client = if mtls {
                    let identity = if self_signed {
                        CertSource::SelfSigned {}
                    } else {
                        match (opt(client_cert_file), opt(client_key_file)) {
                            (Some(c), Some(k)) => CertSource::Files {
                                cert_file: c,
                                key_file: k,
                            },
                            _ => {
                                return Err(
                                    "client_cert_file and client_key_file must both be set, or self-signed set"
                                        .to_string(),
                                );
                            }
                        }
                    };
                    ClientTlsPolicy::Mutual {
                        verification,
                        identity,
                    }
                } else {
                    ClientTlsPolicy::Tls { verification }
                };
            }
        }
        Ok(cfg)
    }

    /// Index into the `tls_level` selection's value list (declaration order above).
    pub(super) fn index(self) -> usize {
        match self {
            TlsLevel::Off => 0,
            TlsLevel::Tls => 1,
            TlsLevel::MutualTls => 2,
        }
    }
}

/// Check every required certificate file is present and exists on disk. `level` has already been
/// checked `>= Tls` by the caller where relevant. `cfg` is assumed already-resolved (the
/// "CA list required" and "cert/key required unless self-signed" *shape* errors are raised by
/// [`TlsLevel::build_config`] itself, since only it has the raw toggle state needed to phrase
/// them helpfully) — this only adds the file-existence check on top.
pub(super) fn validate_tls(
    cfg: &ModbusTlsConfig,
    role: ClientOrServer,
    level: TlsLevel,
    file_exists: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    let exists = |label: &str, path: &str| -> Result<(), String> {
        if !file_exists(path) {
            return Err(format!("{label} not found: {path}"));
        }
        Ok(())
    };

    match role {
        ClientOrServer::Server => {
            let identity = match &cfg.server {
                ServerTlsPolicy::Tls { identity } | ServerTlsPolicy::Mutual { identity, .. } => {
                    identity
                }
                ServerTlsPolicy::None {} => &CertSource::Ephemeral {},
            };
            if level >= TlsLevel::Tls && !matches!(identity, CertSource::SelfSigned {}) {
                match identity {
                    CertSource::Files {
                        cert_file,
                        key_file,
                    } => {
                        exists("Certificate file", cert_file)?;
                        exists("Key file", key_file)?;
                    }
                    CertSource::Ephemeral {} => {
                        return Err(
                            "Certificate file is required for TLS (or enable Self-Signed)."
                                .to_string(),
                        );
                    }
                    CertSource::SelfSigned {} => unreachable!("excluded by the outer !matches!"),
                }
            }
            if level == TlsLevel::MutualTls
                && let ServerTlsPolicy::Mutual { verification, .. } = &cfg.server
                && let CertVerification::CaFiles { ca_files } = verification
            {
                for ca in ca_files {
                    exists("Client CA file", ca)?;
                }
            }
        }
        ClientOrServer::Client => {
            let verification = match &cfg.client {
                ClientTlsPolicy::Tls { verification }
                | ClientTlsPolicy::Mutual { verification, .. } => verification,
                ClientTlsPolicy::None {} => &CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
            };
            let ca_paths: &[String] = match verification {
                CertVerification::RootStore { extra_ca_files } => extra_ca_files,
                CertVerification::CaFiles { ca_files } => ca_files,
                CertVerification::Skip {} => &[],
            };
            for ca in ca_paths {
                if !ca.is_empty() {
                    exists("CA file", ca)?;
                }
            }
            if level == TlsLevel::MutualTls
                && let ClientTlsPolicy::Mutual { identity, .. } = &cfg.client
                && let CertSource::Files {
                    cert_file,
                    key_file,
                } = identity
            {
                exists("Client certificate file", cert_file)?;
                exists("Client key file", key_file)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// UI-R-049 — `TlsLevel` collapses 1:1 into the shared widget's level type, since Modbus's
    /// own level has no credential tier to fold away (OCPP's Basic Authentication is a separate
    /// selection, independent of its own `TlsLevel`).
    fn ut_effective_tls_level_from_tls_level() {
        assert_eq!(
            EffectiveTlsLevel::from(TlsLevel::Off),
            EffectiveTlsLevel::Off
        );
        assert_eq!(
            EffectiveTlsLevel::from(TlsLevel::Tls),
            EffectiveTlsLevel::Tls
        );
        assert_eq!(
            EffectiveTlsLevel::from(TlsLevel::MutualTls),
            EffectiveTlsLevel::MutualTls
        );
    }

    fn inputs<'a>(
        cert_file: &'a str,
        key_file: &'a str,
        client_cert_file: &'a str,
        client_key_file: &'a str,
        ca_files: &'a [String],
    ) -> TlsInputs<'a> {
        TlsInputs {
            cert_file,
            key_file,
            client_cert_file,
            client_key_file,
            ca_files,
            self_signed: false,
            skip_verify: false,
            client_cert_skip_verify: false,
            root_store: true,
        }
    }

    // --- TlsLevel::from_config -----------------------------------------------------------------

    #[test]
    /// MB-R-162 — the TLS fields load from a no-TLS-block config for both roles as `Off`, since
    /// `ModbusTlsConfig::default()` is both policies `None`.
    fn ut_from_config_default_both_roles_is_off() {
        let cfg = ModbusTlsConfig::default();
        assert_eq!(
            TlsLevel::from_config(&cfg, ClientOrServer::Client),
            TlsLevel::Off
        );
        assert_eq!(
            TlsLevel::from_config(&cfg, ClientOrServer::Server),
            TlsLevel::Off
        );
    }

    #[test]
    /// A mutual-TLS client config loads at the MutualTls level.
    fn ut_from_config_mutual_tls_client() {
        let cfg = ModbusTlsConfig {
            client: ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
                identity: CertSource::SelfSigned {},
            },
            ..Default::default()
        };
        assert_eq!(
            TlsLevel::from_config(&cfg, ClientOrServer::Client),
            TlsLevel::MutualTls
        );
    }

    #[test]
    /// MB-R-136 — a mutual-TLS server config loads at the MutualTls level.
    fn ut_from_config_mutual_tls_server() {
        let cfg = ModbusTlsConfig {
            server: ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::CaFiles {
                    ca_files: vec!["ca.pem".to_string()],
                },
            },
            ..Default::default()
        };
        assert_eq!(
            TlsLevel::from_config(&cfg, ClientOrServer::Server),
            TlsLevel::MutualTls
        );
    }

    // --- TlsLevel::build_config -----------------------------------------------------------------

    #[test]
    /// MB-R-135 — a server TLS build resolves cert/key from raw text when self-signed is off.
    fn ut_build_config_tls_server_resolves_cert_key() {
        let cfg = TlsLevel::Tls
            .build_config(ClientOrServer::Server, inputs("cert", "key", "", "", &[]))
            .unwrap();
        assert_eq!(
            cfg.server,
            ServerTlsPolicy::Tls {
                identity: CertSource::Files {
                    cert_file: "cert".to_string(),
                    key_file: "key".to_string(),
                }
            }
        );
    }

    #[test]
    /// MB-R-188 — a mutual-TLS server build parses the comma-separated CA list and sets
    /// `require`-shaped `Mutual` with `CaFiles{ca_files}`.
    fn ut_build_config_mutual_tls_server_parses_ca_list() {
        let cfg = TlsLevel::MutualTls
            .build_config(
                ClientOrServer::Server,
                inputs(
                    "cert",
                    "key",
                    "",
                    "",
                    &["ca1.pem".to_string(), "ca2.pem".to_string()],
                ),
            )
            .unwrap();
        match cfg.server {
            ServerTlsPolicy::Mutual {
                verification: CertVerification::CaFiles { ca_files },
                ..
            } => assert_eq!(ca_files, vec!["ca1.pem".to_string(), "ca2.pem".to_string()]),
            other => panic!("expected Mutual with CaFiles, got {other:?}"),
        }
    }

    #[test]
    /// MB-R-188 — a mutual-TLS server build with an empty CA list and skip-verify off is a
    /// validation error (rather than silently constructing an unrepresentable
    /// `CertVerification::CaFiles { ca_files: vec![] }`).
    fn ut_build_config_mutual_tls_server_empty_ca_list_and_skip_verify_off_is_validation_error() {
        let err = TlsLevel::MutualTls
            .build_config(ClientOrServer::Server, inputs("cert", "key", "", "", &[]))
            .unwrap_err();
        assert!(err.contains("Client CA list is required"));
    }

    #[test]
    /// MB-R-189 — a mutual-TLS server build with skip-verify on needs no CA list at all.
    fn ut_build_config_mutual_tls_server_skip_verify_needs_no_ca_list() {
        let mut i = inputs("cert", "key", "", "", &[]);
        i.client_cert_skip_verify = true;
        let cfg = TlsLevel::MutualTls
            .build_config(ClientOrServer::Server, i)
            .unwrap();
        assert_eq!(
            cfg.server,
            ServerTlsPolicy::Mutual {
                identity: CertSource::Files {
                    cert_file: "cert".to_string(),
                    key_file: "key".to_string(),
                },
                verification: CertVerification::Skip {},
            }
        );
    }

    #[test]
    /// MB-R-213 — a mutual-TLS client build with the self-signed toggle on excludes the
    /// (possibly stale) client-cert/key text and resolves to `CertSource::SelfSigned`.
    fn ut_build_config_mutual_tls_client_self_signed_excludes_cert_key() {
        let mut i = inputs("", "", "stale.crt", "stale.key", &[]);
        i.self_signed = true;
        let cfg = TlsLevel::MutualTls
            .build_config(ClientOrServer::Client, i)
            .unwrap();
        assert_eq!(
            cfg.client,
            ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
                identity: CertSource::SelfSigned {},
            }
        );
    }

    #[test]
    /// MB-R-156 — Root Store On resolves the client verification to `CertVerification::
    /// RootStore` with the list as `extra_ca_files`, empty or not.
    fn ut_client_root_store_on_resolves_root_store_with_list() {
        let list = ["ca1.pem".to_string(), "ca2.pem".to_string()];
        let mut i = inputs("", "", "", "", &list);
        i.root_store = true;
        let cfg = TlsLevel::Tls
            .build_config(ClientOrServer::Client, i)
            .unwrap();
        assert_eq!(
            cfg.client,
            ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec!["ca1.pem".to_string(), "ca2.pem".to_string()],
                },
            }
        );
    }

    #[test]
    /// MB-R-156 — Root Store Off resolves the client verification to `CertVerification::
    /// CaFiles` with the list as `ca_files`.
    fn ut_client_root_store_off_resolves_ca_files() {
        let list = ["ca1.pem".to_string()];
        let mut i = inputs("", "", "", "", &list);
        i.root_store = false;
        let cfg = TlsLevel::Tls
            .build_config(ClientOrServer::Client, i)
            .unwrap();
        assert_eq!(
            cfg.client,
            ClientTlsPolicy::Tls {
                verification: CertVerification::CaFiles {
                    ca_files: vec!["ca1.pem".to_string()],
                },
            }
        );
    }

    #[test]
    /// MB-R-202 — Root Store Off with an empty CA list is a validation error, mirroring
    /// MB-R-136's empty-list rule on the server side: a verification naming no trust anchor
    /// rejects every server certificate and is never the user's intent.
    fn ut_client_root_store_off_with_empty_list_is_validation_error() {
        let mut i = inputs("", "", "", "", &[]);
        i.root_store = false;
        let err = TlsLevel::Tls
            .build_config(ClientOrServer::Client, i)
            .unwrap_err();
        assert!(err.contains("Root Store is Off"));
    }

    #[test]
    /// Building the config for the inactive role leaves it at `ModbusTlsConfig`'s
    /// default placeholder (the caller stitches in the real inactive-role config, if any).
    fn ut_build_config_leaves_inactive_role_at_default() {
        let cfg = TlsLevel::Tls
            .build_config(ClientOrServer::Server, inputs("cert", "key", "", "", &[]))
            .unwrap();
        assert_eq!(cfg.client, ModbusTlsConfig::default().client);
    }

    // --- validate_tls ----------------------------------------------------------------------------

    #[test]
    /// A server at TLS with self_signed set needs no cert/key files.
    fn ut_validate_tls_server_self_signed_needs_no_files() {
        let cfg = ModbusTlsConfig {
            server: ServerTlsPolicy::Tls {
                identity: CertSource::SelfSigned {},
            },
            ..Default::default()
        };
        assert!(validate_tls(&cfg, ClientOrServer::Server, TlsLevel::Tls, &|_| false).is_ok());
    }

    #[test]
    /// A server at TLS without self_signed requires an existing cert and key file.
    fn ut_validate_tls_server_requires_cert_and_key_files() {
        let missing = ModbusTlsConfig {
            server: ServerTlsPolicy::Tls {
                identity: CertSource::Ephemeral {},
            },
            ..Default::default()
        };
        assert!(validate_tls(&missing, ClientOrServer::Server, TlsLevel::Tls, &|_| false).is_err());

        let cfg = ModbusTlsConfig {
            server: ServerTlsPolicy::Tls {
                identity: CertSource::Files {
                    cert_file: "s.crt".into(),
                    key_file: "s.key".into(),
                },
            },
            ..Default::default()
        };
        assert!(validate_tls(&cfg, ClientOrServer::Server, TlsLevel::Tls, &|_| true).is_ok());
        assert!(validate_tls(&cfg, ClientOrServer::Server, TlsLevel::Tls, &|_| false).is_err());
    }

    #[test]
    /// MB-R-136 — a server at mTLS additionally requires every listed client CA file to exist.
    fn ut_validate_tls_server_mutual_tls_requires_client_ca_files() {
        let cfg = ModbusTlsConfig {
            server: ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::CaFiles {
                    ca_files: vec!["ca1.pem".to_string(), "ca2.pem".to_string()],
                },
            },
            ..Default::default()
        };
        assert!(validate_tls(&cfg, ClientOrServer::Server, TlsLevel::MutualTls, &|_| true).is_ok());
        assert!(
            validate_tls(&cfg, ClientOrServer::Server, TlsLevel::MutualTls, &|_| {
                false
            })
            .is_err()
        );
    }

    #[test]
    /// MB-R-189 — a server at mTLS with skip-verify on needs no CA files checked.
    fn ut_validate_tls_server_mutual_tls_skip_verify_on_needs_no_ca_files() {
        let cfg = ModbusTlsConfig {
            server: ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::Skip {},
            },
            ..Default::default()
        };
        assert!(
            validate_tls(&cfg, ClientOrServer::Server, TlsLevel::MutualTls, &|_| {
                false
            })
            .is_ok()
        );
    }

    #[test]
    /// A client's CA file, when set, must exist; skip-verify alone needs no file.
    fn ut_validate_tls_client_ca_file_must_exist_when_set() {
        let cfg = ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec!["ca.pem".into()],
                },
            },
            ..Default::default()
        };
        assert!(validate_tls(&cfg, ClientOrServer::Client, TlsLevel::Tls, &|_| true).is_ok());
        assert!(validate_tls(&cfg, ClientOrServer::Client, TlsLevel::Tls, &|_| false).is_err());

        let skip_verify_only = ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
            ..Default::default()
        };
        assert!(
            validate_tls(
                &skip_verify_only,
                ClientOrServer::Client,
                TlsLevel::Tls,
                &|_| false
            )
            .is_ok()
        );
    }

    #[test]
    /// A client at mTLS requires existing client cert and key files.
    fn ut_validate_tls_client_mutual_tls_requires_client_cert_key_files() {
        let cfg = ModbusTlsConfig {
            client: ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
                identity: CertSource::Files {
                    cert_file: "c.crt".into(),
                    key_file: "c.key".into(),
                },
            },
            ..Default::default()
        };
        assert!(validate_tls(&cfg, ClientOrServer::Client, TlsLevel::MutualTls, &|_| true).is_ok());
        assert!(
            validate_tls(&cfg, ClientOrServer::Client, TlsLevel::MutualTls, &|_| {
                false
            })
            .is_err()
        );
    }

    #[test]
    /// MB-R-214 — a client at mTLS with self-signed set needs no cert/key files checked.
    fn ut_validate_tls_client_self_signed_needs_no_files() {
        let cfg = ModbusTlsConfig {
            client: ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
                identity: CertSource::SelfSigned {},
            },
            ..Default::default()
        };
        assert!(
            validate_tls(&cfg, ClientOrServer::Client, TlsLevel::MutualTls, &|_| {
                false
            })
            .is_ok()
        );
    }
}
