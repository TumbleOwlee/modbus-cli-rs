//! Shared TLS-configuration enums, generalizing the server-certificate-source and
//! client-verification axes that both `ferrowl-modbus`'s `ModbusTlsConfig` and `ferrowl-ocpp`'s
//! `cs::Config.tls`/`csms::Config.tls`/the `ferrowl` crate's `OcppSecurityConfig` represent on
//! the wire. Each of the four types below is internally tagged and self-describing: the
//! serialized form is the sole representation, no shadow struct or hand-written
//! `Serialize`/`Deserialize` pair stands between file and type (MB-R-105/OC-R-126).

use serde::{Deserialize, Serialize};

/// A `ServerTlsPolicy`/`ClientTlsPolicy` construction-time rejection (MB-R-105/MB-R-108/
/// MB-R-109/MB-R-110) -- the three checks `validate()` performs that remain runtime rather than
/// structural, one variant each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("verify = \"ca-files\": ca_files must be non-empty")]
    EmptyCaFiles,
    #[error("source = \"ephemeral\" is not a valid client identity")]
    EphemeralClientIdentity,
    #[error("verify = \"root-store\" is client-only, not valid on a server")]
    RootStoreOnServer,
}

/// Where a certificate/key pair comes from (MB-R-105/OC-R-095/OC-R-096). `Ephemeral` denotes
/// "no TLS material configured, fall back and log" (MB-R-106/OC-R-095); `SelfSigned` is the same
/// generated pair but explicitly chosen, so no fallback is logged for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CertSource {
    Ephemeral {},
    SelfSigned {},
    Files { cert_file: String, key_file: String },
}

/// How a peer's certificate is verified (MB-R-105/MB-R-108/MB-R-109/OC-R-034/OC-R-036/OC-R-039).
/// `RootStore` is client-only (verifying a server); `CaFiles` is used by both roles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verify", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CertVerification {
    Skip {},
    RootStore {
        #[serde(default)]
        extra_ca_files: Vec<String>,
    },
    CaFiles {
        ca_files: Vec<String>,
    },
}

/// A server-role endpoint's TLS configuration (MB-R-105).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ServerTlsPolicy {
    None {},
    Tls {
        identity: CertSource,
    },
    Mutual {
        identity: CertSource,
        verification: CertVerification,
    },
}

impl Default for ServerTlsPolicy {
    fn default() -> Self {
        ServerTlsPolicy::None {}
    }
}

impl ServerTlsPolicy {
    /// The two checks that remain runtime rather than structural (MB-R-105/MB-R-108/MB-R-109):
    /// `CertVerification::CaFiles` must be non-empty, and `RootStore` is client-only, never a
    /// server's client-certificate verification.
    pub fn validate(&self) -> Result<(), PolicyError> {
        if let ServerTlsPolicy::Mutual { verification, .. } = self {
            match verification {
                CertVerification::CaFiles { ca_files } if ca_files.is_empty() => {
                    return Err(PolicyError::EmptyCaFiles);
                }
                CertVerification::RootStore { .. } => {
                    return Err(PolicyError::RootStoreOnServer);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// A client-role endpoint's TLS configuration (MB-R-105).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ClientTlsPolicy {
    None {},
    Tls {
        verification: CertVerification,
    },
    Mutual {
        verification: CertVerification,
        identity: CertSource,
    },
}

impl Default for ClientTlsPolicy {
    fn default() -> Self {
        ClientTlsPolicy::None {}
    }
}

impl ClientTlsPolicy {
    /// The two checks that remain runtime rather than structural (MB-R-105/MB-R-109/MB-R-110):
    /// `CertVerification::CaFiles` must be non-empty, and `CertSource::Ephemeral` is rejected as
    /// a client identity — "nothing configured, fall back and log" is a server-side behavior.
    pub fn validate(&self) -> Result<(), PolicyError> {
        match self {
            ClientTlsPolicy::Tls { verification } => {
                if let CertVerification::CaFiles { ca_files } = verification
                    && ca_files.is_empty()
                {
                    return Err(PolicyError::EmptyCaFiles);
                }
            }
            ClientTlsPolicy::Mutual {
                verification,
                identity,
            } => {
                if let CertVerification::CaFiles { ca_files } = verification
                    && ca_files.is_empty()
                {
                    return Err(PolicyError::EmptyCaFiles);
                }
                if matches!(identity, CertSource::Ephemeral {}) {
                    return Err(PolicyError::EphemeralClientIdentity);
                }
            }
            ClientTlsPolicy::None {} => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toml_round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let toml = toml::to_string(value).expect("serialize toml");
        let back: T = toml::from_str(&toml).expect("deserialize toml");
        assert_eq!(value, &back, "toml round-trip mismatch: {toml}");
    }

    fn json_round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize json");
        let back: T = serde_json::from_str(&json).expect("deserialize json");
        assert_eq!(value, &back, "json round-trip mismatch: {json}");
    }

    /// MB-R-105 — `ServerTlsPolicy::None` round-trips through TOML and JSON.
    #[test]
    fn ut_server_policy_none_round_trip() {
        toml_round_trip(&ServerTlsPolicy::None {});
        json_round_trip(&ServerTlsPolicy::None {});
    }

    /// MB-R-105 — `ServerTlsPolicy::Tls` with each `CertSource` variant round-trips through TOML.
    #[test]
    fn ut_server_policy_toml_round_trip() {
        toml_round_trip(&ServerTlsPolicy::Tls {
            identity: CertSource::Ephemeral {},
        });
        toml_round_trip(&ServerTlsPolicy::Tls {
            identity: CertSource::SelfSigned {},
        });
        toml_round_trip(&ServerTlsPolicy::Tls {
            identity: CertSource::Files {
                cert_file: "c.pem".into(),
                key_file: "k.pem".into(),
            },
        });
        toml_round_trip(&ServerTlsPolicy::Mutual {
            identity: CertSource::SelfSigned {},
            verification: CertVerification::CaFiles {
                ca_files: vec!["ca.pem".into()],
            },
        });
        toml_round_trip(&ServerTlsPolicy::Mutual {
            identity: CertSource::SelfSigned {},
            verification: CertVerification::Skip {},
        });
    }

    /// MB-R-105 — same shapes round-trip through JSON too.
    #[test]
    fn ut_server_policy_json_round_trip() {
        json_round_trip(&ServerTlsPolicy::Tls {
            identity: CertSource::Files {
                cert_file: "c.pem".into(),
                key_file: "k.pem".into(),
            },
        });
        json_round_trip(&ServerTlsPolicy::Mutual {
            identity: CertSource::SelfSigned {},
            verification: CertVerification::CaFiles {
                ca_files: vec!["ca.pem".into()],
            },
        });
    }

    /// MB-R-105 — `ClientTlsPolicy` with each `CertVerification` variant round-trips through
    /// TOML.
    #[test]
    fn ut_client_policy_toml_round_trip() {
        toml_round_trip(&ClientTlsPolicy::None {});
        toml_round_trip(&ClientTlsPolicy::Tls {
            verification: CertVerification::Skip {},
        });
        toml_round_trip(&ClientTlsPolicy::Tls {
            verification: CertVerification::RootStore {
                extra_ca_files: vec![],
            },
        });
        toml_round_trip(&ClientTlsPolicy::Tls {
            verification: CertVerification::RootStore {
                extra_ca_files: vec!["ca.pem".into()],
            },
        });
        toml_round_trip(&ClientTlsPolicy::Tls {
            verification: CertVerification::CaFiles {
                ca_files: vec!["ca.pem".into()],
            },
        });
        toml_round_trip(&ClientTlsPolicy::Mutual {
            verification: CertVerification::RootStore {
                extra_ca_files: vec![],
            },
            identity: CertSource::SelfSigned {},
        });
        toml_round_trip(&ClientTlsPolicy::Mutual {
            verification: CertVerification::Skip {},
            identity: CertSource::Files {
                cert_file: "c.pem".into(),
                key_file: "k.pem".into(),
            },
        });
    }

    /// MB-R-105 — `ClientTlsPolicy` round-trips through JSON too.
    #[test]
    fn ut_client_policy_json_round_trip() {
        json_round_trip(&ClientTlsPolicy::Mutual {
            verification: CertVerification::RootStore {
                extra_ca_files: vec!["ca.pem".into()],
            },
            identity: CertSource::SelfSigned {},
        });
    }

    /// MB-R-105 — a `None {}` variant carrying an unrelated field fails to
    /// deserialize under `deny_unknown_fields`, unlike a unit variant (which would silently
    /// discard it).
    #[test]
    fn ut_policy_none_serializes_as_mode_none() {
        let toml = toml::to_string(&ServerTlsPolicy::None {}).unwrap();
        assert_eq!(toml.trim(), r#"mode = "none""#);

        let err =
            toml::from_str::<ServerTlsPolicy>("mode = \"none\"\nrequire_client_cert = true\n")
                .unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "expected an unknown-field error, got: {err}"
        );
    }

    /// MB-R-164 — every tag (`mode`/`source`/`verify`) and kebab-case variant name is pinned in
    /// the serialized text, so a rename of any of them is caught here rather than passing
    /// silently through the round-trip helpers (which accept any tag spelling as long as it's
    /// self-consistent).
    #[test]
    fn ut_wire_tags_and_variant_names_are_pinned() {
        assert_eq!(
            toml::to_string(&CertSource::SelfSigned {}).unwrap().trim(),
            r#"source = "self-signed""#
        );
        assert_eq!(
            toml::to_string(&CertSource::Ephemeral {}).unwrap().trim(),
            r#"source = "ephemeral""#
        );
        assert_eq!(
            toml::to_string(&CertVerification::RootStore {
                extra_ca_files: vec![]
            })
            .unwrap()
            .lines()
            .next()
            .unwrap(),
            r#"verify = "root-store""#
        );
        assert_eq!(
            toml::to_string(&CertVerification::CaFiles {
                ca_files: vec!["ca.pem".to_string()]
            })
            .unwrap()
            .trim(),
            "verify = \"ca-files\"\nca_files = [\"ca.pem\"]"
        );
        assert_eq!(
            toml::to_string(&CertVerification::Skip {}).unwrap().trim(),
            r#"verify = "skip""#
        );
        assert_eq!(
            toml::to_string(&ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::Skip {},
            })
            .unwrap()
            .lines()
            .next()
            .unwrap(),
            r#"mode = "mutual""#
        );
        assert_eq!(
            toml::to_string(&ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            })
            .unwrap()
            .lines()
            .next()
            .unwrap(),
            r#"mode = "tls""#
        );
    }

    /// MB-R-107 — `CertSource::Files` naming only `cert_file` fails with `missing field
    /// \`key_file\``.
    #[test]
    fn ut_cert_source_files_requires_both_paths() {
        let err = toml::from_str::<CertSource>("source = \"files\"\ncert_file = \"c.pem\"\n")
            .unwrap_err();
        assert!(
            err.to_string().contains("missing field `key_file`"),
            "got: {err}"
        );
    }

    /// MB-R-108, MB-R-167, OC-R-133 — `CertVerification::RootStore` is rejected on a server's
    /// client-certificate verification (Modbus and CSMS alike).
    #[test]
    fn ut_server_policy_rejects_root_store_verification() {
        let policy = ServerTlsPolicy::Mutual {
            identity: CertSource::SelfSigned {},
            verification: CertVerification::RootStore {
                extra_ca_files: vec![],
            },
        };
        assert_eq!(policy.validate(), Err(PolicyError::RootStoreOnServer));
    }

    /// MB-R-108, MB-R-167, OC-R-039 — `CertVerification::CaFiles` with an empty `ca_files` is rejected at
    /// construction on the server role (Modbus and CSMS alike).
    #[test]
    fn ut_server_ca_files_empty_is_rejected() {
        let server = ServerTlsPolicy::Mutual {
            identity: CertSource::SelfSigned {},
            verification: CertVerification::CaFiles { ca_files: vec![] },
        };
        assert_eq!(server.validate(), Err(PolicyError::EmptyCaFiles));
    }

    /// MB-R-109, MB-R-167, OC-R-130 — `CertVerification::CaFiles` with an empty `ca_files` is rejected at
    /// construction on the client role (Modbus and CS alike).
    #[test]
    fn ut_client_ca_files_empty_is_rejected() {
        let client = ClientTlsPolicy::Tls {
            verification: CertVerification::CaFiles { ca_files: vec![] },
        };
        assert_eq!(client.validate(), Err(PolicyError::EmptyCaFiles));
    }

    /// MB-R-110, MB-R-167, MB-R-176, OC-R-168 — `CertSource::Ephemeral` is rejected as a client's mTLS identity
    /// (Modbus and CS alike).
    #[test]
    fn ut_client_policy_rejects_ephemeral_identity() {
        let policy = ClientTlsPolicy::Mutual {
            verification: CertVerification::Skip {},
            identity: CertSource::Ephemeral {},
        };
        assert_eq!(policy.validate(), Err(PolicyError::EphemeralClientIdentity));
    }

    /// MB-R-105 — a well-formed policy of either role passes validation.
    #[test]
    fn ut_validate_accepts_well_formed_policies() {
        assert!(ServerTlsPolicy::None {}.validate().is_ok());
        assert!(
            ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::CaFiles {
                    ca_files: vec!["ca.pem".into()]
                },
            }
            .validate()
            .is_ok()
        );
        assert!(ClientTlsPolicy::None {}.validate().is_ok());
        assert!(
            ClientTlsPolicy::Mutual {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![]
                },
                identity: CertSource::SelfSigned {},
            }
            .validate()
            .is_ok()
        );
    }

    /// CS-R-055 — a `None {}` policy block naming a retired flat field fails the load with an
    /// unknown-field error, rather than silently discarding it under CS-R-052.
    #[test]
    fn ut_policy_rejects_unknown_field_in_none_block() {
        let err =
            toml::from_str::<ServerTlsPolicy>("mode = \"none\"\nrequire_client_cert = true\n")
                .unwrap_err();
        assert!(
            err.to_string()
                .contains("unknown field `require_client_cert`"),
            "got: {err}"
        );
    }

    /// CS-R-055 — a `self-signed` `CertSource` naming a retired flat field (`client_self_signed`)
    /// fails the load rather than being ignored.
    #[test]
    fn ut_cert_source_rejects_unknown_field_in_self_signed_block() {
        let err =
            toml::from_str::<CertSource>("source = \"self-signed\"\nclient_self_signed = true\n")
                .unwrap_err();
        assert!(
            err.to_string()
                .contains("unknown field `client_self_signed`"),
            "got: {err}"
        );
    }

    /// CS-R-055 — a `skip` `CertVerification` naming `ca_files` (a field belonging only to the
    /// `ca-files` variant) fails the load rather than being ignored.
    #[test]
    fn ut_verification_rejects_ca_files_under_skip() {
        let err =
            toml::from_str::<CertVerification>("verify = \"skip\"\nca_files = [\"ca.pem\"]\n")
                .unwrap_err();
        assert!(
            err.to_string().contains("unknown field `ca_files`"),
            "got: {err}"
        );
    }
}
