# CLI & Headless — API Contract

Exhaustive command-line surface a CI script or operator writes against: every flag and subcommand, the `--module`/`--ocpp` descriptor mini-language, the exit-code table. Stable public contract.

`migrate` transformation semantics and the config/session file formats: [`../config-session/`](../config-session/). This file: flag surface and dispatch only.

---

## Top-level command

```
ferrowl [OPTIONS]
ferrowl <SUBCOMMAND> ...
```

Default action (no subcommand): start the TUI with the resolved module set.

| Flag | Value | Default | Repeatable | Purpose | Req |
|---|---|---|---|---|---|
| `--module` | `KEY=VAL,...` | — | yes | one ad-hoc Modbus module (``## `--module` descriptor mini-language (Modbus)``) | CL-R-002 |
| `--session` | `FILE` | — | yes | session file (TOML/JSON) listing module instances. Resolved before `--module` | CL-R-003 |
| `--device` | `FILE` | — | yes | device-config file → one auto-built TCP **client** named `Device <n>` at `127.0.0.1:5020`. No endpoint/role control | CL-R-004, CL-R-044, CL-R-045 |
| `--demo` | (flag) | off | no | eight built-in demo tabs + an example session script; config flags ignored for tab building | CL-R-005, CL-R-006 |
| `--version` | (flag) | — | no | print version, exit 0 | CL-R-001 |
| `--help` | (flag) | — | no | print usage, exit 0 | CL-R-001 |

- Top-level has **no** `--ocpp` and **no** `--exit-on-error`. OCPP modules come only from `--session` here (CL-R-014, CL-R-015).
- Resolution order: `--session` instances, then `--module`, then `--device`. Names de-duplicated across all sources and both module types (later duplicates get ` (2)`, ` (3)`, …) (CL-R-007).
- `--demo` produces `Modbus Server`, `Modbus Client`, and `CSMS`/`CS` pairs for OCPP `v1.6` (port 9000), `v2.0.1` (9001), `v2.1` (9002) (CL-R-006).

---

## Subcommands

### `ferrowl migrate`

```
ferrowl migrate --input FILE --output FILE
ferrowl migrate -i FILE -o FILE
```

| Flag | Short | Value | Required | Purpose | Req |
|---|---|---|---|---|---|
| `--input` | `-i` | `FILE` (.toml/.json) | yes | legacy (≤ v0.3.9 `modbus-cli-rs`) config to read | CL-R-011 |
| `--output` | `-o` | `FILE` (.toml/.json) | yes | destination for the converted device config | CL-R-011 |

Dispatched before any async runtime; converts a legacy device config to the current format; warnings for dropped/approximated fields and the success line go to **stderr**. Exits directly (0 success, 1 failure) (CL-R-012). Input and output encodings each from their own extension. Transformation contract CS-R-040…CS-R-045. Device-config files only — never session files.

### `ferrowl run` (headless / CI)

```
ferrowl run [--session FILE]... [--module KEY=VAL,...]... [--ocpp KEY=VAL,...]... \
            [--duration SECS] [--log-file FILE] [--exit-on-error]
```

| Flag | Value | Default | Repeatable | Purpose | Req |
|---|---|---|---|---|---|
| `--session` | `FILE` | — | yes | session file; supplies Modbus and OCPP instances and session scripts | CL-R-013 |
| `--module` | `KEY=VAL,...` | — | yes | ad-hoc Modbus module (``## `--module` descriptor mini-language (Modbus)``) | CL-R-013 |
| `--ocpp` | `KEY=VAL,...` | — | yes | ad-hoc OCPP module (``## `--ocpp` descriptor mini-language (OCPP)``) | CL-R-013, CL-R-014, CL-R-046 |
| `--duration` | `SECS` (integer) | none | no | run this many seconds then exit 0. Omit → until Ctrl-C | CL-R-024 |
| `--log-file` | `FILE` | none | no | append every drained line to this file (create-and-append) in addition to stdout | CL-R-041 |
| `--exit-on-error` | (flag) | off | no | exit 3 (after stopping all modules) when a drained line has level Error | CL-R-015, CL-R-031 |

- `--device` **not** available on `run`; use `--module` (CL-R-047).
- `--exit-on-error` exists **only** on `run` (CL-R-015).

---

## `--module` descriptor mini-language (Modbus)

Comma-separated `key=value` pairs. Whitespace around keys and values trimmed; empty comma segment skipped. Segment without `=` is an error. Later duplicate keys overwrite earlier.

| Key | Required | Default | Meaning | Req |
|---|---|---|---|---|
| `name` | yes | — | instance/tab name and `C_Module` registry key | CL-R-002 |
| `device` | yes* | — | device-config file path | CL-R-002 |
| `type` | — | — | **alias for `device`**: used only if `device` absent | CL-R-002 |
| `role` | — | `server` | `client` or `server`. Other → error | CL-R-002 |
| `transport` | — | `tcp` | `tcp`, `rtu`, `rtu_over_tcp`, `udp`, `ascii`, `ascii_over_tcp`. Other → error | CL-R-002 |
| `ip` | — | `127.0.0.1` | TCP/UDP only: peer/bind IP | CL-R-002 |
| `port` | yes (tcp) | — | TCP/UDP only. Required for `transport=tcp`, `rtu_over_tcp`, `udp`, `ascii_over_tcp`; numeric | CL-R-002 |
| `path` | yes (rtu) | — | RTU only: serial device path. Required for `transport=rtu` or `ascii` | CL-R-002 |
| `baud` / `baud_rate` | — | `19200` | RTU only: baud rate (aliases) | CL-R-002 |
| `parity` | — | unset | RTU only: parity string (passed through) | CL-R-002 |
| `data_bits` | — | unset | RTU only: data bits (numeric) | CL-R-002 |
| `stop_bits` | — | unset | RTU only: stop bits (numeric) | CL-R-002 |

\* `device` required, but `type` may supply it. At least one of `device`/`type` must be present (CL-R-002).

- **Default role `server`** for `--module` (contrast `--device`, always a client) (CL-R-002, CL-R-044).
- `port` required for TCP, **no** default; `path` required for RTU (CL-R-002).
- RTU keys here (`baud`, `parity`, `data_bits`, `stop_bits`, …) are this mini-language's own keys, not clap short flags — [`edge-cases.md`](./edge-cases.md) RTU/clap collision.

Example:

```
--module name=evse-1,device=configs/evse.toml,transport=tcp,ip=10.0.0.5,port=502,role=server
```

---

## `--ocpp` descriptor mini-language (OCPP)

Same grammar as ``## `--module` descriptor mini-language (Modbus)``. Role/version/timeout/security/scripts are **not** on the command line — they come from the device file.

| Key | Required | Default | Meaning | Req |
|---|---|---|---|---|
| `name` | yes | — | instance name / registry key | CL-R-014 |
| `device` | yes | — | OCPP device-config file path | CL-R-014 |
| `protocol` | — | `ws` | `ws` or `wss`. Other → error | CL-R-014 |
| `ip` | — | `127.0.0.1` | peer/bind IP | CL-R-014 |
| `port` | yes | — | port; numeric | CL-R-014 |
| `path` | — | empty string | WebSocket path (e.g. `/ocpp/cp001`) | CL-R-014 |

Example:

```
--ocpp name=cs-1,device=configs/cs.toml,protocol=ws,ip=127.0.0.1,port=9000,path=/ocpp/cp001
```

---

## Exit-code table

### `ferrowl run`

| Code | Meaning | Req |
|---|---|---|
| `0` | ran to completion: `--duration` reached, or Ctrl-C (SIGINT). No error condition fired | CL-R-032 |
| `1` | setup failure: device config failed to load, `start` reported an error, `--session` failed to load/parse, or `--log-file` could not be opened. `Error: …` on stderr; started modules stopped first | CL-R-030, CL-R-049, CL-R-050 |
| `2` | argument-parser usage error (e.g. unknown flag) — emitted before the run | CL-R-035 |
| `3` | `--exit-on-error` set **and** a drained line had level Error. All modules stopped, then exit 3 | CL-R-031 |

### `ferrowl migrate` exit codes

| Code | Meaning | Req |
|---|---|---|
| `0` | conversion succeeded; output written | CL-R-033 |
| `1` | failure: unrecognized input/output extension, input parse failure, or output write failure. `error: …` on stderr | CL-R-033 |

### Top-level / parser (all commands)

| Code | Meaning | Req |
|---|---|---|
| `0` | `--help` or `--version` displayed | CL-R-001 |
| `2` | argument-parser usage error (unknown flag, missing required option) | CL-R-035 |

---

## Headless output format

- **stdout** carries the drained log stream, one line per entry: `[<timestamp>] <source> | <message>`. `<source>` = module's deduped name, or `session` for session-sim lines (CL-R-040).
- **stderr** carries setup/fatal diagnostics (`Error:`/`error:`, the TUI's module-skip warnings) and the per-module teardown lines `Stopped '<name>'` / `Error: failed to stop '<name>': <detail>` (CL-R-055, CL-R-056, CL-R-057), so stdout stays parseable (CL-R-042).
- With `--log-file FILE`, every stdout line is also appended to `FILE` (create-and-append) (CL-R-041).
