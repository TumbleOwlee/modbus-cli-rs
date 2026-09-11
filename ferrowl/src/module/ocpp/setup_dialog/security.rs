//! Security domain for the OCPP setup dialog: the TLS selector and the independent Basic
//! Authentication choice, plus the mapping between raw field text and an [`OcppSecurityConfig`]
//! (`from_config` infers a level, `build_config` resolves one, `validate_security` checks files).
//! Mirrors `ferrowl::module::modbus::setup_dialog::tls`'s `TlsLevel` exactly — see `OC-R-037`,
//! `OC-R-039`, `OC-R-040`, `OC-R-096`, `OC-R-110`, `OC-R-111`, `OC-R-113`, `OC-R-115`, `OC-R-116`,
//! `OC-R-127`, `OC-R-128`.

use ferrowl_ui::traits::ToLabel;
use ferrowl_util::tls::{CertSource, CertVerification, ClientTlsPolicy, ServerTlsPolicy};

use crate::dialog::tls_section::EffectiveTlsLevel;
use crate::module::ocpp::config::device::OcppSecurityConfig;
use crate::module::ocpp::config::session::OcppRole;

/// TLS selector (OC-R-127), shown for both roles: maps one-to-one onto the role's policy
/// variant, `Off` to `None`, `Tls` to `Tls`, `MutualTls` to `Mutual`.
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

impl From<TlsLevel> for EffectiveTlsLevel {
    fn from(level: TlsLevel) -> Self {
        match level {
            TlsLevel::Off => EffectiveTlsLevel::Off,
            TlsLevel::Tls => EffectiveTlsLevel::Tls,
            TlsLevel::MutualTls => EffectiveTlsLevel::MutualTls,
        }
    }
}

/// Basic Authentication toggle (OC-R-128), independent of [`TlsLevel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAuthChoice {
    Off,
    On,
}

impl ToLabel for BasicAuthChoice {
    fn to_label(&self) -> String {
        match self {
            BasicAuthChoice::Off => "Off",
            BasicAuthChoice::On => "On",
        }
        .to_string()
    }
}

/// Raw text of every security input field, plus every toggle, passed by name so the many
/// look-alike path fields cannot be transposed at a call site. `ca_files` is the shared
/// add/remove/edit list widget's current entries (OC-R-113/OC-R-125) — already individual
/// paths, no further parsing.
pub struct SecurityInputs<'a> {
    pub username: &'a str,
    pub password: &'a str,
    pub basic_auth: bool,
    pub cert_file: &'a str,
    pub key_file: &'a str,
    pub client_cert_file: &'a str,
    pub client_key_file: &'a str,
    pub ca_files: &'a [String],
    /// Server: server certificate is self-signed (OC-R-110). Client, at `MutualTls` only: the
    /// client's own mTLS identity is self-signed (OC-R-116).
    pub self_signed: bool,
    /// Client: accept any server certificate (OC-R-111).
    pub skip_verify: bool,
    /// Server, at `MutualTls` only: accept any client certificate (OC-R-113).
    pub client_cert_skip_verify: bool,
    /// Client-only Root Store toggle (OC-R-125): On resolves `ca_files` as `CertVerification::
    /// RootStore`'s `extra_ca_files`; Off, as `CertVerification::CaFiles`'s `ca_files`.
    pub root_store: bool,
}

impl TlsLevel {
    /// Infer the level an existing [`OcppSecurityConfig`] represents, by role, from that role's
    /// own nested policy (`cfg.tls.server`/`cfg.tls.client` — the other role's half is always
    /// present on the wire too, per `device.rs`'s doc comment, but never consulted here).
    pub fn from_config(cfg: &OcppSecurityConfig, role: OcppRole) -> TlsLevel {
        // OC-R-126: the level is decided by matching the role's own policy variant directly,
        // never by comparing against an all-unset field-presence baseline -- the variant *is*
        // the state (api-contract.md §9.1). `Ephemeral` (OC-R-095's "nothing configured, fall
        // back and log" identity) still resolves to `Tls` here, same as `SelfSigned`/`Files`:
        // it is distinguished from `None {}` by the outer `mode`, not by which identity a `Tls`
        // policy carries.
        match role {
            OcppRole::Client => match &cfg.tls.client {
                ClientTlsPolicy::Mutual { .. } => TlsLevel::MutualTls,
                ClientTlsPolicy::Tls { .. } => TlsLevel::Tls,
                ClientTlsPolicy::None {} => TlsLevel::Off,
            },
            OcppRole::Server => match &cfg.tls.server {
                ServerTlsPolicy::Mutual { .. } => TlsLevel::MutualTls,
                ServerTlsPolicy::Tls { .. } => TlsLevel::Tls,
                ServerTlsPolicy::None {} => TlsLevel::Off,
            },
        }
    }

    /// Build the active role's resolved policy from raw field text and toggle state (OC-R-111/
    /// OC-R-113/OC-R-116), producing a full [`OcppSecurityConfig`] whose *inactive* role's half
    /// is left at [`OcppSecurityConfig::default`]'s placeholder — the caller
    /// (`OcppSetupDialog::resolve`) overwrites that half from the original config, if any, so a
    /// role toggle preserves the other role's previously-saved settings. Basic Auth fields
    /// (`username`/`password`) are role-independent and gated on `basic_auth` alone (OC-R-128),
    /// never on `self`.
    pub fn build_config(
        self,
        role: OcppRole,
        inputs: SecurityInputs<'_>,
    ) -> Result<OcppSecurityConfig, String> {
        let SecurityInputs {
            username,
            password,
            basic_auth,
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

        let mut cfg = OcppSecurityConfig {
            username: if basic_auth { opt(username) } else { None },
            password: if basic_auth { opt(password) } else { None },
            ..OcppSecurityConfig::default()
        };

        match role {
            OcppRole::Server => {
                if self == TlsLevel::Off {
                    cfg.tls.server = ServerTlsPolicy::None {};
                    return Ok(cfg);
                }
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
                cfg.tls.server = if mtls {
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
            OcppRole::Client => {
                if self == TlsLevel::Off {
                    cfg.tls.client = ClientTlsPolicy::None {};
                    return Ok(cfg);
                }
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
                cfg.tls.client = if mtls {
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

/// Check every required credential/certificate file is present and exists on disk. `level` has
/// already been checked `!= Off` for the server role by the caller. `cfg` is assumed
/// already-resolved (the "CA list required" and "cert/key required unless self-signed" *shape*
/// errors are raised by [`TlsLevel::build_config`] itself, since only it has the raw toggle
/// state needed to phrase them helpfully) — this only adds the file-existence check on top.
pub(super) fn validate_security(
    cfg: &OcppSecurityConfig,
    role: OcppRole,
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
        OcppRole::Server => {
            let identity = match &cfg.tls.server {
                ServerTlsPolicy::Tls { identity } | ServerTlsPolicy::Mutual { identity, .. } => {
                    identity
                }
                ServerTlsPolicy::None {} => &CertSource::Ephemeral {},
            };
            // OC-R-110: Self-Signed needs no cert/key files.
            if level != TlsLevel::Off && !matches!(identity, CertSource::SelfSigned {}) {
                match identity {
                    CertSource::Files {
                        cert_file,
                        key_file,
                    } => {
                        exists("Certificate file", cert_file)?;
                        exists("Key file", key_file)?;
                    }
                    CertSource::Ephemeral {} => {
                        return Err("Certificate file is required for TLS.".to_string());
                    }
                    CertSource::SelfSigned {} => unreachable!("excluded by the outer !matches!"),
                }
            }
            if level == TlsLevel::MutualTls
                && let ServerTlsPolicy::Mutual { verification, .. } = &cfg.tls.server
            {
                match verification {
                    CertVerification::CaFiles { ca_files } => {
                        for ca in ca_files {
                            exists("Client CA file", ca)?;
                        }
                    }
                    CertVerification::Skip {} | CertVerification::RootStore { .. } => {}
                }
            }
        }
        OcppRole::Client => {
            let verification = match &cfg.tls.client {
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
                && let ClientTlsPolicy::Mutual { identity, .. } = &cfg.tls.client
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
    use crate::module::ocpp::config::device::OcppTlsConfig;

    #[test]
    /// UI-R-049 — `TlsLevel` maps 1:1 onto `EffectiveTlsLevel`, shared with the Modbus dialog's
    /// TLS section.
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

    #[allow(clippy::too_many_arguments)]
    fn inputs<'a>(
        username: &'a str,
        password: &'a str,
        cert_file: &'a str,
        key_file: &'a str,
        client_cert_file: &'a str,
        client_key_file: &'a str,
        ca_files: &'a [String],
    ) -> SecurityInputs<'a> {
        SecurityInputs {
            username,
            password,
            basic_auth: true,
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

    // --- TlsLevel::from_config -----------------------------------------------------------

    #[test]
    /// The TLS fields load `Off` from a no-TLS config for both roles.
    fn ut_from_config_none_both_roles() {
        let cfg = OcppSecurityConfig::default();
        assert_eq!(TlsLevel::from_config(&cfg, OcppRole::Client), TlsLevel::Off);
        assert_eq!(TlsLevel::from_config(&cfg, OcppRole::Server), TlsLevel::Off);
    }

    #[test]
    /// A client TLS config loads into the CA-file field.
    fn ut_from_config_tls_client_is_ca_file() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                client: ClientTlsPolicy::Tls {
                    verification: CertVerification::RootStore {
                        extra_ca_files: vec!["ca.pem".into()],
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(TlsLevel::from_config(&cfg, OcppRole::Client), TlsLevel::Tls);
    }

    #[test]
    /// A server TLS config loads into the cert and key fields.
    fn ut_from_config_tls_server_is_cert_and_key() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ServerTlsPolicy::Tls {
                    identity: CertSource::Files {
                        cert_file: "s.crt".into(),
                        key_file: "s.key".into(),
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(TlsLevel::from_config(&cfg, OcppRole::Server), TlsLevel::Tls);
    }

    #[test]
    /// OC-R-158 — the level is decided by the policy variant alone, never by comparing against
    /// an all-unset field-presence baseline: a client `Tls` policy whose `RootStore` carries an
    /// empty `extra_ca_files` (the widget's own default state) is still `Tls`, not `Off`.
    fn ut_from_config_tls_client_with_empty_root_store_is_still_tls() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                client: ClientTlsPolicy::Tls {
                    verification: CertVerification::RootStore {
                        extra_ca_files: vec![],
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(TlsLevel::from_config(&cfg, OcppRole::Client), TlsLevel::Tls);
    }

    #[test]
    /// OC-R-158 — a server `Tls` policy with a `SelfSigned` identity is still `Tls`, never
    /// falling through to `Off`: the variant is the state, distinguishable from the
    /// OC-R-095 `Ephemeral` fallback by construction rather than by a "was anything configured"
    /// comparison.
    fn ut_from_config_tls_server_self_signed_is_still_tls() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ServerTlsPolicy::Tls {
                    identity: CertSource::SelfSigned {},
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(TlsLevel::from_config(&cfg, OcppRole::Server), TlsLevel::Tls);
    }

    #[test]
    /// OC-R-158 — the OC-R-095 fallback state (`Ephemeral`) is distinguished from a real `Off`
    /// only by whether the policy is `None {}` at all, not by which identity variant it carries;
    /// `Ephemeral` itself is unreachable from the dialog (only `SelfSigned`/`Files` are offered),
    /// but a hand-edited config carrying it still reports `Tls`, matching OC-R-096's fallback.
    fn ut_from_config_tls_server_ephemeral_is_still_tls() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ServerTlsPolicy::Tls {
                    identity: CertSource::Ephemeral {},
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(TlsLevel::from_config(&cfg, OcppRole::Server), TlsLevel::Tls);
    }

    #[test]
    /// OC-R-116 — a mutual-TLS client config loads into the client-cert fields.
    fn ut_from_config_mutual_tls_client_is_client_cert() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
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
            },
            ..Default::default()
        };
        assert_eq!(
            TlsLevel::from_config(&cfg, OcppRole::Client),
            TlsLevel::MutualTls
        );
    }

    #[test]
    /// OC-R-113 — a mutual-TLS server config loads at the MutualTls level.
    fn ut_from_config_mutual_tls_server() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ServerTlsPolicy::Mutual {
                    identity: CertSource::SelfSigned {},
                    verification: CertVerification::CaFiles {
                        ca_files: vec!["ca.pem".to_string()],
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            TlsLevel::from_config(&cfg, OcppRole::Server),
            TlsLevel::MutualTls
        );
    }

    // --- TlsLevel::build_config -----------------------------------------------------------

    #[test]
    /// OC-R-127, OC-R-160 — selector `Off` resolves the server role's policy to `None {}`, reading no
    /// cert/key text at all even if present.
    fn ut_selector_off_resolves_none_policy_server() {
        let cfg = TlsLevel::Off
            .build_config(
                OcppRole::Server,
                inputs(
                    "u",
                    "p",
                    "cert",
                    "key",
                    "ccert",
                    "ckey",
                    &["cca".to_string()],
                ),
            )
            .unwrap();
        assert_eq!(cfg.username.as_deref(), Some("u"));
        assert_eq!(cfg.tls.server, ServerTlsPolicy::None {});
    }

    #[test]
    /// OC-R-127, OC-R-160 — selector `Off` resolves the client role's policy to `None {}`.
    fn ut_selector_off_resolves_none_policy_client() {
        let cfg = TlsLevel::Off
            .build_config(OcppRole::Client, inputs("", "", "", "", "", "", &[]))
            .unwrap();
        assert_eq!(cfg.tls.client, ClientTlsPolicy::None {});
    }

    #[test]
    /// OC-R-150 — a mutual-TLS server build parses the comma-separated CA list and sets a
    /// `Mutual` policy with `CaFiles{ca_files}`.
    fn ut_build_config_mutual_tls_server_parses_ca_list() {
        let cfg = TlsLevel::MutualTls
            .build_config(
                OcppRole::Server,
                inputs(
                    "",
                    "",
                    "cert",
                    "key",
                    "",
                    "",
                    &["ca1.pem".to_string(), "ca2.pem".to_string()],
                ),
            )
            .unwrap();
        match cfg.tls.server {
            ServerTlsPolicy::Mutual {
                verification: CertVerification::CaFiles { ca_files },
                ..
            } => assert_eq!(ca_files, vec!["ca1.pem".to_string(), "ca2.pem".to_string()]),
            other => panic!("expected Mutual with CaFiles, got {other:?}"),
        }
    }

    #[test]
    /// OC-R-150 — a mutual-TLS server build with an empty CA list and skip-verify off is a
    /// validation error.
    fn ut_build_config_mutual_tls_server_empty_ca_list_and_skip_verify_off_is_validation_error() {
        let err = TlsLevel::MutualTls
            .build_config(OcppRole::Server, inputs("", "", "cert", "key", "", "", &[]))
            .unwrap_err();
        assert!(err.contains("Client CA list is required"));
    }

    #[test]
    /// OC-R-147 — a mutual-TLS client build with the self-signed toggle on excludes the
    /// (possibly stale) client-cert/key text and resolves to `CertSource::SelfSigned`.
    fn ut_build_config_mutual_tls_client_self_signed_excludes_cert_key() {
        let mut i = inputs("", "", "", "", "stale.crt", "stale.key", &[]);
        i.self_signed = true;
        let cfg = TlsLevel::MutualTls
            .build_config(OcppRole::Client, i)
            .unwrap();
        assert_eq!(
            cfg.tls.client,
            ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
                identity: CertSource::SelfSigned {},
            }
        );
    }

    #[test]
    /// A mutual-TLS client build keeps the client cert/key (and any trust-anchor CA
    /// file text set alongside it — the two are independent axes, both legal under mTLS).
    fn ut_build_config_mutual_tls_client_keeps_client_cert_key() {
        let cfg = TlsLevel::MutualTls
            .build_config(
                OcppRole::Client,
                inputs("", "", "", "", "ccert", "ckey", &["ca".to_string()]),
            )
            .unwrap();
        assert_eq!(
            cfg.tls.client,
            ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec!["ca".to_string()],
                },
                identity: CertSource::Files {
                    cert_file: "ccert".to_string(),
                    key_file: "ckey".to_string(),
                },
            }
        );
    }

    #[test]
    /// OC-R-173 — Root Store On resolves the client verification to `CertVerification::
    /// RootStore` with the list as `extra_ca_files`, empty or not.
    fn ut_cs_root_store_on_resolves_root_store_with_list() {
        let list = ["ca1.pem".to_string()];
        let mut i = inputs("", "", "", "", "", "", &list);
        i.root_store = true;
        let cfg = TlsLevel::Tls.build_config(OcppRole::Client, i).unwrap();
        assert_eq!(
            cfg.tls.client,
            ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec!["ca1.pem".to_string()],
                },
            }
        );
    }

    #[test]
    /// OC-R-154 — Root Store Off with an empty CA list is a validation error, mirroring
    /// OC-R-150's empty-list rule on the server side.
    fn ut_cs_root_store_off_empty_list_is_validation_error() {
        let mut i = inputs("", "", "", "", "", "", &[]);
        i.root_store = false;
        let err = TlsLevel::Tls.build_config(OcppRole::Client, i).unwrap_err();
        assert!(err.contains("Root Store is Off"));
    }

    #[test]
    /// OC-R-174 — Root Store Off with a non-empty CA list resolves the client verification to
    /// `CertVerification::CaFiles` with the list as `ca_files`.
    fn ut_cs_root_store_off_resolves_ca_files_with_list() {
        let list = ["ca1.pem".to_string()];
        let mut i = inputs("", "", "", "", "", "", &list);
        i.root_store = false;
        let cfg = TlsLevel::Tls.build_config(OcppRole::Client, i).unwrap();
        assert_eq!(
            cfg.tls.client,
            ClientTlsPolicy::Tls {
                verification: CertVerification::CaFiles {
                    ca_files: vec!["ca1.pem".to_string()],
                },
            }
        );
    }

    #[test]
    /// OC-R-164 — Basic Auth Off leaves `username`/`password` both unset even with non-empty
    /// text present, regardless of the TLS selector (the On case is covered at
    /// setup_dialog.rs's `ut_basic_auth_on_resolves_credentials`). Uses `Tls`, not `Off`, so the
    /// gate is proven to be `basic_auth` alone, not the level: an `Off` fixture cannot
    /// distinguish the two, since the old level-derived rule and the independent toggle happen
    /// to agree there.
    fn ut_basic_auth_off_unsets_credentials_regardless_of_text() {
        let mut i = inputs("u", "p", "", "", "", "", &[]);
        i.basic_auth = false;
        let cfg = TlsLevel::Tls.build_config(OcppRole::Client, i).unwrap();
        assert_eq!(cfg.username, None);
        assert_eq!(cfg.password, None);
    }

    // --- validate_security -----------------------------------------------------------------------

    #[test]
    /// OC-R-143 — a server at TLS with self_signed set needs no cert/key files.
    fn ut_validate_security_server_self_signed_needs_no_files() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ServerTlsPolicy::Tls {
                    identity: CertSource::SelfSigned {},
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_security(&cfg, OcppRole::Server, TlsLevel::Tls, &|_| false).is_ok());
    }

    #[test]
    /// OC-R-113 — a server at mTLS additionally requires every listed client CA file to exist.
    fn ut_validate_security_server_mutual_tls_requires_client_ca_files() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ServerTlsPolicy::Mutual {
                    identity: CertSource::SelfSigned {},
                    verification: CertVerification::CaFiles {
                        ca_files: vec!["ca1.pem".to_string(), "ca2.pem".to_string()],
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_security(&cfg, OcppRole::Server, TlsLevel::MutualTls, &|_| true).is_ok());
        assert!(
            validate_security(&cfg, OcppRole::Server, TlsLevel::MutualTls, &|_| false).is_err()
        );
    }

    #[test]
    /// OC-R-151 — a server at mTLS with skip-verify on needs no CA files checked.
    fn ut_validate_security_server_mutual_tls_skip_verify_on_needs_no_ca_files() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                server: ServerTlsPolicy::Mutual {
                    identity: CertSource::SelfSigned {},
                    verification: CertVerification::Skip {},
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_security(&cfg, OcppRole::Server, TlsLevel::MutualTls, &|_| false).is_ok());
    }

    #[test]
    /// A client's CA file, when set, must exist; skip-verify alone needs no file.
    fn ut_validate_security_client_ca_file_must_exist_when_set() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                client: ClientTlsPolicy::Tls {
                    verification: CertVerification::RootStore {
                        extra_ca_files: vec!["ca.pem".into()],
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_security(&cfg, OcppRole::Client, TlsLevel::Tls, &|_| true).is_ok());
        assert!(validate_security(&cfg, OcppRole::Client, TlsLevel::Tls, &|_| false).is_err());
    }

    #[test]
    /// A client at mTLS requires existing client cert and key files.
    fn ut_validate_security_client_mutual_tls_requires_client_cert_key_files() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
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
            },
            ..Default::default()
        };
        assert!(validate_security(&cfg, OcppRole::Client, TlsLevel::MutualTls, &|_| true).is_ok());
        assert!(
            validate_security(&cfg, OcppRole::Client, TlsLevel::MutualTls, &|_| false).is_err()
        );
    }

    #[test]
    /// OC-R-147 — a client at mTLS with self-signed set needs no cert/key files checked.
    fn ut_validate_security_client_self_signed_needs_no_files() {
        let cfg = OcppSecurityConfig {
            tls: OcppTlsConfig {
                client: ClientTlsPolicy::Mutual {
                    verification: CertVerification::RootStore {
                        extra_ca_files: vec![],
                    },
                    identity: CertSource::SelfSigned {},
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_security(&cfg, OcppRole::Client, TlsLevel::MutualTls, &|_| false).is_ok());
    }
}
