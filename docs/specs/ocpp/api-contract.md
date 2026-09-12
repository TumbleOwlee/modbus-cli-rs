# OCPP — API Contract

Exhaustive action table per version, direction per action, OCPP-J error codes, OCPP module config fields (TLS/mTLS, HTTP Basic Auth). Per [`../README.md`](../README.md)'s ownership rule, OCPP config fields live here, not `config-session/` (envelope only).

---

## Versions and subprotocols

| Version | Subprotocol token | Actions | CS→CSMS | CSMS→CS | Req |
|---|---|---|---|---|---|
| 1.6 | `ocpp1.6` | **28** | 10 | 18 | OC-R-001, OC-R-002, OC-R-003, OC-R-004 |
| 2.0.1 | `ocpp2.0.1` | **64** | 25 | 39 | OC-R-001, OC-R-002, OC-R-003, OC-R-004 |
| 2.1 | `ocpp2.1` | **90** | 37 | 53 | OC-R-001, OC-R-002, OC-R-003, OC-R-004, OC-R-007 |

2.1 is a strict superset of 2.0.1: 64 shared actions verbatim plus 26 new (OC-R-007). The one-way streaming datagram `NotifyPeriodicEventStream` has no request/response pair and is deliberately **not** an action.

**Connector scope** on CSMS→CS actions: `None` (charge-point-wide only), `Optional` (charge-point or connector level), `Required` (connector-only). Derived from the presence and optionality of the request's *top-level* connector/EVSE target; a nested-optional EVSE field (inside charging-profile or variable criteria) counts as `None` (OC-R-005).

---

## OCPP 1.6 — 28 actions

### CS→CSMS (10)

`Authorize`, `BootNotification`, `DataTransfer`, `DiagnosticsStatusNotification`, `FirmwareStatusNotification`, `Heartbeat`, `MeterValues`, `StartTransaction`, `StatusNotification`, `StopTransaction` (OC-R-002, OC-R-003).

### CSMS→CS (18)

| Action | Scope | Req |
|---|---|---|
| `CancelReservation` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ChangeAvailability` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `ChangeConfiguration` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ClearCache` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ClearChargingProfile` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `GetCompositeSchedule` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `GetConfiguration` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetDiagnostics` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetLocalListVersion` | None | OC-R-002, OC-R-003, OC-R-005 |
| `RemoteStartTransaction` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `RemoteStopTransaction` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ReserveNow` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `Reset` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SendLocalList` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SetChargingProfile` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `TriggerMessage` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `UnlockConnector` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `UpdateFirmware` | None | OC-R-002, OC-R-003, OC-R-005 |

---

## OCPP 2.0.1 — 64 actions

### CS→CSMS (25)

`Authorize`, `BootNotification`, `ClearedChargingLimit`, `DataTransfer`, `FirmwareStatusNotification`, `Get15118EVCertificate`, `GetCertificateStatus`, `Heartbeat`, `LogStatusNotification`, `MeterValues`, `NotifyChargingLimit`, `NotifyCustomerInformation`, `NotifyDisplayMessages`, `NotifyEVChargingNeeds`, `NotifyEVChargingSchedule`, `NotifyEvent`, `NotifyMonitoringReport`, `NotifyReport`, `PublishFirmwareStatusNotification`, `ReportChargingProfiles`, `ReservationStatusUpdate`, `SecurityEventNotification`, `SignCertificate`, `StatusNotification`, `TransactionEvent` (OC-R-002, OC-R-003).

### CSMS→CS (39)

| Action | Scope | Req |
|---|---|---|
| `CancelReservation` | None | OC-R-002, OC-R-003, OC-R-005 |
| `CertificateSigned` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ChangeAvailability` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `ClearCache` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ClearChargingProfile` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ClearDisplayMessage` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ClearVariableMonitoring` | None | OC-R-002, OC-R-003, OC-R-005 |
| `CostUpdated` | None | OC-R-002, OC-R-003, OC-R-005 |
| `CustomerInformation` | None | OC-R-002, OC-R-003, OC-R-005 |
| `DeleteCertificate` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetBaseReport` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetChargingProfiles` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `GetCompositeSchedule` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `GetDisplayMessages` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetInstalledCertificateIds` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetLocalListVersion` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetLog` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetMonitoringReport` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetReport` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetTransactionStatus` | None | OC-R-002, OC-R-003, OC-R-005 |
| `GetVariables` | None | OC-R-002, OC-R-003, OC-R-005 |
| `InstallCertificate` | None | OC-R-002, OC-R-003, OC-R-005 |
| `PublishFirmware` | None | OC-R-002, OC-R-003, OC-R-005 |
| `RequestStartTransaction` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `RequestStopTransaction` | None | OC-R-002, OC-R-003, OC-R-005 |
| `ReserveNow` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `Reset` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `SendLocalList` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SetChargingProfile` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `SetDisplayMessage` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SetMonitoringBase` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SetMonitoringLevel` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SetNetworkProfile` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SetVariableMonitoring` | None | OC-R-002, OC-R-003, OC-R-005 |
| `SetVariables` | None | OC-R-002, OC-R-003, OC-R-005 |
| `TriggerMessage` | Optional | OC-R-002, OC-R-003, OC-R-005 |
| `UnlockConnector` | Required | OC-R-002, OC-R-003, OC-R-005 |
| `UnpublishFirmware` | None | OC-R-002, OC-R-003, OC-R-005 |
| `UpdateFirmware` | None | OC-R-002, OC-R-003, OC-R-005 |

---

## OCPP 2.1 — 90 actions

All 64 of `## OCPP 2.0.1 — 64 actions` plus the 26 below. **No shared action changes direction or scope in 2.1.** 2.1's additions to shared payload types are all optional fields, so shared actions remain decode-compatible.

### New in 2.1, CS→CSMS (12)

`BatterySwap`, `GetCertificateChainStatus`, `NotifyDERAlarm`, `NotifyDERStartStop`, `NotifyPriorityCharging`, `NotifySettlement`, `NotifyWebPaymentStarted`, `OpenPeriodicEventStream`, `ClosePeriodicEventStream`, `PullDynamicScheduleUpdate`, `ReportDERControl`, `VatNumberValidation` (OC-R-002, OC-R-003, OC-R-007).

### New in 2.1, CSMS→CS (14)

| Action | Scope | Req |
|---|---|---|
| `AFRRSignal` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `AdjustPeriodicEventStream` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `ChangeTransactionTariff` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `ClearDERControl` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `ClearTariffs` | Optional | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `GetDERControl` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `GetPeriodicEventStream` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `GetTariffs` | Required | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `NotifyAllowedEnergyTransfer` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `RequestBatterySwap` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `SetDERControl` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `SetDefaultTariff` | Required | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `UpdateDynamicSchedule` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |
| `UsePriorityCharging` | None | OC-R-002, OC-R-003, OC-R-005, OC-R-007 |

Totals: 25 + 12 = **37** CS→CSMS; 39 + 14 = **53** CSMS→CS; **90**.

---

## OCPP-J CallError codes

Fixed set, spelled as on the wire:

| Code | Emitted when | Req |
|---|---|---|
| `NotImplemented` | action unknown to the negotiated version, or the peer's simulator does not handle it | OC-R-026 |
| `NotSupported` | (accepted on the wire; never emitted) | OC-R-012 |
| `InternalError` | a crafted response failed to encode; a CSMS command named an unknown connection | OC-R-055 |
| `ProtocolError` | (accepted on the wire; never emitted) | OC-R-012 |
| `SecurityError` | (accepted on the wire; never emitted) | OC-R-012 |
| `FormationViolation` | Call payload failed to deserialize, or failed the version's validation rules | OC-R-027, OC-R-098 |
| `PropertyConstraintViolation` | inbound Call targets a connector/EVSE the station lacks | OC-R-063 |
| `OccurenceConstraintViolation` | (accepted on the wire; never emitted — spec's own spelling) | OC-R-012 |
| `TypeConstraintViolation` | (accepted on the wire; never emitted) | OC-R-012 |
| `GenericError` | awaited Call timed out, its connection closed, or was torn down | OC-R-020, OC-R-022, OC-R-012 |

An `errorCode` matching none of the ten is accepted and read as `GenericError` (OC-R-012).

---

## Module instance spec (session / `--ocpp`)

One OCPP instance: the per-instance on-the-wire endpoint. Version, role, timeout, security, scripts, connectors, configuration keys, (client role) CS boot identity live in the device config (`## Device config (one file = one device type)`), never here.

| Field | Type | Default | Valid values | Req |
|---|---|---|---|---|
| `name` | string | — (required) | tab / instance name | CS-R-014, OC-R-081 |
| `device` | string | — (required) | OCPP device config file path | CS-R-015, OC-R-081 |
| `protocol` | enum | `ws` | `ws`, `wss` | OC-R-042, OC-R-097 |
| `ip` | string | — (required in the session file; `127.0.0.1` from `--ocpp`) | host | OC-R-043, OC-R-081 |
| `port` | u16 | — (required) | 0–65535; `0` in the server role binds an OS-assigned port | OC-R-043, OC-R-081 |
| `path` | string | empty | URL path, e.g. `/ocpp/cp001`. Empty = none | OC-R-043, OC-R-044, OC-R-081 |

Dialed/advertised URL: `{protocol}://{ip}:{port}{path}` (OC-R-043). Charge-point identity by convention = last non-empty segment of `path` (OC-R-044).

### `--ocpp` key/value form

`--ocpp name=…,device=…,protocol=…,ip=…,port=…,path=…` accepts the same keys; `name`, `device`, `port` **required**; `ip` default `127.0.0.1`; `protocol` default `ws`; `path` default empty. `protocol` other than `ws`/`wss` is an error (CL-R-002).

---

## Endpoint enums

| Enum | Values | Serialized as | Req |
|---|---|---|---|
| `ocpp_version` | 1.6 (default), 2.0.1, 2.1 | `"1.6"`, `"2.0.1"`, `"2.1"` | OC-R-001, OC-R-080 |
| `role` | client (default), server | `"client"`, `"server"` | OC-R-079 |
| `protocol` | ws (default), wss | `"ws"`, `"wss"` | OC-R-042, OC-R-097 |

`client` = Charging Station (CS); `server` = CSMS.

---

## Device config (one file = one device type)

| Field | Type | Default | Notes | Req |
|---|---|---|---|---|
| `version` | optional string | unset | ferrowl version, stamped on save | CS-R-022 |
| `ocpp_version` | enum | `1.6` | `## Endpoint enums`. Version-locks the file: scripts call version-specific actions | OC-R-001, OC-R-080 |
| `role` | enum | `client` | `## Endpoint enums` | OC-R-079 |
| `timeout_ms` | optional u64 | `30000` when unset | awaited-reply timeout, both roles | OC-R-020 |
| `scripts` | list of script defs | empty | Lua sim scripts — `scripting/`. Client role only | SC-R-022 |
| `script_interval` | f64 seconds | `1.0` | Lua sim cycle; floored at `0.05`; NaN/∞/≤0 → `1.0` | SC-R-016, SC-R-045 |
| `log_file` | optional string | unset | persistent log-file base, also set by `:log <file>` | OC-R-087, OC-R-088 |
| `rfids` | list of string | empty | **server only**: charge-point-wide RFID accept-list | OC-R-074, OC-R-075 |
| `connector_rfids` | list of `ConnectorRfids` | empty | **server only**: per-connector accept-lists | OC-R-074, OC-R-075 |
| `connectors` | list of `ConnectorRef` | empty | **client only**: connector-table seed. Empty = CS-level only. Unbounded | OC-R-057, OC-R-081 |
| `config` | list of `ConfigKeyDef` | empty | **client only**: persisted configuration/variable key store. Empty = built-in defaults | OC-R-057, OC-R-081 |
| `extra_headers` | list of `HeaderDef` | empty | **client only**: extra HTTP headers on the WebSocket upgrade, in addition to the client's own. OC-R-117–119 | OC-R-117, OC-R-152, OC-R-153, OC-R-118, OC-R-119 |
| `model` | optional string | unset | **client only**: CS boot identity model, seeded/written like `connectors`/`config` | OC-R-103, OC-R-140 |
| `vendor` | optional string | unset | **client only**: CS boot identity vendor | OC-R-103 |
| `firmware_version` | optional string | unset | **client only**: CS boot identity firmware version | OC-R-103 |
| `serial_number` | optional string | unset | **client only**: CS boot identity serial number | OC-R-103 |
| `iccid` | optional string | unset | **client only, 1.6 only**: SIM ICCID, seeded/written like `model`/`vendor`. Inert for 2.0.1/2.1 | OC-R-104, OC-R-141 |
| `imsi` | optional string | unset | **client only, 1.6 only**: SIM IMSI | OC-R-104 |
| `meter_serial_number` | optional string | unset | **client only, 1.6 only**: installed meter's serial number | OC-R-104 |
| `meter_type` | optional string | unset | **client only, 1.6 only**: installed meter's type/model | OC-R-104 |
| `security` | `OcppSecurityConfig` | all-unset | ``## Security config (`security`)`` | OC-R-126 |

A device config written before any of these fields existed still loads: every field defaulted.

### `ConnectorRef`

| Field | Type | Notes | Req |
|---|---|---|---|
| `evse` | optional i64 | `None` for 1.6 (connector-only addressing); `Some` for 2.0.1/2.1 | — |
| `connector` | i64 | connector id | OC-R-058, OC-R-077 |

### `ConnectorRfids`

| Field | Type | Notes | Req |
|---|---|---|---|
| `evse` | optional i64 | as above | — |
| `connector` | optional i64 | as above | OC-R-058, OC-R-077 |
| `rfids` | list of string | tags accepted for that connector, **in addition to** the charge-point-wide list | OC-R-074, OC-R-075 |

### `ConfigKeyDef`

| Field | Type | Default | Req |
|---|---|---|---|
| `key` | string | — (required) | OC-R-057, OC-R-066 |
| `value` | string | empty | OC-R-057, OC-R-066 |
| `readonly` | bool | `false` | OC-R-066 |

### `HeaderDef`

| Field | Type | Default | Req |
|---|---|---|---|
| `name` | string | — (required) | OC-R-117, OC-R-118 |
| `value` | string | — (required) | OC-R-117, OC-R-118 |

---

## Security config (`security`)

One section, both roles. Basic Auth role-shared (OC-R-156); TLS held per role in a `tls` sub-block (OC-R-126, OC-R-158), of which an instance consults only its own role's — the other inert. Default (no auth, both policies `none`) = plain `ws://`.

| Field | Type | Default | Role | Meaning | Req |
|---|---|---|---|---|---|
| `username` | optional string | unset | both | Basic Auth username (Profile 1). Client sends; server requires | OC-R-030, OC-R-031, OC-R-128, OC-R-156 |
| `password` | optional string | unset | both | Basic Auth password. Never logged | OC-R-030, OC-R-031, OC-R-033, OC-R-128, OC-R-156 |
| `tls.server` | `ServerTlsPolicy` | `none` | server | consulted when the instance runs as CSMS | OC-R-126, OC-R-037 |
| `tls.client` | `ClientTlsPolicy` | `none` | client | consulted when the instance runs as CS | OC-R-126, OC-R-035 |

`ServerTlsPolicy`, tagged `mode`:

| `mode` | Payload | Meaning | Req |
|---|---|---|---|
| `none` | — | plain listener | OC-R-042 |
| `tls` | `identity: CertSource` | presents a server certificate, requests none | OC-R-037, OC-R-096 |
| `mutual` | `identity: CertSource`, `verification: CertVerification` | also requests and verifies a client certificate | OC-R-037, OC-R-039, OC-R-096 |

`ClientTlsPolicy`, tagged `mode`:

| `mode` | Payload | Meaning | Req |
|---|---|---|---|
| `none` | — | plain connection | OC-R-097 |
| `tls` | `verification: CertVerification` | verifies the server, presents no identity | OC-R-034 |
| `mutual` | `verification: CertVerification`, `identity: CertSource` | also presents a client certificate | OC-R-034, OC-R-035 |

`CertSource`, tagged `source`:

| `source` | Payload | Meaning | Req |
|---|---|---|---|
| `ephemeral` | — | server only: no material configured, bind an ephemeral self-signed certificate and log the fallback (OC-R-095) | OC-R-095 |
| `self-signed` | — | ephemeral self-signed pair, explicitly chosen, no fallback logged | OC-R-037, OC-R-095 |
| `files` | `cert_file`, `key_file` — both required | PEM chain and matching private key | OC-R-037, OC-R-112 |

`CertVerification`, tagged `verify`:

| `verify` | Payload | Meaning | Req |
|---|---|---|---|
| `skip` | — | accept any peer certificate unauthenticated. Test rigs only | OC-R-036, OC-R-134 |
| `root-store` | `extra_ca_files` — list, may be empty | client only: webpki root store plus these anchors | OC-R-034, OC-R-129 |
| `ca-files` | `ca_files` — list, non-empty | exactly these anchors, not the root store | OC-R-034, OC-R-130, OC-R-039 |

### Derivation rules

- **Basic Auth on** iff *both* `username` and `password` set. Either alone inert (OC-R-164).
- **TLS on** for an instance iff its own role's policy is other than `none`. The variant *is* the state; the other role's policy never affects it (OC-R-158).
- **mTLS on** iff that policy's `mode` is `mutual`. A `mutual` client always carries an identity, a `mutual` server always a verification — required by the variant (OC-R-035, OC-R-039).
- **Endpoint scheme gates TLS** in both roles (OC-R-042). `ws://` always plaintext; any policy alongside inert. A URL never advertises a transport its peer does not speak.
- **A `wss://` server's TLS material** follows its `identity` directly (OC-R-096); the old "explicit files always win" precedence is unrepresentable.
- **A `wss://` server with identity `ephemeral`** binds an ephemeral self-signed certificate, not plain TCP, and logs the fallback (OC-R-095).
- Two role-only rejections at construction: `verify = "root-store"` under `tls.server`; `source = "ephemeral"` as a `tls.client` identity (OC-R-133, OC-R-035).
- **The setup dialog does not expose `protocol` as an input** (OC-R-161): it displays a scheme derived from the TLS selector — `wss://` at TLS/mTLS, `ws://` at Off — and writes the matching `protocol`. The field (`## Endpoint enums`) is unchanged; a hand-written config may still pair any scheme with any policy, subject to OC-R-042/OC-R-097.

### Security profiles

| Profile | Configuration | Req |
|---|---|---|
| 1 — Basic Auth over `ws://` | `username` + `password`, `protocol = ws` | OC-R-029, OC-R-030, OC-R-031 |
| 2 — TLS, server cert only | `protocol = wss`; CSMS `tls.server.mode = "tls"` with any `identity`; CS `tls.client.mode = "tls"` with a `verification`. Optionally plus Basic Auth | OC-R-029, OC-R-037 |
| 3 — mutual TLS | Profile 2 with `mode = "mutual"` on both ends: CSMS adds `verification` (`ca-files`), CS adds `identity` | OC-R-029, OC-R-039 |

```toml
# One OCPP device config, both roles present; the instance's role picks one
[security]
username = "cs001"
password = "hunter2"

# used when this device runs as CSMS: require client certs from a private CA
[security.tls.server]
mode = "mutual"
[security.tls.server.identity]
source = "files"
cert_file = "/etc/ferrowl/csms.crt"
key_file  = "/etc/ferrowl/csms.key"
[security.tls.server.verification]
verify = "ca-files"
ca_files = ["/etc/ferrowl/fleet-ca.pem"]

# used when it runs as CS: platform roots plus one private CA, no identity
[security.tls.client]
mode = "tls"
[security.tls.client.verification]
verify = "root-store"
extra_ca_files = ["/etc/ferrowl/private-ca.pem"]
```

---

## `:` commands

Protocol-specific commands owned here (mechanism owned by `tui/`).

### Client (CS) view

| Command | Effect | Req |
|---|---|---|
| `:start` | connect to the CSMS | OC-R-083 |
| `:stop` | disconnect, abandoning an in-flight dial | OC-R-047, OC-R-175 |
| `:restart` | disconnect, then reconnect | OC-R-084 |
| `:e` / `:edit` | open the module setup dialog | — |
| `:wd` / `:write-device [path]` | save the device config | OC-R-103 |
| `:compact` | toggle compact table rows | — |
| `:log [file]` | set (no argument: clear) the persistent log file | OC-R-087, OC-R-088 |

### Server (CSMS) view

| Command | Effect | Req |
|---|---|---|
| `:start` | bind the listener | OC-R-138 |
| `:stop` | unbind the listener, abandoning an in-flight bind or a pending accept, and discard every observed station entry | OC-R-084, OC-R-176, OC-R-177 |
| `:restart` | rebind the listener; discards every observed station entry | OC-R-084 |
| `:e` / `:edit` | open the module setup dialog | — |
| `:wd` / `:write-device [path]` | save the device config | — |
| `:compact` | toggle compact table rows | — |
| `:log [file]` | set (or clear) the persistent log file | OC-R-087, OC-R-088 |

A client does **not** connect on creation — `:start` required. A server binds automatically on creation (OC-R-083, OC-R-138).
