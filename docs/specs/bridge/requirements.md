# Bridge Mode — Requirements

`ferrowl bridge`: a headless subcommand relaying Modbus requests between two independently configured interfaces — an **upstream** (server role) and a **downstream** (client role). No persistent register store, no TUI, no Lua/`C_*` module — a pure protocol relay.

Per [`../README.md`](../README.md)'s ownership rules, this area does **not** own: the Modbus wire protocol, register codec, or client/server behavior bridge reuses unchanged ([`../modbus/`](../modbus/)); the `--module`/`--ocpp` mini-language grammar or the headless `run` exit-code/log contract bridge mirrors ([`../cli-headless/`](../cli-headless/)).

---

## Subcommand and scope

**BR-R-001** — The program exposes a `bridge` subcommand (`ferrowl bridge`), a third `SubCommand` variant alongside `migrate` and `run` (CL-R-010). Invoking it replaces the default action (starting the TUI).

**BR-R-002** — Bridge mode does not start the TUI, load a session file, or touch the Lua/`C_*` module or sim framework. It relays between exactly two configured interfaces and owns no persistent register store.

## Configuration

**BR-R-003** — `bridge` accepts exactly one `--upstream <descriptor>` and exactly one `--downstream <descriptor>`, both required. Missing either is a setup failure (exit 1, BR-R-013).

**BR-R-004** — Each descriptor uses the `key=val[,key=val...]` mini-language of `--module`'s endpoint keys: `transport` in {`tcp`, `rtu`, `rtu_over_tcp`, `ascii_over_tcp`} (default `tcp`).

**BR-R-016** — A descriptor's remaining keys (BR-R-004) match that transport's connection fields (`ip`,`port` for tcp/rtu_over_tcp/ascii_over_tcp; `path`,`baud`,`parity`,`data_bits`,`stop_bits` for rtu), plus optional `timeout_ms`, `reconnect`, `unit_ids` (BR-R-015), and, for tcp/rtu_over_tcp/ascii_over_tcp only, the `tls` key set (BR-R-011).

**BR-R-017** — `rtu_over_tcp`/`ascii_over_tcp` descriptors (BR-R-004) carry the same `tcp::Config` field set as `tcp` (MB-R-113/MB-R-125), differing only in framing; either may be upstream or downstream independently of the other side's transport.

## Roles and relay behavior

**BR-R-005** — Upstream always acts as server: bridge listens for/accepts connections (tcp, rtu_over_tcp, ascii_over_tcp) or serves the opened link (rtu), reusing the existing server accept-loop / single-serial-link behavior unchanged.

**BR-R-006** — Downstream always acts as client: bridge connects (tcp, rtu_over_tcp, ascii_over_tcp) or opens the serial port (rtu) as an ordinary client, including reconnect/backoff (MB-R-050–056) when enabled.

**BR-R-007** — Each decoded upstream request is forwarded downstream unmodified (same unit id, function code, address, count) and awaited; the downstream response or exception is relayed back upstream unmodified. No register/bit-count limit beyond each transport's wire format.

**BR-R-008** — A request to slave id 0 received on an RTU upstream is forwarded downstream and receives no upstream response (MB-R-103).

**BR-R-009** — A request forwarded to an RTU downstream addressed to slave id 0 is transmitted fire-and-forget, not awaited (MB-R-102).

**BR-R-010** — When downstream fails to connect while a forwarded request is outstanding (no established connection/serial link), bridge answers the upstream requester with exception `GatewayPathUnavailable` (0x0A).

**BR-R-018** — When downstream is connected but the forwarded request times out or the connection drops before a response, bridge answers the upstream requester with exception `GatewayTargetDeviceFailedToRespond` (0x0B).

**BR-R-019** — Both bridge exception codes `GatewayPathUnavailable` (0x0A, BR-R-010) and `GatewayTargetDeviceFailedToRespond` (0x0B, BR-R-018) exist in the vendored `rust_modbus::ExceptionCode` enum (values 10/11); ordinary ferrowl servers never emit them (`api-contract.md`'s exhaustive list is 0x01–0x04).

**BR-R-011** — TCP-socket interfaces (`tcp`, `rtu_over_tcp`, `ascii_over_tcp`; upstream and/or downstream) may enable TLS through descriptor keys mirroring the block form's paths, the interface's role fixed by its flag: an upstream descriptor carries a `ServerTlsPolicy` (BR-R-005), a downstream a `ClientTlsPolicy` (BR-R-006), neither the two-role container of MB-R-104.

**BR-R-020** — Within BR-R-004's grammar each TLS key (BR-R-011) is one dotted path: `tls.mode` (`none`|`tls`|`mutual`, default `none`), `tls.identity.source` (`ephemeral`|`self-signed`|`files`, default `ephemeral`), `tls.identity.cert_file`, `tls.identity.key_file`, `tls.verification.verify` (`skip`|`root-store`|`ca-files`, default `root-store` with empty `extra_ca_files`), `tls.verification.ca_files`, `tls.verification.extra_ca_files`; the two CA-list keys take `;` as intra-value delimiter. A TLS key whose path the selected variant does not define is a setup failure (exit 1, BR-R-013).

**BR-R-021** — Under `tls.mode=tls` or `tls.mode=mutual` (BR-R-020), an absent `tls.identity.source` and an absent `tls.verification.verify` take those defaults wherever the selected variant defines that field, so a descriptor naming only a mode resolves to an ephemeral identity and root-store verification with no extra CA files (the block form requires `identity` and `verification` written out, MB-R-105); a defaulted field is subject to every rule an explicit one faces.

**BR-R-022** — `tls.verification.verify=root-store` on an upstream descriptor (one role-only half of MB-R-167's three checks) is a setup failure (exit 1, BR-R-013).

**BR-R-028** — `tls.identity.source=ephemeral` on a downstream descriptor (the other role-only half of MB-R-167's three checks) is a setup failure (exit 1, BR-R-013).

**BR-R-029** — The BR-R-022 and BR-R-028 rejections apply to a defaulted value (BR-R-021) exactly as to a written one, so an upstream `tls.mode=mutual` with no `tls.verification.verify` and a downstream `tls.mode=mutual` with no `tls.identity.source` are each a setup failure.

**BR-R-030** — An upstream `tls.mode=tls` and a downstream `tls.mode=tls` are accepted, neither being caught by BR-R-022, BR-R-028, or BR-R-029.

**BR-R-023** — Any `tls.*` key on a `transport=rtu` descriptor is a setup failure (exit 1, BR-R-013) naming the offending key, whatever its path or value, including `tls.mode=none`.

**BR-R-024** — Descriptor TLS resolution, verification, and validation not covered by BR-R-011, BR-R-020, BR-R-021, BR-R-022, and BR-R-023 follow MB-R-104–109 and MB-R-161–MB-R-174; the bridge defines no TLS field of its own.

```
--upstream 'transport=tcp,ip=0.0.0.0,port=8502,tls.mode=mutual,tls.identity.source=self-signed,tls.verification.verify=ca-files,tls.verification.ca_files=/etc/ferrowl/a.pem;/etc/ferrowl/b.pem'
```

## Logging and process contract

**BR-R-012** — Bridge drains relayed-request and lifecycle log lines to stdout in the `[<timestamp>] <source> | <message>` format (CL-R-040), optionally appends to `--log-file` (CL-R-041), and keeps setup/fatal diagnostics on stderr (CL-R-042).

**BR-R-013** — Exit codes mirror `run` (CL-R-030–032): 1 for setup failure (missing/invalid `--upstream`/`--downstream`, upstream bind/listen/serial-open failure).

**BR-R-025** — The bridge exits 0 on `--duration` deadline or Ctrl-C (exit codes mirror `run`, BR-R-013).

**BR-R-026** — With `--exit-on-error`, the bridge exits 3 on a drained `[bridge]`-sourced error line (exit codes mirror `run`, BR-R-013).

**BR-R-014** — `bridge` accepts an optional `--duration <secs>`, identical semantics to `run`'s (CL-R-013 family).

## Multidrop bus safety

**BR-R-015** — The upstream descriptor's optional `unit_ids` key takes a comma-separated list/range (e.g. `unit_ids=1,3,5-8`) of allowed unit ids. When present, only a request to a listed unit id is forwarded (BR-R-007); any other is ignored entirely (not forwarded, no upstream response), so another device sharing the upstream link can answer it.

**BR-R-027** — When the upstream descriptor's `unit_ids` key (BR-R-015) is absent, every unit id is forwarded (MB-R-065).
