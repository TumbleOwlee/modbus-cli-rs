# Config & Session — Requirements

The **configuration envelope**: TOML/JSON format, session file (module instances + session scripts), device-config file and how the two compose, save/load round-trip, `migrate` subcommand.

Per [`../README.md`](../README.md)'s ownership rule, this area owns only the *envelope*. Protocol-specific fields (Modbus register/timing/endpoint, OCPP version/role/security/config-key) are [`../modbus/`](../modbus/) and [`../ocpp/`](../ocpp/). The `:write` mechanism is [`../tui/`](../tui/); the `migrate`/`run` CLI surface is [`../cli-headless/`](../cli-headless/). This file specifies what those surfaces read and write.

---

## File format & encoding

**CS-R-001** — Every configuration file (session or device-config) is TOML or JSON; never YAML.

**CS-R-002** — Encoding is selected solely from the path extension: `.toml` TOML, `.json` JSON, case-insensitive. Content is never sniffed.

**CS-R-003** — A path whose extension is neither `.toml` nor `.json` (including no extension) fails with an unknown-format error on load and save, before any read or write of contents.

**CS-R-004** — The two encodings describe the same data model: a value serialized to one and re-serialized to the other (via the conversion helper) deserializes back to an equal value.

**CS-R-005** — On TOML serialization a numeric value is emitted as a plain TOML integer or float, never an internal arbitrary-precision wrapper table.

**CS-R-056** — On TOML serialization a `u64` exceeding the signed 64-bit range is emitted as a TOML float (CS-R-005).

**CS-R-006** — TOML has no null. A field with no value is omitted, not written as null.

**CS-R-057** — A JSON null at the top level or inside a TOML array (no key to omit, so CS-R-006 cannot apply) fails TOML serialization.

---

## Session model

**CS-R-010** — A session file consists of exactly four envelope-level fields: optional `version` string, `modules` list of module-instance entries, `scripts` list of session-level Lua scripts, `interval` sim-cycle period in seconds.

**CS-R-011** — Each `modules` entry is a self-describing object carrying a `"type"` tag naming its kind (`"modbus"` or `"ocpp"`); the loader dispatches on it.

**CS-R-012** — An entry with no `"type"` tag is treated as `"modbus"`, so session files predating multiple module types still load.

**CS-R-013** — An entry with a `"type"` other than `"modbus"` or `"ocpp"` is rejected with a hard error aborting session resolution.

**CS-R-014** — Each module instance carries a `name`: its tab title and `C_Module` registry key.

**CS-R-058** — When a session yields two module instances with the same `name` (CS-R-014), the second and later are renamed by appending ` (2)`, ` (3)`, … in creation order, skipping any suffix already taken.

**CS-R-015** — Each instance references its device type by a `device` field holding the device-config file path, plus the per-instance endpoint fields its protocol area defines. The entry carries no register table, timing, TLS/security, or OCPP version/role; those live in the referenced device config.

**CS-R-016** — The session `scripts` list holds session-level Lua scripts running in their own Lua state with access to every module; execution semantics in [`../scripting/`](../scripting/). A file lacking `scripts` loads with an empty list.

**CS-R-017** — The session `interval` is a sim-cycle period in seconds, default `1.0` when absent.

**CS-R-059** — A non-finite, zero, or negative session `interval` (CS-R-017) falls back to `1.0`.

**CS-R-060** — A valid positive session `interval` (CS-R-017) is used as-is; there is no minimum floor.

**CS-R-018** — The session `version` field is informational only: stamped with the writing build's version on save, never consulted by any load-time or migration branch.

---

## Device config composition

**CS-R-020** — The configuration model distinguishes a **session file** (module instances plus session scripts) and a **device-config file** (exactly one device type). One device-config file may be referenced by any number of session instances.

**CS-R-021** — Per-instance wire addressing (name, role, endpoint) lives in the session entry; everything describing the device type (register/variable model, timing, scripts, security) lives in the device-config file. Device-config field sets are specified in the Modbus and OCPP areas.

*Coverage note: CS-R-020 and CS-R-021 are structural — the split is a Rust-type-level fact (`Session` vs `DeviceConfig` are distinct structs with disjoint fields), not an independently testable runtime behavior. Exercised by CS-R-004's cross-encoding device round-trip, CS-R-033's session round-trip, and CS-R-015's instance-vs-device field-split assertions; no dedicated test for the umbrella.*

**CS-R-022** — A device-config file also carries an optional, informational `version` string with CS-R-018's semantics: stamped on save, never branched on.

**CS-R-023** — A device-config file loads even when it predates fields added later: every recognized field has a default.

---

## Save / load & round-trip

**CS-R-030** — The running TUI saves the current module instances as a session file on `:write`.

**CS-R-069** — `:write` with no path given (CS-R-030) writes `session.toml`.

**CS-R-070** — A session file written by `:write` (CS-R-030) is encoded from the target path's extension per CS-R-002.

**CS-R-031** — A save persists **configuration only**: module instance specs, session scripts, session interval, freshly stamped `version`.

**CS-R-061** — A save writes no live runtime state (current register/coil values, in-flight Modbus transactions, the CSMS's observed station topology, runtime mutations to an OCPP config-key/variable store); only the configuration of CS-R-031 is written.

**CS-R-032** — A session `:write` writes no device-config file. Device configs are saved through their own command surface ([`../tui/`](../tui/) and the protocol areas).

**CS-R-033** — A session file saved by the TUI and loaded again reproduces the same instance list (names, types, device paths, endpoints), session scripts, and interval.

**CS-R-034** — Serialization omits fields carrying their default/empty value where the schema declares them omittable (informational `version` when unset, empty `scripts`, unset optional endpoint sub-fields). A file so written reloads to an equal value because each omitted field's load-time default matches.

---

## Migration

**CS-R-040** — The `migrate` subcommand converts a pre-rewrite (`modbus-cli-rs`, ≤ v0.3.9) configuration file into a current device-config file. CLI invocation (`--input` / `--output`) is [`../cli-headless/`](../cli-headless/); this area specifies the transformation.

**CS-R-041** — Migration applies the legacy-to-current transformation, starting by swapping the holding/input read codes.

**CS-R-062** — Migration splits a trailing `le` type suffix into an explicit little-endian byte order.

**CS-R-063** — Migration folds each legacy per-register `on_update` Lua snippet into a named entry of the global `scripts` list.

**CS-R-064** — Migration merges `[[contiguous_memory]]` ranges into `read_ranges` grouped by function code.

**CS-R-065** — Migration renames `delay_after_connect_ms` to the current delay field.

**CS-R-042** — Migration drops legacy fields with no current equivalent (e.g. `history_length`, per-register `reverse`, per-range `slave_id`, UTF-8 string subtypes) and emits a warning for each.

**CS-R-043** — Migration stamps the output with the current build's `version`. Input and output encodings are each chosen from their own extension, so any TOML/JSON source may migrate to a TOML or JSON destination.

**CS-R-044** — A per-register conversion error during migration (unknown read code, address exceeding 16-bit) skips only that register with a warning.

**CS-R-066** — During migration an unrecognized input/output extension or a load/save failure aborts with a non-zero exit code and a diagnostic on stderr.

**CS-R-045** — `migrate` converts device-config files only; it does not convert or produce session files.

---

## Error handling

**CS-R-050** — Malformed TOML or JSON fails to load with a deserialize error. No partial or best-effort object.

**CS-R-051** — Valid TOML/JSON omitting a required field (e.g. a module instance with no `name` or no endpoint) fails to load with a deserialize error.

**CS-R-052** — A field present in a file but not in the schema is ignored silently on load, except within a TLS block, governed by CS-R-055.

**CS-R-053** — When a session references a missing or unreadable device-config file, startup does not abort. The instance is skipped with a warning naming it and the failed path, identically for Modbus and OCPP; neither falls back to a default device config.

**CS-R-067** — A **blank** device path in a session instance is not a failure (unlike CS-R-053's missing file): the instance is a quick-start built on the default device config.

**CS-R-054** — Loading a device config self-heals a legacy per-register `update` snippet on **every** load, not only via `migrate`, folding it into the global `scripts` list and clearing the per-register field, so a subsequent save writes only the global list.

**CS-R-055** — Strict field checking applies throughout a TLS subtree (the `tls` container, each `server`/`client` policy block, each policy's `identity`/`verification` payload) and to the OCPP `security` table enclosing one, so a field the enclosing variant or table does not define fails the load rather than being ignored under CS-R-052.

**CS-R-071** — `username` and `password` remain defined members of the OCPP `security` table, so neither is rejected by CS-R-055's strict checking.

**CS-R-072** — CS-R-055 is the sole exception to CS-R-052: no table outside a TLS subtree and its enclosing OCPP `security` table is strict-checked, the exception being made because silently ignoring a retired TLS field can weaken an endpoint's security posture.

**CS-R-068** — A table in the strictly checked TLS subtree or OCPP `security` table (CS-R-055) naming a pre-merge field (`require_client_cert`, `client_ca_files`, `client_ca_file`, `client_cert_skip_verify`, `insecure_skip_verify`, `client_cert_file`, `client_key_file`, `client_self_signed`, `ca_file`, or a bare `self_signed`/`cert_file`/`key_file` outside an `identity` block) fails the load with an error naming the retired fields found and pointing at the current block shape. No value migrated.
