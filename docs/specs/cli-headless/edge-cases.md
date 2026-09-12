# CLI & Headless — Edge Cases & Known Limitations

Boundary and error behavior of the process command line and headless runner, plus known limitations. Config-file and `migrate` transformation cases: [`../config-session/`](../config-session/); Lua/assertion semantics: [`../scripting/`](../scripting/).

---

## Argument-parsing errors

- **CL-E-001** — **Unknown flag / missing required option** — parser aborts before any run or migration, prints a usage diagnostic to stderr, exits **2**. No modules started.
- **CL-E-002** — **`--help` / `--version`** — printed to stdout, exit **0**, no run.
- **CL-E-003** — Parser usage error (**2**) and `--exit-on-error` trip (**3**) are distinct codes, so CI can tell "mistyped a flag" from "the run detected an error".

## Malformed `--module` / `--ocpp` descriptors

- **CL-E-004** — **Segment without `=`** (`name=m,oops,device=d`) → parse error.
- **CL-E-005** — **Empty comma segment** (`name=m,,device=d`) → skipped, not an error.
- **CL-E-006** — **Missing required key** — `--module` without `name`, or without both `device` and `type`; TCP `--module` without `port`; RTU `--module` without `path`; `--ocpp` without `name`, `device`, or `port` → parse error.
- **CL-E-007** — **Non-numeric `port`/`data_bits`/`stop_bits`/`baud`** → parse error.
- **CL-E-008** — **Invalid enum value** — `role` other than `client`/`server`, `transport` other than `tcp`/`rtu`/`rtu_over_tcp`/`udp`/`ascii`/`ascii_over_tcp`, `protocol` other than `ws`/`wss` → parse error.
- **CL-E-009** — In `ferrowl run`, any such error is a setup failure: exit **1** with `Error:` on stderr before the loop. In the TUI path it aborts startup with `Error:` on stderr.

## `run` with no modules / no session

- **CL-E-010** — **`ferrowl run` with no `--module`, `--ocpp`, or `--session`** — empty module set, no session sim; without `--duration` idles until Ctrl-C then exits **0**; with `--duration N` exits 0 after N seconds with no drained lines. Not rejected.
- **CL-E-011** — **`--session` file with no enabled session script** — no session sim; only per-module logs drained.

## `--duration` boundaries

- **CL-E-012** — **`--duration 0`** — deadline is "now", checked only at the **end** of the first tick, so the run executes roughly one ~100 ms tick (one refresh + drain) then exits **0**.
- **CL-E-013** — **Very large `--duration`** — accepted (seconds); no upper clamp.
- **CL-E-014** — Ctrl-C always short-circuits `--duration`, exits **0**.

## Session / device load failures in headless

- **CL-E-015** — **`--session` file fails to load or parse** — setup failure, exit **1** (`Error:` on stderr). Stricter than the TUI, which tolerates a file vanishing mid-startup by falling back to defaults.
- **CL-E-016** — **A module's device config fails to load** — headless exits **1** (`'<name>': failed to load '<path>': …` under `Error:`). The TUI skips that module with a stderr warning and keeps the rest (CS-E-008). Deliberate asymmetry: headless must not silently run a partial set in CI.
- **CL-E-017** — **Blank `device` path** — for OCPP a legitimate quick-start on the default device config; CS-R-067 governs.

## `--exit-on-error` detection is level-based

- **CL-E-018** — Keys off the drained line's level (`Level::Error`), not message text. A Lua error never reaching the log, or logged lower, does not trip it.
- **CL-E-019** — Assertions: `C_Test:Assert` failures surface through the sim's `[sim] <error>` line at Error. Without `--exit-on-error`, an assertion failure does **not** change the exit code. CI that must fail on assertions passes `--exit-on-error` ([`../scripting/`](../scripting/)).

## `--log-file`

- **CL-E-020** — **Unopenable path** (bad directory, permission denied) — runner stops already-started modules, exits **1** with `Error: failed to open --log-file …`.
- **CL-E-021** — **Existing file** — create-and-append: prior content preserved, new lines appended.

## Log draining under load

- **CL-E-022** — Exact-by-count from a monotonic written-line counter, so a message repeated verbatim within one window is fully emitted.
- **CL-E-023** — If more lines are written between ticks than the ring holds (~80), the oldest overflow is gone; the runner emits a synthetic `(<n> lines dropped: ring overflowed between ticks)` line rather than under-counting. A sim logging faster than one tick drains loses the *content* of the oldest lines while accounting for their count.

## Top-level flags alongside a subcommand

- **CL-E-024** — Top-level `--module`/`--session`/`--device`/`--demo` with a `run` or `migrate` subcommand is accepted by the parser but has **no effect**: the subcommand reads only its own flags. `ferrowl --module X run --duration 1` runs headless with **no** modules.

---

## Known limitations (stated, not bugs)

- **CL-E-025** — **The RTU `Config` / clap short-flag collision.** The Modbus RTU settings struct in `ferrowl-modbus` doubles as a clap `Args` group with auto-derived short flags. Two collide: `-s` (`slave` and `stop_bits`), `-d` (`data_bits` and `delay_ms`). Flattening this group into a `clap::Parser` command panics at parse time via clap's debug assertions. **Latent**: the shipped CLI does **not** flatten it; RTU parameters come through the `--module …,transport=rtu,path=…,baud=…` mini-language ([`api-contract.md`](./api-contract.md) ``## `--module` descriptor mini-language (Modbus)``), which has no short-flag collision. Documented in source, intentionally unfixed. (Equivalent latent collision in the TCP settings `Args` group for `timeout`/`delay`/`interval` short flags.)
- **CL-E-026** — **`migrate`'s `-i`/`-o` are the only short options.** Top-level flags and every `run` flag are long-only; no `-m`/`-s`/`-d` short forms (sidesteps the collision above).
- **CL-E-027** — **`--exit-on-error` only catches logged Error-level lines.** A level match, not a structured result channel (``## `--exit-on-error` detection is level-based``). Errors never reaching the log, or logged below Error, are invisible.
- **CL-E-028** — **Headless has no per-module error isolation.** Any one module's startup failure fails the whole `run` with exit 1 (`## Session / device load failures in headless`); no "start the good ones, report the bad" mode.
- **CL-E-029** — **Teardown lines are stderr, not the drained log stream.** CL-R-055's `Stopped '<name>'` lines never reach stdout and are therefore never mirrored into `--log-file` (CL-R-041): a `--log-file` capture shows the run's logs but no teardown record. Deliberate — stdout stays a pure drained-log stream (CL-R-042), and the setup-failure teardown happens before any drain has run at all.
