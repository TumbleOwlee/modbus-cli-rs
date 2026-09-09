# Config & Session — Edge Cases & Known Limitations

Boundary and error behavior of the envelope, plus known limitations. Protocol-specific cases (bad register range, wss server without certs) belong to `modbus/` or `ocpp/`.

---

## Malformed and ill-typed files

- **CS-E-001** — **Malformed TOML / JSON** — bytes not parsing as the extension's format fail with a deserialize error. No partial object (CS-R-050).
- **CS-E-002** — **Wrong / missing extension** — neither `.toml` nor `.json` (including none) rejected with an unknown-format error before the file is read or written. Never inferred from content (CS-R-002, CS-R-003).
- **CS-E-003** — **Missing required field** — syntactically valid file omitting a required field (module instance with no `name`, `device`, or endpoint) fails with a deserialize error (CS-R-051).
- **CS-E-004** — **Unknown field** — a field the schema does not define is silently ignored. No config type uses strict unknown-field rejection, so a **misspelled key silently takes its default**: a known sharp edge for hand-edited files.

## Module `"type"` dispatch

- **CS-E-005** — **Absent `"type"`** — treated as `"modbus"`, so pre-multi-type session files load (CS-R-012).
- **CS-E-006** — **Unsupported `"type"`** — anything other than `"modbus"`/`"ocpp"` is a hard error aborting session resolution (CS-R-013). Differs from the missing-device-file case (CS-E-008), which is non-fatal.
- **CS-E-007** — **Type/spec mismatch** — an entry tagged `"modbus"` whose fields are an OCPP endpoint (or vice versa) fails deserialization as its declared type and aborts resolution.

## A session referencing a device config that does not exist

- **CS-E-008** — Both module types handle a missing or unreadable device path the same way: the instance is **skipped** with a warning on stderr (`Skipping '<name>': failed to load '<path>': …`), startup continues without that tab. A broken `device` path never silently degrades to defaults. A **blank** `device` path is not an error: a quick-start with no device file, built on the default device config; only a *non-blank* path that fails is skipped. Neither case aborts startup.

## Duplicate instance names

Two instances resolving to the same `name` do not collide: the first keeps it; each later duplicate gets ` (2)`, ` (3)`, … in creation order, skipping taken suffixes. De-duplication spans **both** module types (a modbus `evse` and an ocpp `evse` become `evse` and `evse (2)`). The renamed tab logs a warning.

## Save targets

- **CS-E-009** — **No path** — `:write` defaults to `session.toml` (TOML, current working directory).
- **CS-E-010** — **Unwritable / non-existent target directory** — save fails with a create/write error in the active tab's log; in-memory session unchanged. No partial-file guarantee beyond the OS: a failed write may leave a truncated file, but running state is not corrupted.
- **CS-E-011** — **Format from extension** — `:write out.json` writes JSON, `:write out.toml` TOML; unrecognized extension fails with an unknown-format message, writes nothing.

## Round-trip omissions (working as designed)

- **CS-E-012** — `:write` persists configuration, not runtime state: live register/coil values, in-flight transactions, CSMS observed topology, OCPP runtime config-key mutations are **not** written (CS-R-061). A reloaded session starts every instance from its device-config baseline.
- **CS-E-013** — `:write` does **not** save referenced device-config files (CS-R-032). TUI edits to a device config must be saved through the device-config save command ([`../tui/`](../tui/) / protocol areas); otherwise lost.

## Migration edge cases

- **CS-E-014** — **Migrating an already-current config** — `migrate` always parses input against the **legacy** schema. A current file fed to it is read as legacy: fields sharing names/shapes carry through, current-only fields are ignored, result stamped with the current version. No already-current detection; not identity-preserving. Point it only at genuinely pre-v0.4.0 files.
- **CS-E-015** — **Unrecognized / unparseable legacy config** — extension not `.toml`/`.json`, or contents not deserializing against the legacy schema, aborts with a non-zero exit code and a diagnostic; nothing written (CS-R-066).
- **CS-E-016** — **Per-register failures non-fatal** — unknown read code or address above 16-bit skips only that register with a warning; migration completes with the rest (CS-R-044).
- **CS-E-017** — **Lossy-but-intended drops** — `history_length`, per-register `reverse`, per-range `slave_id`, UTF-8 string subtypes have no equivalent; dropped with a warning each.

## Legacy `update` self-heal on ordinary load

Loading a Modbus device config folds any legacy per-register `update` snippet into the global `scripts` list and clears the field — on **every** load, not only `migrate`. A device config still carrying `update` fields will, once loaded and saved, be rewritten with the snippets in `scripts` and the `update` fields gone. Intentional; `update` is never written back.

---

## Known limitations (stated, not bugs)

- **CS-E-018** — **The `version` field is inert.** Session and device-config files carry a `version` stamped on save and **never read by any load-time or migration branch**. Loading a file from any past or future build behaves identically regardless of the stamp (CS-R-018, CS-R-022).
- **CS-E-019** — **Strict field validation is scoped to the TLS subtree and the OCPP `security` table (CS-R-055).** Everywhere else no config type rejects unknown fields, so a misspelled key outside them silently takes its default.
- **CS-E-020** — **Missing device file drops the tab; both types.** The CS-E-008 skip is easy to miss because startup continues.
- **CS-E-021** — **`migrate` has no already-current guard.** Input unconditionally interpreted as legacy (`## Migration edge cases`); meaningful only on pre-v0.4.0 `modbus-cli-rs` files.
- **CS-E-022** — **Retired TLS field** — a TLS block naming a pre-merge field fails the load with an error naming the retired fields and the current block shape, rather than being ignored under CS-R-052. Dropping a server's `require_client_cert`/`client_ca_files` would silently downgrade a mutual-TLS listener to one accepting any client. The other retired names fail closed on their own (a client that stops presenting an identity, or stops trusting a private CA, fails its handshake loudly) but are rejected alongside so one rule covers the block.
- **CS-E-023** — **Retired-field scan is keyed off field names, not schema position.** The re-parse phrasing the retired-field error (CS-R-068) enters scope on any key literally named `tls` or `security`, at any depth; a register/script/module named `tls` or `security` would be scanned as a TLS container. No current schema uses either name otherwise, so latent, not live. The scan descends into array elements.
