//! Command-line interface. Modules can be supplied ad-hoc with repeatable
//! `--module key=val,...` flags and/or pre-configured `--session <file>` files; both resolve
//! to the same [`ModuleSpec`] list.

use std::collections::HashMap;

use clap::{Args, Parser, Subcommand};

pub mod bridge;
pub mod headless;

use crate::config::ocpp::OcppProtocol;
use crate::config::{self, ClientOrServer, Endpoint, ModuleSpec, OcppModuleSpec, Role};

#[derive(Parser, Debug)]
#[command(version, about = "Ferrowl — a modbus client/server TUI", long_about = None)]
pub struct CliArgs {
    /// Migrate a v0.3.9 config to the current device config format instead of starting the TUI.
    #[command(subcommand)]
    pub command: Option<SubCommand>,

    /// A module to start, e.g.
    /// --module name=evse-1,device=configs/evse.toml,transport=tcp,ip=10.0.0.5,port=502,role=server
    #[arg(long = "module", value_name = "KEY=VAL,...")]
    pub modules: Vec<String>,

    /// A session file listing multiple module instances (repeatable).
    #[arg(long = "session", value_name = "FILE")]
    pub sessions: Vec<String>,

    /// A device configuration used to initialize module.
    #[arg(long = "device", value_name = "FILE")]
    pub devices: Vec<String>,

    /// Demo mode
    #[arg(long)]
    pub demo: bool,
}

#[derive(Subcommand, Debug)]
pub enum SubCommand {
    /// Migrate a v0.3.9 (`modbus-cli-rs`) configuration file to the current device config format.
    ///
    /// Reads a TOML or JSON config from INPUT and writes a converted DeviceConfig to OUTPUT.
    /// Warnings about dropped or approximated fields are printed to stderr.
    Migrate(MigrateArgs),

    /// Run configured modules without the TUI (headless/CI mode). See [`crate::cli::headless::run`]
    /// for the exit-code contract.
    Run(RunArgs),

    /// Relay Modbus requests between two interfaces without the TUI: an upstream interface that
    /// bridge mode serves (answering requests, like a real device) and a downstream interface it
    /// connects to as a client, forwarding every request it receives on the upstream side and
    /// relaying the answer back. Useful for placing a TCP-only master in front of a serial-only
    /// device, or vice versa.
    Bridge(BridgeArgs),
}

#[derive(Args, Debug)]
pub struct BridgeArgs {
    /// Required. The interface bridge mode listens on and answers requests from, e.g.
    /// --upstream transport=tcp,ip=0.0.0.0,port=502
    /// or --upstream transport=rtu,path=/dev/ttyUSB0,baud=19200
    #[arg(long, value_name = "KEY=VAL,...")]
    pub upstream: Option<String>,

    /// Required. The interface bridge mode connects to and forwards every upstream request to,
    /// e.g. --downstream transport=tcp,ip=10.0.0.5,port=502
    /// or --downstream transport=rtu,path=/dev/ttyUSB1,baud=19200
    /// Both descriptors accept `transport` (`tcp` default, `rtu`, `rtu_over_tcp`, or
    /// `ascii_over_tcp`), `timeout_ms`, `reconnect` (true/false); `tcp`/`rtu_over_tcp`/
    /// `ascii_over_tcp` also take `ip`, `port`, and TLS keys; `rtu` also takes `path`, `baud`,
    /// `parity`, `data_bits`, `stop_bits`. `--upstream` additionally accepts `unit_ids` (e.g.
    /// `unit_ids=1,3,5-8`) to restrict which slave ids the bridge answers for.
    #[arg(long, value_name = "KEY=VAL,...")]
    pub downstream: Option<String>,

    /// Run for this many seconds then exit cleanly (code 0). Omit to run until Ctrl-C.
    #[arg(long, value_name = "SECS")]
    pub duration: Option<u64>,

    /// Append every drained log line to this file too (in addition to stdout).
    #[arg(long = "log-file", value_name = "FILE")]
    pub log_file: Option<String>,

    /// Exit with code 2 if a drained log line starts with the `[bridge]` prefix bridge errors are
    /// logged under. This is plain log-string detection, not a structured error channel, so it
    /// only catches errors that are actually logged.
    #[arg(long = "exit-on-error")]
    pub exit_on_error: bool,
}

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Path to the v0.3.9 configuration file (.toml or .json).
    #[arg(long, short, value_name = "FILE")]
    pub input: String,

    /// Destination path for the converted device config (.toml or .json).
    #[arg(long, short, value_name = "FILE")]
    pub output: String,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// A session file listing multiple module instances (repeatable).
    #[arg(long = "session", value_name = "FILE")]
    pub sessions: Vec<String>,

    /// A module to start, e.g.
    /// --module name=evse-1,device=configs/evse.toml,transport=tcp,ip=10.0.0.5,port=502,role=server
    #[arg(long = "module", value_name = "KEY=VAL,...")]
    pub modules: Vec<String>,

    /// An ad-hoc OCPP module to start, e.g.
    /// --ocpp name=cs-1,device=configs/cs.toml,protocol=ws,ip=127.0.0.1,port=9000,path=/ocpp/cp001
    #[arg(long = "ocpp", value_name = "KEY=VAL,...")]
    pub ocpp: Vec<String>,

    /// Run for this many seconds then exit cleanly (code 0). Omit to run until Ctrl-C.
    #[arg(long, value_name = "SECS")]
    pub duration: Option<u64>,

    /// Append every drained log line to this file too (in addition to stdout).
    #[arg(long = "log-file", value_name = "FILE")]
    pub log_file: Option<String>,

    /// Exit with code 3 (after stopping every module) if a drained log line has log level error.
    #[arg(long = "exit-on-error")]
    pub exit_on_error: bool,
}

impl RunArgs {
    /// Resolve modbus module specs the same way the TUI path does: delegate to
    /// [`CliArgs::module_specs`] over the equivalent `--session`/`--module` flags.
    pub fn module_specs(&self) -> Result<Vec<ModuleSpec>, String> {
        self.as_cli_args().module_specs()
    }

    /// Resolve OCPP module specs from `--session` files (same as the TUI path via
    /// [`CliArgs::ocpp_specs`]), plus any ad-hoc `--ocpp key=val,...` flags.
    pub fn ocpp_specs(&self) -> Result<Vec<OcppModuleSpec>, String> {
        let mut specs = self.as_cli_args().ocpp_specs()?;
        for spec in &self.ocpp {
            specs.push(parse_ocpp_spec(spec)?);
        }
        Ok(specs)
    }

    fn as_cli_args(&self) -> CliArgs {
        CliArgs {
            command: None,
            modules: self.modules.clone(),
            sessions: self.sessions.clone(),
            devices: Vec::new(),
            demo: false,
        }
    }
}

impl CliArgs {
    /// Resolve every module instance from `--session` files (first) and `--module` flags.
    /// Session modules are stored as `serde_json::Value`; we dispatch on the `"type"` field
    /// (defaulting to `"modbus"`) to deserialize the right spec type.
    pub fn module_specs(&self) -> Result<Vec<ModuleSpec>, String> {
        let mut specs = Vec::new();
        for path in &self.sessions {
            let session = config::load_session(path).map_err(|e| e.to_string())?;
            for module_val in session.modules {
                let ty = module_val
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("modbus");
                match ty {
                    "modbus" => {
                        let spec: ModuleSpec = serde_json::from_value(module_val)
                            .map_err(|e| format!("invalid modbus module spec: {e}"))?;
                        specs.push(spec);
                    }
                    // OCPP modules are resolved separately by `ocpp_specs`.
                    "ocpp" => {}
                    other => {
                        return Err(format!("unsupported module type '{other}'"));
                    }
                }
            }
        }
        for spec in &self.modules {
            specs.push(parse_module_spec(spec)?);
        }
        for (num, device) in self.devices.iter().enumerate() {
            specs.push(create_module_spec_by_device(
                format!("Device {num}"),
                device.clone(),
            ));
        }
        Ok(specs)
    }

    /// Resolve every OCPP module instance from `--session` files (modules tagged
    /// `"type":"ocpp"`). Each entry carries the device-config path + endpoint; the device file
    /// (version/role/timeout/scripts) is loaded separately when the tab is built.
    pub fn ocpp_specs(&self) -> Result<Vec<OcppModuleSpec>, String> {
        let mut specs = Vec::new();
        for path in &self.sessions {
            let session = config::load_session(path).map_err(|e| e.to_string())?;
            for module_val in session.modules {
                let ty = module_val.get("type").and_then(|v| v.as_str());
                if ty == Some("ocpp") {
                    let spec: OcppModuleSpec = serde_json::from_value(module_val)
                        .map_err(|e| format!("invalid ocpp module spec: {e}"))?;
                    specs.push(spec);
                }
            }
        }
        Ok(specs)
    }
}

/// Build a [`ModuleSpec`] for a `--device` flag: a TCP client polling the default demo
/// endpoint 127.0.0.1:5020 (matches the `--demo` server). Endpoint/role are not
/// configurable here — use `--module` for full control.
pub fn create_module_spec_by_device(name: String, device: String) -> ModuleSpec {
    ModuleSpec {
        name,
        device,
        role: Role::Client,
        endpoint: Endpoint::Tcp {
            ip: "127.0.0.1".to_string(),
            port: 5020,
        },
    }
}

/// Parse a single `--module` value (`key=val,key=val,...`) into a [`ModuleSpec`].
pub fn parse_module_spec(input: &str) -> Result<ModuleSpec, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got '{part}'"))?;
        map.insert(key.trim().to_string(), value.trim().to_string());
    }

    let get = |k: &str| map.get(k).cloned();

    let name = get("name").ok_or("module requires 'name'")?;
    let device = get("device")
        .or_else(|| get("type"))
        .ok_or("module requires 'device' (or 'type')")?;

    let role = match get("role").as_deref() {
        None | Some("server") => Role::Server,
        Some("client") => Role::Client,
        Some(other) => return Err(format!("invalid role '{other}' (expected client|server)")),
    };

    let transport = get("transport").unwrap_or_else(|| "tcp".to_string());
    let endpoint = match transport.as_str() {
        "tcp" => Endpoint::Tcp {
            ip: get("ip").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: get("port")
                .ok_or("tcp module requires 'port'")?
                .parse()
                .map_err(|_| "invalid 'port'")?,
        },
        "rtu_over_tcp" => Endpoint::RtuOverTcp {
            ip: get("ip").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: get("port")
                .ok_or("rtu_over_tcp module requires 'port'")?
                .parse()
                .map_err(|_| "invalid 'port'")?,
        },
        "rtu" => Endpoint::Rtu {
            path: get("path").ok_or("rtu module requires 'path'")?,
            baud_rate: parse_opt(get("baud").or_else(|| get("baud_rate")), "baud")?
                .unwrap_or(19200),
            parity: get("parity"),
            data_bits: parse_opt(get("data_bits"), "data_bits")?,
            stop_bits: parse_opt(get("stop_bits"), "stop_bits")?,
        },
        "udp" => Endpoint::Udp {
            ip: get("ip").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: get("port")
                .ok_or("udp module requires 'port'")?
                .parse()
                .map_err(|_| "invalid 'port'")?,
        },
        "ascii" => Endpoint::Ascii {
            path: get("path").ok_or("ascii module requires 'path'")?,
            baud_rate: parse_opt(get("baud").or_else(|| get("baud_rate")), "baud")?
                .unwrap_or(19200),
            parity: get("parity"),
            data_bits: parse_opt(get("data_bits"), "data_bits")?,
            stop_bits: parse_opt(get("stop_bits"), "stop_bits")?,
        },
        "ascii_over_tcp" => Endpoint::AsciiOverTcp {
            ip: get("ip").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: get("port")
                .ok_or("ascii_over_tcp module requires 'port'")?
                .parse()
                .map_err(|_| "invalid 'port'")?,
        },
        other => {
            return Err(format!(
                "invalid transport '{other}' (expected tcp|rtu|rtu_over_tcp|udp|ascii|ascii_over_tcp)"
            ));
        }
    };

    Ok(ModuleSpec {
        name,
        device,
        role,
        endpoint,
    })
}

/// Parse a single `--ocpp` value (`key=val,key=val,...`) into an [`OcppModuleSpec`]. Mirrors
/// [`parse_module_spec`]; role/version/timeout/scripts live in the referenced device file, not
/// on the command line.
pub fn parse_ocpp_spec(input: &str) -> Result<OcppModuleSpec, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got '{part}'"))?;
        map.insert(key.trim().to_string(), value.trim().to_string());
    }

    let get = |k: &str| map.get(k).cloned();

    let name = get("name").ok_or("ocpp module requires 'name'")?;
    let device = get("device").ok_or("ocpp module requires 'device'")?;
    let ip = get("ip").unwrap_or_else(|| "127.0.0.1".to_string());
    let port = get("port")
        .ok_or("ocpp module requires 'port'")?
        .parse()
        .map_err(|_| "invalid 'port'")?;
    let path = get("path").unwrap_or_default();
    let protocol = match get("protocol").as_deref() {
        None | Some("ws") => OcppProtocol::Ws,
        Some("wss") => OcppProtocol::Wss,
        Some(other) => return Err(format!("invalid protocol '{other}' (expected ws|wss)")),
    };

    Ok(OcppModuleSpec {
        name,
        device,
        protocol,
        ip,
        port,
        path,
    })
}

/// Open `--log-file` (create-and-append, after `ferrowl_util::path::expand`), or `Ok(None)` when
/// no path was given. On failure, returns the original path alongside the I/O error so the
/// caller can print the exact `Error: failed to open --log-file '{path}': {e}` message and run
/// its own cleanup before choosing its exit code.
pub(crate) fn open_log_file(
    path: Option<&str>,
) -> Result<Option<std::fs::File>, (String, std::io::Error)> {
    match path {
        Some(path) => std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(ferrowl_util::path::expand(path))
            .map(Some)
            .map_err(|e| (path.to_string(), e)),
        None => Ok(None),
    }
}

fn parse_opt<T: std::str::FromStr>(
    value: Option<String>,
    field: &str,
) -> Result<Option<T>, String> {
    value
        .map(|v| v.parse::<T>().map_err(|_| format!("invalid '{field}'")))
        .transpose()
}

/// Parse a single `--upstream`/`--downstream` value (`key=val,key=val,...`) into a
/// [`ferrowl_modbus::bridge::BridgeEndpointSpec`] (BR-R-004). Mirrors [`parse_module_spec`].
/// `role` is the interface's fixed role (BR-R-005/BR-R-006: upstream is always `Server`,
/// downstream always `Client`), consulted only to pick which half of the TLS descriptor keys
/// (BR-R-011) applies.
pub fn parse_bridge_descriptor(
    input: &str,
    role: ClientOrServer,
) -> Result<ferrowl_modbus::bridge::BridgeEndpointSpec, String> {
    // A plain `,`-split would break `unit_ids=1,3,5-8` (BR-R-015's own list/range grammar
    // uses the same comma the descriptor uses between keys): a comma-separated segment with
    // no `=` is a continuation of the previous key's value, not a new key.
    let mut map: HashMap<String, String> = HashMap::new();
    let mut last_key: Option<String> = None;
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((key, value)) => {
                let key = key.trim().to_string();
                map.insert(key.clone(), value.trim().to_string());
                last_key = Some(key);
            }
            None => {
                let key = last_key
                    .as_ref()
                    .ok_or_else(|| format!("expected key=value, got '{part}'"))?;
                let entry = map
                    .get_mut(key)
                    .expect("last_key always names a present map entry");
                entry.push(',');
                entry.push_str(part);
            }
        }
    }
    let get = |k: &str| map.get(k).cloned();

    let unit_ids = get("unit_ids")
        .map(|s| ferrowl_modbus::bridge::UnitIdFilter::parse(&s))
        .transpose()?;

    let timeout_ms = parse_opt(get("timeout_ms"), "timeout_ms")?.unwrap_or(3000usize);
    let reconnect = match get("reconnect").as_deref() {
        None => true,
        Some("true") => true,
        Some("false") => false,
        Some(other) => {
            return Err(format!(
                "invalid 'reconnect' value '{other}' (expected true|false)"
            ));
        }
    };

    let transport = get("transport").unwrap_or_else(|| "tcp".to_string());
    let kind = match transport.as_str() {
        "tcp" => {
            let tls = build_descriptor_tls(&map, role)?;
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(ferrowl_modbus::tcp::Config {
                ip: get("ip").unwrap_or_else(|| "127.0.0.1".to_string()),
                port: get("port")
                    .ok_or("tcp descriptor requires 'port'")?
                    .parse()
                    .map_err(|_| "invalid 'port'")?,
                timeout_ms,
                delay_ms: 0,
                interval_ms: 0,
                reconnect,
                tls,
            })
        }
        "rtu_over_tcp" => {
            let tls = build_descriptor_tls(&map, role)?;
            ferrowl_modbus::bridge::BridgeEndpointKind::RtuOverTcp(ferrowl_modbus::tcp::Config {
                ip: get("ip").unwrap_or_else(|| "127.0.0.1".to_string()),
                port: get("port")
                    .ok_or("rtu_over_tcp descriptor requires 'port'")?
                    .parse()
                    .map_err(|_| "invalid 'port'")?,
                timeout_ms,
                delay_ms: 0,
                interval_ms: 0,
                reconnect,
                tls,
            })
        }
        "ascii_over_tcp" => {
            let tls = build_descriptor_tls(&map, role)?;
            ferrowl_modbus::bridge::BridgeEndpointKind::AsciiOverTcp(ferrowl_modbus::tcp::Config {
                ip: get("ip").unwrap_or_else(|| "127.0.0.1".to_string()),
                port: get("port")
                    .ok_or("ascii_over_tcp descriptor requires 'port'")?
                    .parse()
                    .map_err(|_| "invalid 'port'")?,
                timeout_ms,
                delay_ms: 0,
                interval_ms: 0,
                reconnect,
                tls,
            })
        }
        "rtu" => {
            // BR-R-011 — TLS is scoped to TCP-socket interfaces; a `tls.*` key here can only be
            // a mistake about which descriptor is being configured, so it is rejected outright
            // (including `tls.mode=none`, which would otherwise be a silent no-op).
            if let Some(key) = map.keys().find(|k| k.starts_with("tls.")) {
                return Err(format!(
                    "unrecognized descriptor key '{key}' (TLS is only available on tcp, rtu_over_tcp and ascii_over_tcp)"
                ));
            }
            ferrowl_modbus::bridge::BridgeEndpointKind::Rtu(ferrowl_modbus::rtu::Config {
                path: get("path").ok_or("rtu descriptor requires 'path'")?,
                baud_rate: parse_opt(get("baud").or_else(|| get("baud_rate")), "baud")?
                    .unwrap_or(19200),
                slave: 1, // BR-R-004/edge-cases.md — inert for bridge, same as an ordinary RTU server.
                parity: get("parity"),
                data_bits: parse_opt(get("data_bits"), "data_bits")?,
                stop_bits: parse_opt(get("stop_bits"), "stop_bits")?,
                timeout_ms,
                delay_ms: 0,
                interval_ms: 0,
                reconnect,
            })
        }
        other => {
            return Err(format!(
                "invalid transport '{other}' (expected tcp|rtu|rtu_over_tcp|ascii_over_tcp)"
            ));
        }
    };
    Ok(ferrowl_modbus::bridge::BridgeEndpointSpec { kind, unit_ids })
}

/// BR-R-011 — the dotted `tls.*` descriptor keys, mapped onto whichever tagged-enum policy
/// (MB-R-105) `role` selects: an upstream (`Server`) descriptor fills `ModbusTlsConfig.server`
/// and leaves `client` at its `None {}` default (BR-R-005), a downstream (`Client`) descriptor
/// the reverse (BR-R-006). The two CA-list keys split on `;`, `,` already being the descriptor's
/// key separator.
fn build_descriptor_tls(
    map: &HashMap<String, String>,
    role: ClientOrServer,
) -> Result<ferrowl_modbus::tcp::ModbusTlsConfig, String> {
    const IDENTITY_KEYS: [&str; 3] = [
        "tls.identity.source",
        "tls.identity.cert_file",
        "tls.identity.key_file",
    ];
    const VERIFICATION_KEYS: [&str; 3] = [
        "tls.verification.verify",
        "tls.verification.ca_files",
        "tls.verification.extra_ca_files",
    ];

    let get = |k: &str| map.get(k).cloned();

    // BR-R-011 — any `tls.*` key that is neither `tls.mode` nor a member of either variant's
    // own key set is unrecognized (a misspelling, or a key that names a path the selected
    // variant does not define under a different mode) and is a setup failure rather than being
    // silently ignored.
    for key in map.keys() {
        if key == "tls.mode" {
            continue;
        }
        if key.starts_with("tls.")
            && !IDENTITY_KEYS.contains(&key.as_str())
            && !VERIFICATION_KEYS.contains(&key.as_str())
        {
            return Err(format!("unrecognized descriptor key '{key}'"));
        }
    }

    let mode = get("tls.mode").unwrap_or_else(|| "none".to_string());
    let reject_present = |keys: &[&str]| -> Result<(), String> {
        for key in keys {
            if get(key).is_some() {
                return Err(format!("'{key}' is not valid with tls.mode={mode}"));
            }
        }
        Ok(())
    };
    let split_list = |value: String| -> Vec<String> {
        value
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };

    let identity = || -> Result<ferrowl_util::tls::CertSource, String> {
        match get("tls.identity.source").as_deref() {
            None | Some("ephemeral") => {
                if get("tls.identity.cert_file").is_some() || get("tls.identity.key_file").is_some()
                {
                    return Err("'tls.identity.cert_file'/'tls.identity.key_file' require \
                         tls.identity.source=files"
                        .to_string());
                }
                Ok(ferrowl_util::tls::CertSource::Ephemeral {})
            }
            Some("self-signed") => {
                if get("tls.identity.cert_file").is_some() || get("tls.identity.key_file").is_some()
                {
                    return Err("'tls.identity.cert_file'/'tls.identity.key_file' require \
                         tls.identity.source=files"
                        .to_string());
                }
                Ok(ferrowl_util::tls::CertSource::SelfSigned {})
            }
            Some("files") => {
                let cert_file = get("tls.identity.cert_file")
                    .ok_or("tls.identity.source=files requires 'tls.identity.cert_file'")?;
                let key_file = get("tls.identity.key_file")
                    .ok_or("tls.identity.source=files requires 'tls.identity.key_file'")?;
                Ok(ferrowl_util::tls::CertSource::Files {
                    cert_file,
                    key_file,
                })
            }
            Some(other) => Err(format!(
                "invalid 'tls.identity.source' value '{other}' (expected ephemeral|self-signed|files)"
            )),
        }
    };

    let verification = || -> Result<ferrowl_util::tls::CertVerification, String> {
        match get("tls.verification.verify").as_deref() {
            None | Some("root-store") => {
                if get("tls.verification.ca_files").is_some() {
                    return Err("'tls.verification.ca_files' is not valid with \
                         tls.verification.verify=root-store"
                        .to_string());
                }
                Ok(ferrowl_util::tls::CertVerification::RootStore {
                    extra_ca_files: get("tls.verification.extra_ca_files")
                        .map(split_list)
                        .unwrap_or_default(),
                })
            }
            Some("skip") => {
                if get("tls.verification.ca_files").is_some()
                    || get("tls.verification.extra_ca_files").is_some()
                {
                    return Err(
                        "'tls.verification.ca_files'/'tls.verification.extra_ca_files' are not \
                         valid with tls.verification.verify=skip"
                            .to_string(),
                    );
                }
                Ok(ferrowl_util::tls::CertVerification::Skip {})
            }
            Some("ca-files") => {
                if get("tls.verification.extra_ca_files").is_some() {
                    return Err("'tls.verification.extra_ca_files' is not valid with \
                         tls.verification.verify=ca-files"
                        .to_string());
                }
                Ok(ferrowl_util::tls::CertVerification::CaFiles {
                    ca_files: get("tls.verification.ca_files")
                        .map(split_list)
                        .unwrap_or_default(),
                })
            }
            Some(other) => Err(format!(
                "invalid 'tls.verification.verify' value '{other}' (expected skip|root-store|ca-files)"
            )),
        }
    };

    let mut tls = ferrowl_modbus::tcp::ModbusTlsConfig::default();
    match (role, mode.as_str()) {
        (_, "none") => {
            reject_present(&IDENTITY_KEYS)?;
            reject_present(&VERIFICATION_KEYS)?;
        }
        (ClientOrServer::Server, "tls") => {
            reject_present(&VERIFICATION_KEYS)?;
            tls.server = ferrowl_util::tls::ServerTlsPolicy::Tls {
                identity: identity()?,
            };
        }
        (ClientOrServer::Server, "mutual") => {
            tls.server = ferrowl_util::tls::ServerTlsPolicy::Mutual {
                identity: identity()?,
                verification: verification()?,
            };
        }
        (ClientOrServer::Client, "tls") => {
            reject_present(&IDENTITY_KEYS)?;
            tls.client = ferrowl_util::tls::ClientTlsPolicy::Tls {
                verification: verification()?,
            };
        }
        (ClientOrServer::Client, "mutual") => {
            tls.client = ferrowl_util::tls::ClientTlsPolicy::Mutual {
                verification: verification()?,
                identity: identity()?,
            };
        }
        (_, other) => {
            return Err(format!(
                "invalid 'tls.mode' value '{other}' (expected none|tls|mutual)"
            ));
        }
    }

    match role {
        ClientOrServer::Server => tls.server.validate().map_err(|e| e.to_string())?,
        ClientOrServer::Client => tls.client.validate().map_err(|e| e.to_string())?,
    }

    Ok(tls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_test_support::reserve_temp_dir;

    #[test]
    /// CL-R-002 — a --module TCP descriptor parses into a module instance.
    fn ut_parse_tcp_module() {
        let spec = parse_module_spec(
            "name=evse-1,device=configs/evse.toml,transport=tcp,ip=10.0.0.5,port=0,role=server",
        )
        .unwrap();
        assert_eq!(spec.name, "evse-1");
        assert_eq!(spec.device, "configs/evse.toml");
        assert_eq!(spec.role, Role::Server);
        assert_eq!(
            spec.endpoint,
            Endpoint::Tcp {
                ip: "10.0.0.5".into(),
                port: 0
            }
        );
    }

    #[test]
    /// CL-R-002 — a --module TCP descriptor applies its documented defaults.
    fn ut_parse_tcp_defaults() {
        // ip and role default; type is an alias for device.
        let spec = parse_module_spec("name=m,type=d.toml,port=1502").unwrap();
        assert_eq!(spec.role, Role::Server);
        assert_eq!(
            spec.endpoint,
            Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 1502
            }
        );
    }

    #[test]
    /// CL-R-002 — a --module RTU descriptor parses into a module instance.
    fn ut_parse_rtu_module() {
        let spec = parse_module_spec(
            "name=m,device=d.toml,transport=rtu,path=/dev/ttyUSB0,baud=9600,role=client",
        )
        .unwrap();
        assert_eq!(spec.role, Role::Client);
        assert_eq!(
            spec.endpoint,
            Endpoint::Rtu {
                path: "/dev/ttyUSB0".into(),
                baud_rate: 9600,
                parity: None,
                data_bits: None,
                stop_bits: None,
            }
        );
    }

    #[test]
    /// CL-R-002 — a --module RtuOverTcp descriptor parses like TCP (same ip/port
    /// keys), tagged `rtu_over_tcp`.
    fn ut_parse_rtu_over_tcp_module() {
        let spec = parse_module_spec(
            "name=m,device=d.toml,transport=rtu_over_tcp,ip=10.0.0.5,port=0,role=client",
        )
        .unwrap();
        assert_eq!(spec.role, Role::Client);
        assert_eq!(
            spec.endpoint,
            Endpoint::RtuOverTcp {
                ip: "10.0.0.5".into(),
                port: 0
            }
        );
    }

    #[test]
    /// CL-R-002 — an unknown `transport` value is still a parse error, listing all
    /// six valid options.
    fn ut_parse_invalid_transport_still_errors() {
        assert!(parse_module_spec("name=m,device=d,transport=usb").is_err());
    }

    #[test]
    /// CL-R-002 — a --module Ascii descriptor parses like RTU (same path/baud/parity/
    /// data_bits/stop_bits keys), tagged `ascii`.
    fn ut_parse_ascii_module() {
        let spec = parse_module_spec(
            "name=m,device=d.toml,transport=ascii,path=/dev/ttyUSB0,baud=9600,role=client",
        )
        .unwrap();
        assert_eq!(spec.role, Role::Client);
        assert_eq!(
            spec.endpoint,
            Endpoint::Ascii {
                path: "/dev/ttyUSB0".into(),
                baud_rate: 9600,
                parity: None,
                data_bits: None,
                stop_bits: None
            }
        );
    }

    #[test]
    /// CL-R-002 — a --module AsciiOverTcp descriptor parses like TCP (same ip/port keys),
    /// tagged `ascii_over_tcp`.
    fn ut_parse_ascii_over_tcp_module() {
        let spec = parse_module_spec(
            "name=m,device=d.toml,transport=ascii_over_tcp,ip=10.0.0.5,port=0,role=client",
        )
        .unwrap();
        assert_eq!(spec.role, Role::Client);
        assert_eq!(
            spec.endpoint,
            Endpoint::AsciiOverTcp {
                ip: "10.0.0.5".into(),
                port: 0
            }
        );
    }

    #[test]
    /// MB-R-116 — `transport=udp` parses for either role (no restriction this run).
    fn ut_parse_module_spec_udp_ok() {
        let spec =
            parse_module_spec("name=m,device=d.toml,transport=udp,ip=127.0.0.1,port=0").unwrap();
        assert_eq!(
            spec.endpoint,
            Endpoint::Udp {
                ip: "127.0.0.1".into(),
                port: 0
            }
        );

        let client_spec =
            parse_module_spec("name=m,device=d.toml,role=client,transport=udp,ip=127.0.0.1,port=0")
                .unwrap();
        assert_eq!(client_spec.role, Role::Client);
        assert!(matches!(client_spec.endpoint, Endpoint::Udp { .. }));
    }

    #[test]
    /// CL-R-044 — a --device occurrence auto-builds a TCP client with the fixed endpoint and default name.
    fn ut_device_spec_defaults() {
        let spec = create_module_spec_by_device("Device 0".to_string(), "d.toml".to_string());
        assert_eq!(spec.name, "Device 0");
        assert_eq!(spec.device, "d.toml");
        assert_eq!(spec.role, Role::Client);
        assert_eq!(
            spec.endpoint,
            Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 5020
            }
        );
    }

    #[test]
    /// CL-R-002 — a --module descriptor missing its name is a parse error.
    fn ut_missing_name_errors() {
        assert!(parse_module_spec("device=d.toml,port=0").is_err());
    }

    #[test]
    /// CL-R-002 — a --module descriptor missing its port is a parse error.
    fn ut_missing_port_errors() {
        assert!(parse_module_spec("name=m,device=d.toml,transport=tcp").is_err());
    }

    #[test]
    /// CL-R-007 — the instance set concatenates session, --module, then --device sources in order.
    fn ut_module_specs_combines_all_sources() {
        use ferrowl_util::convert::{Converter, FileType};
        let session = config::Session {
            version: None,
            modules: vec![
                serde_json::to_value(create_module_spec_by_device("S".into(), "s.toml".into()))
                    .unwrap(),
            ],
            scripts: vec![],
            interval: 1.0,
        };
        let dir = reserve_temp_dir("ferrowl_cli");
        let path = dir.join("session.toml");
        let path = path.to_str().unwrap().to_string();
        Converter::save(&session, &path, FileType::Toml).unwrap();

        let args = CliArgs {
            command: None,
            modules: vec!["name=m,device=d.toml,port=1".into()],
            sessions: vec![path],
            devices: vec!["dev.toml".into()],
            demo: false,
        };
        let specs = args.module_specs().unwrap();
        assert_eq!(specs.len(), 3); // session + module + device
    }

    #[test]
    /// CL-R-054 — session instances resolve before --module instances.
    fn ut_session_instances_resolve_before_module_instances() {
        use ferrowl_util::convert::{Converter, FileType};
        let session = config::Session {
            version: None,
            modules: vec![
                serde_json::to_value(create_module_spec_by_device(
                    "from_session".into(),
                    "s.toml".into(),
                ))
                .unwrap(),
            ],
            scripts: vec![],
            interval: 1.0,
        };
        let dir = reserve_temp_dir("ferrowl_cli");
        let path = dir.join("session.toml");
        let path = path.to_str().unwrap().to_string();
        Converter::save(&session, &path, FileType::Toml).unwrap();

        let args = CliArgs {
            command: None,
            modules: vec!["name=from_module,device=d.toml,port=1".into()],
            sessions: vec![path],
            devices: vec![],
            demo: false,
        };
        let specs = args.module_specs().unwrap();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["from_session", "from_module"]);
    }

    #[test]
    /// CL-R-003, CL-R-053 — a --session file's instances resolve, split into modbus and ocpp.
    fn ut_session_splits_modbus_and_ocpp() {
        use ferrowl_util::convert::{Converter, FileType};
        let mut modbus =
            serde_json::to_value(create_module_spec_by_device("mb".into(), "s.toml".into()))
                .unwrap();
        modbus
            .as_object_mut()
            .unwrap()
            .insert("type".into(), "modbus".into());
        let mut ocpp = serde_json::to_value(OcppModuleSpec {
            name: "cs".into(),
            device: "cs.toml".into(),
            protocol: config::ocpp::OcppProtocol::Ws,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
        })
        .unwrap();
        ocpp.as_object_mut()
            .unwrap()
            .insert("type".into(), "ocpp".into());

        let session = config::Session {
            version: None,
            modules: vec![modbus, ocpp],
            scripts: vec![],
            interval: 1.0,
        };
        let dir = reserve_temp_dir("ferrowl_cli");
        let path = dir.join("mixed_session.json");
        let path = path.to_str().unwrap().to_string();
        Converter::save(&session, &path, FileType::Json).unwrap();

        let args = CliArgs {
            command: None,
            modules: vec![],
            sessions: vec![path],
            devices: vec![],
            demo: false,
        };
        // Modbus loader sees only the modbus module; OCPP loader sees only the ocpp module.
        assert_eq!(args.module_specs().unwrap().len(), 1);
        let ocpp = args.ocpp_specs().unwrap();
        assert_eq!(ocpp.len(), 1);
        assert_eq!(ocpp[0].name, "cs");
        assert_eq!(ocpp[0].device, "cs.toml");
        assert_eq!(ocpp[0].port, 9000);
    }

    #[test]
    /// A --session file that fails to load surfaces an error during resolution.
    fn ut_module_specs_session_load_error() {
        let args = CliArgs {
            command: None,
            modules: vec![],
            sessions: vec!["/no/such/ferrowl.toml".into()],
            devices: vec![],
            demo: false,
        };
        assert!(args.module_specs().is_err());
    }

    #[test]
    /// CL-R-002 — the descriptor mini-language rejects empty parts and malformed values.
    fn ut_parse_empty_parts_and_error_paths() {
        // Empty comma segment is skipped.
        assert_eq!(
            parse_module_spec("name=m,,device=d.toml,port=1")
                .unwrap()
                .name,
            "m"
        );
        // Invalid role / transport.
        assert!(parse_module_spec("name=m,device=d,port=1,role=bogus").is_err());
        assert!(parse_module_spec("name=m,device=d,transport=usb").is_err());
        // Segment without '='.
        assert!(parse_module_spec("name=m,oops,device=d,port=1").is_err());
        // RTU missing path; invalid numeric option; invalid port.
        assert!(parse_module_spec("name=m,device=d,transport=rtu").is_err());
        assert!(parse_module_spec("name=m,device=d,transport=rtu,path=/x,data_bits=foo").is_err());
        assert!(parse_module_spec("name=m,device=d,port=notanum").is_err());
    }

    #[test]
    /// CL-R-002 — a --module RTU descriptor parses its full option set.
    fn ut_parse_rtu_full_options() {
        let spec = parse_module_spec(
            "name=m,device=d,transport=rtu,path=/dev/x,baud_rate=4800,parity=even,data_bits=7,stop_bits=2",
        )
        .unwrap();
        assert_eq!(
            spec.endpoint,
            Endpoint::Rtu {
                path: "/dev/x".into(),
                baud_rate: 4800,
                parity: Some("even".into()),
                data_bits: Some(7),
                stop_bits: Some(2),
            }
        );
    }

    #[test]
    /// CL-R-013 — a run --ocpp descriptor applies its documented defaults.
    fn ut_parse_ocpp_spec_defaults() {
        let spec = parse_ocpp_spec("name=cs-1,device=cs.toml,port=9000").unwrap();
        assert_eq!(spec.name, "cs-1");
        assert_eq!(spec.device, "cs.toml");
        assert_eq!(spec.ip, "127.0.0.1");
        assert_eq!(spec.port, 9000);
        assert_eq!(spec.path, "");
        assert_eq!(spec.protocol, config::ocpp::OcppProtocol::Ws);
    }

    #[test]
    /// CL-R-013 — a run --ocpp descriptor parses its full option set.
    fn ut_parse_ocpp_spec_full() {
        let spec = parse_ocpp_spec(
            "name=cs-1,device=cs.toml,protocol=wss,ip=10.0.0.5,port=9001,path=/ocpp/cp001",
        )
        .unwrap();
        assert_eq!(spec.protocol, config::ocpp::OcppProtocol::Wss);
        assert_eq!(spec.ip, "10.0.0.5");
        assert_eq!(spec.path, "/ocpp/cp001");
    }

    #[test]
    /// OC-R-119 — extra_headers is not a --ocpp key=value field; an unrecognized key is silently
    /// inert, same as config/connectors.
    fn ut_parse_ocpp_spec_ignores_extra_headers_key() {
        let spec = parse_ocpp_spec("name=cs1,device=d.toml,port=9000,extra_headers=X-Tenant:acme")
            .unwrap();
        assert_eq!(spec.name, "cs1");
        assert_eq!(spec.device, "d.toml");
        assert_eq!(spec.port, 9000);
    }

    #[test]
    /// CL-R-013 — a malformed run --ocpp descriptor is a parse error.
    fn ut_parse_ocpp_spec_errors() {
        assert!(parse_ocpp_spec("device=d,port=1").is_err()); // missing name
        assert!(parse_ocpp_spec("name=m,port=1").is_err()); // missing device
        assert!(parse_ocpp_spec("name=m,device=d").is_err()); // missing port
        assert!(parse_ocpp_spec("name=m,device=d,port=notanum").is_err());
        assert!(parse_ocpp_spec("name=m,device=d,port=1,protocol=bogus").is_err());
    }

    #[test]
    /// CL-R-013 — the run subcommand parses its own flag set.
    fn ut_run_subcommand_parses() {
        let args = CliArgs::parse_from([
            "ferrowl",
            "run",
            "--module",
            "name=m,device=d.toml,port=1502",
            "--ocpp",
            "name=cs,device=cs.toml,port=9000",
            "--duration",
            "5",
            "--log-file",
            "out.log",
            "--exit-on-error",
        ]);
        match args.command {
            Some(SubCommand::Run(run)) => {
                assert_eq!(
                    run.modules,
                    vec!["name=m,device=d.toml,port=1502".to_string()]
                );
                assert_eq!(
                    run.ocpp,
                    vec!["name=cs,device=cs.toml,port=9000".to_string()]
                );
                assert_eq!(run.duration, Some(5));
                assert_eq!(run.log_file.as_deref(), Some("out.log"));
                assert!(run.exit_on_error);
            }
            _ => panic!("expected SubCommand::Run"),
        }
    }

    #[test]
    /// CL-R-046 — run resolves modbus from --session/--module and ocpp from --session/--ocpp.
    fn ut_run_args_resolve_module_and_ocpp_specs() {
        let run = RunArgs {
            sessions: vec![],
            modules: vec!["name=m,device=d.toml,port=1502".into()],
            ocpp: vec!["name=cs,device=cs.toml,port=9000".into()],
            duration: None,
            log_file: None,
            exit_on_error: false,
        };
        let specs = run.module_specs().unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "m");
        let ocpp = run.ocpp_specs().unwrap();
        assert_eq!(ocpp.len(), 1);
        assert_eq!(ocpp[0].name, "cs");
        assert_eq!(ocpp[0].port, 9000);
    }

    // --- Argument surface: version/help, subcommands, flag scoping, parse errors ----------

    #[test]
    /// CL-R-001, CL-R-051 — --version and --help print and exit 0, taking precedence over starting the TUI.
    fn ut_version_and_help_exit_zero() {
        use clap::error::ErrorKind;
        let v = CliArgs::try_parse_from(["ferrowl", "--version"]).unwrap_err();
        assert_eq!(v.kind(), ErrorKind::DisplayVersion);
        assert_eq!(v.exit_code(), 0);
        let h = CliArgs::try_parse_from(["ferrowl", "--help"]).unwrap_err();
        assert_eq!(h.kind(), ErrorKind::DisplayHelp);
        assert_eq!(h.exit_code(), 0);
    }

    #[test]
    /// CL-R-010 — the two subcommands dispatch, replacing the default (start-the-TUI) action.
    fn ut_subcommands_dispatch() {
        assert!(matches!(
            CliArgs::parse_from(["ferrowl", "run"]).command,
            Some(SubCommand::Run(_))
        ));
        assert!(matches!(
            CliArgs::parse_from(["ferrowl", "migrate", "-i", "a.toml", "-o", "b.toml"]).command,
            Some(SubCommand::Migrate(_))
        ));
        // No subcommand → default action (the TUI); `command` stays None.
        assert!(CliArgs::parse_from(["ferrowl"]).command.is_none());
    }

    #[test]
    /// CL-R-011 — the migrate subcommand requires both --input and --output.
    fn ut_migrate_requires_input_and_output() {
        assert!(CliArgs::try_parse_from(["ferrowl", "migrate"]).is_err());
        assert!(CliArgs::try_parse_from(["ferrowl", "migrate", "-i", "a.toml"]).is_err());
        assert!(CliArgs::try_parse_from(["ferrowl", "migrate", "-o", "b.toml"]).is_err());
        match CliArgs::parse_from(["ferrowl", "migrate", "-i", "a.toml", "-o", "b.toml"]).command {
            Some(SubCommand::Migrate(m)) => {
                assert_eq!(m.input, "a.toml");
                assert_eq!(m.output, "b.toml");
            }
            _ => panic!("expected migrate"),
        }
    }

    #[test]
    /// CL-R-014, CL-R-047 — --ocpp is accepted only on run; --device only on the top-level command.
    fn ut_ocpp_and_device_flag_scoping() {
        assert!(CliArgs::try_parse_from(["ferrowl", "--ocpp", "name=cs,device=d,port=1"]).is_err());
        assert!(
            CliArgs::try_parse_from(["ferrowl", "run", "--ocpp", "name=cs,device=d,port=1"])
                .is_ok()
        );
        assert!(CliArgs::try_parse_from(["ferrowl", "run", "--device", "d.toml"]).is_err());
        assert!(CliArgs::try_parse_from(["ferrowl", "--device", "d.toml"]).is_ok());
    }

    #[test]
    /// CL-R-015 — --exit-on-error exists only on the run subcommand.
    fn ut_exit_on_error_is_run_only() {
        assert!(CliArgs::try_parse_from(["ferrowl", "--exit-on-error"]).is_err());
        assert!(CliArgs::try_parse_from(["ferrowl", "run", "--exit-on-error"]).is_ok());
    }

    #[test]
    /// CL-R-016 — top-level values supplied alongside a run subcommand do not reach the runner.
    fn ut_run_ignores_top_level_values() {
        let args = CliArgs::parse_from(["ferrowl", "--module", "name=m,device=d,port=1", "run"]);
        assert_eq!(args.modules, vec!["name=m,device=d,port=1".to_string()]);
        match args.command {
            Some(SubCommand::Run(run)) => {
                assert!(
                    run.modules.is_empty(),
                    "run must not see the top-level --module"
                );
            }
            _ => panic!("expected run"),
        }
    }

    #[test]
    /// CL-R-035 — an argument-parsing error exits with the parser's usage exit code (2).
    fn ut_arg_parse_error_exits_two() {
        use clap::error::ErrorKind;
        let e = CliArgs::try_parse_from(["ferrowl", "--bogus-flag"]).unwrap_err();
        assert_ne!(e.kind(), ErrorKind::DisplayHelp);
        assert_eq!(e.exit_code(), 2);
    }

    // --- Config envelope: module-entry type dispatch and required fields ------------------

    /// Save `modules` as a session file and resolve it through [`CliArgs::module_specs`].
    fn resolve_session(
        tag: &str,
        modules: Vec<serde_json::Value>,
    ) -> Result<Vec<ModuleSpec>, String> {
        use ferrowl_util::convert::{Converter, FileType};
        let session = config::Session {
            version: None,
            modules,
            scripts: vec![],
            interval: 1.0,
        };
        let dir = reserve_temp_dir("ferrowl_cli");
        let path = dir.join(format!("{tag}.toml"));
        Converter::save(&session, path.to_str().unwrap(), FileType::Toml).unwrap();
        let args = CliArgs {
            command: None,
            modules: vec![],
            sessions: vec![path.to_str().unwrap().to_string()],
            devices: vec![],
            demo: false,
        };
        args.module_specs()
    }

    #[test]
    /// CS-R-013 — a module entry whose `type` is neither modbus nor ocpp aborts session resolution.
    fn ut_unknown_module_type_aborts_resolution() {
        let err = resolve_session(
            "unknown_type",
            vec![serde_json::json!({"type": "plc", "name": "x"})],
        )
        .unwrap_err();
        assert!(
            err.contains("unsupported module type"),
            "expected a hard error, got: {err}"
        );
    }

    #[test]
    /// CS-R-051 — a module instance missing a schema-required field fails to load.
    fn ut_module_missing_required_field_errors() {
        let mut module =
            serde_json::to_value(create_module_spec_by_device("x".into(), "d.toml".into()))
                .unwrap();
        // Drop the required `name`, keep the modbus tag.
        module.as_object_mut().unwrap().remove("name");
        module
            .as_object_mut()
            .unwrap()
            .insert("type".into(), "modbus".into());
        assert!(resolve_session("missing_name", vec![module]).is_err());
    }

    // --- Bridge mode: SubCommand::Bridge and descriptor parsing ---------------------------

    #[test]
    /// BR-R-001, BR-R-003, BR-R-014 — the bridge subcommand parses upstream/downstream/
    /// duration flags.
    fn ut_bridge_subcommand_parses() {
        let args = CliArgs::parse_from([
            "ferrowl",
            "bridge",
            "--upstream",
            "transport=tcp,ip=0.0.0.0,port=0",
            "--downstream",
            "transport=tcp,ip=10.0.0.5,port=0",
            "--duration",
            "5",
        ]);
        match args.command {
            Some(SubCommand::Bridge(bridge)) => {
                assert_eq!(
                    bridge.upstream.as_deref(),
                    Some("transport=tcp,ip=0.0.0.0,port=0")
                );
                assert_eq!(
                    bridge.downstream.as_deref(),
                    Some("transport=tcp,ip=10.0.0.5,port=0")
                );
                assert_eq!(bridge.duration, Some(5));
            }
            _ => panic!("expected SubCommand::Bridge"),
        }
    }

    #[test]
    /// BR-R-003 — clap accepts `bridge` with neither --upstream nor --downstream; the
    /// required-endpoint check happens at runtime, not at the clap layer.
    fn ut_bridge_upstream_and_downstream_both_optional_at_clap_layer() {
        assert!(CliArgs::try_parse_from(["ferrowl", "bridge"]).is_ok());
    }

    #[test]
    /// BR-R-004 — a bare `port=0` descriptor defaults transport/ip/timeout/reconnect/tls/
    /// unit_ids.
    fn ut_parse_bridge_descriptor_tcp_defaults() {
        let spec = parse_bridge_descriptor("port=0", ClientOrServer::Server).unwrap();
        assert!(spec.unit_ids.is_none());
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => {
                assert_eq!(cfg.ip, "127.0.0.1");
                assert_eq!(cfg.port, 0);
                assert_eq!(cfg.timeout_ms, 3000);
                assert!(cfg.reconnect);
                assert!(cfg.tls.is_none());
            }
            _ => panic!("expected Tcp"),
        }
    }

    #[test]
    /// BR-R-017 — every rtu_over_tcp descriptor key round-trips, reusing the same field set
    /// (and defaults) as plain tcp.
    fn ut_parse_bridge_descriptor_rtu_over_tcp_full() {
        let spec = parse_bridge_descriptor(
            "transport=rtu_over_tcp,ip=10.0.0.9,port=1502,timeout_ms=500,reconnect=false",
            ClientOrServer::Server,
        )
        .unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::RtuOverTcp(cfg) => {
                assert_eq!(cfg.ip, "10.0.0.9");
                assert_eq!(cfg.port, 1502);
                assert_eq!(cfg.timeout_ms, 500);
                assert!(!cfg.reconnect);
            }
            _ => panic!("expected RtuOverTcp"),
        }
    }

    #[test]
    /// BR-R-017 — a rtu_over_tcp descriptor missing `port` errors, same as plain tcp.
    fn ut_parse_bridge_descriptor_rtu_over_tcp_requires_port() {
        assert!(parse_bridge_descriptor("transport=rtu_over_tcp", ClientOrServer::Server).is_err());
    }

    #[test]
    /// BR-R-017 — every ascii_over_tcp descriptor key round-trips, reusing the same field set
    /// (and defaults) as plain tcp.
    fn ut_parse_bridge_descriptor_ascii_over_tcp_full() {
        let spec = parse_bridge_descriptor(
            "transport=ascii_over_tcp,ip=10.0.0.10,port=1503,timeout_ms=500,reconnect=false",
            ClientOrServer::Server,
        )
        .unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::AsciiOverTcp(cfg) => {
                assert_eq!(cfg.ip, "10.0.0.10");
                assert_eq!(cfg.port, 1503);
                assert_eq!(cfg.timeout_ms, 500);
                assert!(!cfg.reconnect);
            }
            _ => panic!("expected AsciiOverTcp"),
        }
    }

    #[test]
    /// BR-R-017 — an ascii_over_tcp descriptor missing `port` errors, same as plain tcp.
    fn ut_parse_bridge_descriptor_ascii_over_tcp_requires_port() {
        assert!(
            parse_bridge_descriptor("transport=ascii_over_tcp", ClientOrServer::Server).is_err()
        );
    }

    #[test]
    /// BR-R-011 — rtu_over_tcp/ascii_over_tcp descriptors accept the same dotted `tls.*` key
    /// set as plain tcp (BR-R-004's shared tcp::Config field set).
    fn ut_parse_bridge_descriptor_over_tcp_variants_accept_tls() {
        for transport in ["rtu_over_tcp", "ascii_over_tcp"] {
            let spec = parse_bridge_descriptor(
                &format!(
                    "transport={transport},port=0,tls.mode=tls,tls.identity.source=self-signed"
                ),
                ClientOrServer::Server,
            )
            .unwrap();
            let tls = match spec.kind {
                ferrowl_modbus::bridge::BridgeEndpointKind::RtuOverTcp(cfg) => cfg.tls,
                ferrowl_modbus::bridge::BridgeEndpointKind::AsciiOverTcp(cfg) => cfg.tls,
                _ => panic!("expected an over-tcp variant"),
            };
            assert!(!tls.is_none(), "{transport} must accept tls fields");
        }
    }

    #[test]
    /// BR-R-004, BR-R-015 — every rtu descriptor key round-trips.
    fn ut_parse_bridge_descriptor_rtu_full() {
        let spec = parse_bridge_descriptor(
            "transport=rtu,path=/dev/ttyUSB0,baud=9600,parity=even,data_bits=7,stop_bits=2,\
             timeout_ms=500,reconnect=false,unit_ids=1,3",
            ClientOrServer::Server,
        )
        .unwrap();
        assert_eq!(
            spec.unit_ids,
            Some(ferrowl_modbus::bridge::UnitIdFilter::parse("1,3").unwrap())
        );
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Rtu(cfg) => {
                assert_eq!(cfg.path, "/dev/ttyUSB0");
                assert_eq!(cfg.baud_rate, 9600);
                assert_eq!(cfg.parity.as_deref(), Some("even"));
                assert_eq!(cfg.data_bits, Some(7));
                assert_eq!(cfg.stop_bits, Some(2));
                assert_eq!(cfg.timeout_ms, 500);
                assert!(!cfg.reconnect);
            }
            _ => panic!("expected Rtu"),
        }
    }

    #[test]
    /// BR-R-020 — absent `tls.mode` defaults to `none`, leaving `tls` both-`None`.
    fn ut_bridge_descriptor_absent_tls_mode_defaults_to_none() {
        let spec = parse_bridge_descriptor("port=0", ClientOrServer::Server).unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => assert!(cfg.tls.is_none()),
            _ => unreachable!(),
        }
    }

    #[test]
    /// BR-R-011 — an upstream (`Server`) descriptor's `tls.*` keys build a `ServerTlsPolicy`,
    /// leaving `client` at its `None {}` default.
    fn ut_bridge_descriptor_upstream_server_policy_from_dotted_keys() {
        let spec = parse_bridge_descriptor(
            "port=0,tls.mode=mutual,tls.identity.source=files,\
             tls.identity.cert_file=s.crt,tls.identity.key_file=s.key,\
             tls.verification.verify=ca-files,tls.verification.ca_files=ca.pem",
            ClientOrServer::Server,
        )
        .unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => {
                assert_eq!(
                    cfg.tls.server,
                    ferrowl_util::tls::ServerTlsPolicy::Mutual {
                        identity: ferrowl_util::tls::CertSource::Files {
                            cert_file: "s.crt".to_string(),
                            key_file: "s.key".to_string(),
                        },
                        verification: ferrowl_util::tls::CertVerification::CaFiles {
                            ca_files: vec!["ca.pem".to_string()]
                        },
                    }
                );
                assert_eq!(cfg.tls.client, ferrowl_util::tls::ClientTlsPolicy::None {});
            }
            _ => unreachable!(),
        }
    }

    #[test]
    /// BR-R-011 — a downstream (`Client`) descriptor's `tls.*` keys build a `ClientTlsPolicy`,
    /// leaving `server` at its `None {}` default.
    fn ut_bridge_descriptor_downstream_client_policy_from_dotted_keys() {
        let spec = parse_bridge_descriptor(
            "port=0,tls.mode=tls,tls.verification.verify=skip",
            ClientOrServer::Client,
        )
        .unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => {
                assert_eq!(
                    cfg.tls.client,
                    ferrowl_util::tls::ClientTlsPolicy::Tls {
                        verification: ferrowl_util::tls::CertVerification::Skip {}
                    }
                );
                assert_eq!(cfg.tls.server, ferrowl_util::tls::ServerTlsPolicy::None {});
            }
            _ => unreachable!(),
        }
    }

    #[test]
    /// BR-R-020 — `tls.verification.extra_ca_files` (under `verify=root-store`) splits on `;`.
    fn ut_bridge_descriptor_ca_files_split_on_semicolon() {
        let spec = parse_bridge_descriptor(
            "port=0,tls.mode=tls,tls.verification.verify=root-store,\
             tls.verification.extra_ca_files=a.pem;b.pem",
            ClientOrServer::Client,
        )
        .unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => {
                assert_eq!(
                    cfg.tls.client,
                    ferrowl_util::tls::ClientTlsPolicy::Tls {
                        verification: ferrowl_util::tls::CertVerification::RootStore {
                            extra_ca_files: vec!["a.pem".to_string(), "b.pem".to_string()]
                        }
                    }
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    /// BR-R-020 — `tls.verification.ca_files` (under `verify=ca-files`) also splits on `;`.
    fn ut_bridge_descriptor_ca_files_under_verify_ca_files_split_on_semicolon() {
        let spec = parse_bridge_descriptor(
            "port=0,tls.mode=tls,tls.verification.verify=ca-files,\
             tls.verification.ca_files=a.pem;b.pem",
            ClientOrServer::Client,
        )
        .unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => {
                assert_eq!(
                    cfg.tls.client,
                    ferrowl_util::tls::ClientTlsPolicy::Tls {
                        verification: ferrowl_util::tls::CertVerification::CaFiles {
                            ca_files: vec!["a.pem".to_string(), "b.pem".to_string()]
                        }
                    }
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    /// BR-R-020 — `tls.verification.ca_files` under `verify=root-store` (the ca-files key
    /// belongs to `verify=ca-files`, not `root-store`) is a setup failure.
    fn ut_bridge_descriptor_rejects_ca_files_under_verify_root_store() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.verification.verify=root-store,\
                 tls.verification.ca_files=ca.pem",
                ClientOrServer::Client
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-020 — a `tls.*` key outside the selected variant is a setup failure: `tls.identity.*`
    /// under `tls.mode=none`, and `tls.verification.*` under a server's `tls.mode=tls` (a server's
    /// `Tls` variant has no `verification` field — `Mutual` is the sole trigger).
    fn ut_bridge_descriptor_rejects_key_outside_selected_variant() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.identity.source=self-signed",
                ClientOrServer::Server
            )
            .is_err()
        );
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.identity.source=self-signed,\
                 tls.verification.verify=skip",
                ClientOrServer::Server
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-020 — a `tls.*` key not among `tls.mode` or the selected variant's own keys (a
    /// misspelling like `tls.identiy.source`, or an invented one like `tls.verify`) is a setup
    /// failure rather than being silently ignored.
    fn ut_bridge_descriptor_rejects_unrecognized_tls_key() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.identiy.source=self-signed",
                ClientOrServer::Server
            )
            .is_err()
        );
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.verify=skip",
                ClientOrServer::Client
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-020 — an unrecognized value on `tls.mode`, `tls.identity.source`, or
    /// `tls.verification.verify` is a setup failure.
    fn ut_bridge_descriptor_rejects_unknown_enum_values() {
        assert!(parse_bridge_descriptor("port=0,tls.mode=bogus", ClientOrServer::Server).is_err());
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.identity.source=bogus",
                ClientOrServer::Server
            )
            .is_err()
        );
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.verification.verify=bogus",
                ClientOrServer::Client
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-024, MB-R-107 — `tls.identity.source=files` with only `tls.identity.cert_file` set
    /// (no `tls.identity.key_file`) is a setup failure.
    fn ut_bridge_descriptor_rejects_files_identity_with_only_cert_file() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.identity.source=files,tls.identity.cert_file=s.crt",
                ClientOrServer::Server
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-022 — `tls.verification.verify=root-store` on an upstream (`Server`) descriptor is
    /// rejected: `RootStore` is client-only, never a server's client-certificate verification.
    fn ut_bridge_descriptor_rejects_root_store_upstream() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=mutual,tls.identity.source=self-signed,\
                 tls.verification.verify=root-store",
                ClientOrServer::Server
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-028 — `tls.identity.source=ephemeral` on a downstream (`Client`) descriptor is
    /// rejected: "nothing configured, fall back and log" is a server-side behavior.
    fn ut_bridge_descriptor_rejects_ephemeral_downstream() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=mutual,tls.verification.verify=skip,\
                 tls.identity.source=ephemeral",
                ClientOrServer::Client
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-021, BR-R-030 — a bare upstream `tls.mode=tls` (no `tls.identity.source`) accepts the
    /// defaulted `ephemeral` identity.
    fn ut_bridge_descriptor_upstream_bare_tls_mode_defaults_to_ephemeral_identity() {
        let spec = parse_bridge_descriptor("port=0,tls.mode=tls", ClientOrServer::Server).unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => {
                assert_eq!(
                    cfg.tls.server,
                    ferrowl_util::tls::ServerTlsPolicy::Tls {
                        identity: ferrowl_util::tls::CertSource::Ephemeral {}
                    }
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    /// BR-R-021, BR-R-030 — a bare downstream `tls.mode=tls` (no `tls.verification.verify`) accepts the
    /// defaulted `root-store` verification with an empty `extra_ca_files`.
    fn ut_bridge_descriptor_downstream_bare_tls_mode_defaults_to_root_store_verification() {
        let spec = parse_bridge_descriptor("port=0,tls.mode=tls", ClientOrServer::Client).unwrap();
        match spec.kind {
            ferrowl_modbus::bridge::BridgeEndpointKind::Tcp(cfg) => {
                assert_eq!(
                    cfg.tls.client,
                    ferrowl_util::tls::ClientTlsPolicy::Tls {
                        verification: ferrowl_util::tls::CertVerification::RootStore {
                            extra_ca_files: vec![]
                        }
                    }
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    /// BR-R-029 — an upstream `tls.mode=mutual` with no `tls.verification.verify` defaults the
    /// verification to `root-store`, which is a setup failure on a server exactly as if it had
    /// been written explicitly.
    fn ut_bridge_descriptor_rejects_upstream_mutual_with_defaulted_root_store() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=mutual,tls.identity.source=self-signed",
                ClientOrServer::Server
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-029 — a downstream `tls.mode=mutual` with no `tls.identity.source` defaults the
    /// identity to `ephemeral`, which is a setup failure on a client exactly as if it had been
    /// written explicitly.
    fn ut_bridge_descriptor_rejects_downstream_mutual_with_defaulted_ephemeral() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=mutual,tls.verification.verify=skip",
                ClientOrServer::Client
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-024 — `tls.verification.verify=ca-files` with an empty `tls.verification.ca_files`
    /// is rejected.
    fn ut_bridge_descriptor_rejects_empty_ca_files() {
        assert!(
            parse_bridge_descriptor(
                "port=0,tls.mode=tls,tls.verification.verify=ca-files",
                ClientOrServer::Client
            )
            .is_err()
        );
    }

    #[test]
    /// BR-R-023 — any `tls.*` key on a `transport=rtu` descriptor is a setup failure naming the
    /// key, including `tls.mode=none`, for both roles; the same descriptor without the key
    /// parses.
    fn ut_bridge_descriptor_rtu_rejects_tls_keys() {
        let upstream = match parse_bridge_descriptor(
            "transport=rtu,path=/dev/ttyUSB0,tls.mode=none",
            ClientOrServer::Server,
        ) {
            Ok(_) => panic!("a tls.* key on an rtu descriptor must be rejected"),
            Err(e) => e,
        };
        assert!(
            upstream.contains("unrecognized descriptor key 'tls.mode'"),
            "{upstream}"
        );

        let downstream = match parse_bridge_descriptor(
            "transport=rtu,path=/dev/ttyUSB0,tls.mode=tls",
            ClientOrServer::Client,
        ) {
            Ok(_) => panic!("a tls.* key on an rtu descriptor must be rejected"),
            Err(e) => e,
        };
        assert!(
            downstream.contains("unrecognized descriptor key 'tls.mode'"),
            "{downstream}"
        );

        assert!(
            parse_bridge_descriptor("transport=rtu,path=/dev/ttyUSB0", ClientOrServer::Server)
                .is_ok()
        );
    }

    #[test]
    /// BR-R-004 — an unknown transport, a tcp descriptor missing `port`, and an rtu descriptor
    /// missing `path` all error.
    fn ut_parse_bridge_descriptor_rejects_invalid_transport_and_missing_required() {
        assert!(parse_bridge_descriptor("transport=usb,port=0", ClientOrServer::Server).is_err());
        assert!(parse_bridge_descriptor("transport=tcp", ClientOrServer::Server).is_err());
        assert!(parse_bridge_descriptor("transport=rtu", ClientOrServer::Server).is_err());
    }

    #[test]
    /// BR-R-016 — an invalid `reconnect` value errors.
    fn ut_parse_bridge_descriptor_rejects_bad_reconnect_value() {
        assert!(parse_bridge_descriptor("port=0,reconnect=maybe", ClientOrServer::Server).is_err());
    }

    #[test]
    /// CL-R-045 — `--device` carries the flag's value straight into `ModuleSpec::device` as a
    /// literal path, never split as a `key=val,...` descriptor the way `--module` is, and
    /// exposes no endpoint or role control (both are the flag's fixed defaults).
    fn ut_device_flag_value_is_a_literal_path_not_a_descriptor() {
        let spec =
            create_module_spec_by_device("Device 0".to_string(), "name=x,port=1".to_string());
        assert_eq!(spec.device, "name=x,port=1");
        assert_eq!(spec.name, "Device 0");
        assert_eq!(spec.role, Role::Client);
        assert_eq!(
            spec.endpoint,
            Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 5020
            }
        );

        let args = CliArgs {
            command: None,
            modules: vec![],
            sessions: vec![],
            devices: vec!["name=x,port=1".into()],
            demo: false,
        };
        let specs = args
            .module_specs()
            .expect("a --device value is a path, never parsed as a descriptor");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].device, "name=x,port=1");
        assert_eq!(specs[0].name, "Device 0");
        assert_eq!(specs[0].role, Role::Client);
        assert_eq!(
            specs[0].endpoint,
            Endpoint::Tcp {
                ip: "127.0.0.1".into(),
                port: 5020
            }
        );
    }
}
