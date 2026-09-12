# CLI & Headless — Requirements

The ferrowl **process command line** and the **headless runner** (`ferrowl run`): argument surface (top-level flags, subcommands), `--module`/`--ocpp` key=val mini-language, headless run lifecycle and exit-code contract, headless stdout/stderr contract.

Per [`../README.md`](../README.md)'s ownership rules, this area does **not** own: the in-TUI `:` command line ([`../tui/`](../tui/)); config/session **file format** and `migrate` **transformation** ([`../config-session/`](../config-session/)) — only the `migrate` *CLI surface* (flags, invocation, exit behavior); what Lua scripts and `C_Test:Assert` **do** ([`../scripting/`](../scripting/)) — only how a logged script error maps to an exit code; protocol behavior ([`../modbus/`](../modbus/), [`../ocpp/`](../ocpp/)).

---

## Argument surface (top-level command)

**CL-R-001** — The program presents `--version` (print build version, exit 0) and `--help` (print usage, exit 0), taking precedence over starting the TUI.

**CL-R-002** — The top-level command accepts a repeatable `--module` option whose value is a `key=val,...` module descriptor ([`api-contract.md`](./api-contract.md)). Each occurrence contributes one Modbus module instance, in command-line order.

**CL-R-003** — The top-level command accepts a repeatable `--session` option naming a session file.

**CL-R-053** — Each session file named by `--session` (CL-R-003) has its instances resolved, and every one of them contributes to the started set.

**CL-R-054** — Session instances (CL-R-003) resolve before `--module` instances.

**CL-R-004** — The top-level command accepts a repeatable `--device` option naming a device-config file.

**CL-R-044** — Each `--device` occurrence (CL-R-004) contributes one auto-built Modbus **TCP client** named `Device <n>` (n = 0-based index) at the fixed endpoint `127.0.0.1:5020`.

**CL-R-045** — `--device` (CL-R-004) exposes no endpoint or role control.

**CL-R-005** — The top-level command accepts a boolean `--demo`. When set, the program starts a fixed set of built-in demo tabs and ignores `--module`, `--session`, `--device` for tab building.

**CL-R-006** — With `--demo`, the demo set is exactly eight tabs: two Modbus (one server, one client) and six OCPP (client and server for each of 1.6, 2.0.1, 2.1). Each is started, and the demo session loads one example session-level Lua script.

**CL-R-007** — The final instance set is the concatenation, in order, of `--session` instances, then `--module`, then `--device`. Names are de-duplicated across all sources and both module types per CS-R-058: first occurrence keeps its name; later duplicates get ` (2)`, ` (3)`, ….

---

## Subcommands

**CL-R-010** — Two subcommands, `migrate` and `run`. Invoking one replaces the default action (starting the TUI).

**CL-R-011** — `migrate` requires `--input`/`-i` and `--output`/`-o`, each naming a file: read `--input`, write the converted device config to `--output`. The transformation is [`../config-session/`](../config-session/) (CS-R-040…CS-R-045); this area specifies invocation and exit behavior.

**CL-R-012** — `migrate` is dispatched before any async runtime is created, never starts the TUI or a headless run, and exits the process directly with its own code (0 success; non-zero per CL-R-032).

**CL-R-013** — `run` (headless) accepts: repeatable `--session`, `--module`, `--ocpp`; optional `--duration` (seconds); optional `--log-file`; boolean `--exit-on-error`.

**CL-R-046** — On `run`, Modbus modules resolve from `--session` + `--module`, OCPP modules from `--session` + `--ocpp`, same descriptor mini-language as the top-level command.

**CL-R-014** — `--ocpp` (ad-hoc OCPP descriptor) is accepted **only** on `run`; the top-level command resolves OCPP modules solely from `--session`.

**CL-R-047** — `--device` is accepted only on the top-level command, never on `run`.

**CL-R-015** — `--exit-on-error` is accepted only on `run`.

**CL-R-016** — Top-level `--module`/`--session`/`--device`/`--demo` values supplied alongside a `run` (or `migrate`) subcommand do not affect it: the runner reads only the subcommand's own flags.

---

## Headless run lifecycle

**CL-R-020** — `ferrowl run` builds the same module views the TUI builds and starts each via the module's `start` command, but never enters the alternate screen, reads the terminal, or renders.

**CL-R-021** — Unlike the TUI (which skips a module whose device config fails to load), the headless runner treats any module's device-config load failure or `start` error as fatal to startup: no partial module set (CL-R-030).

**CL-R-022** — After all modules start, the runner loops on a fixed ~100 ms tick.

**CL-R-048** — Each tick of the headless runner loop (CL-R-022): refresh every module, drain each module's newly appended log lines to stdout (and, if configured, the log file), evaluate stop conditions.

**CL-R-023** — When `--session` files supply at least one **enabled** session-level script, the runner also runs the session-level sim (resolved cycle interval) and drains its log under source name `session`. No enabled script → no session sim.

**CL-R-024** — With `--duration <secs>`, the run exits cleanly once elapsed time reaches the deadline, evaluated at the end of a tick. Without it, the run continues until Ctrl-C (SIGINT).

**CL-R-025** — Ctrl-C (SIGINT) during the run ends the loop as a clean shutdown (exit 0).

**CL-R-026** — On any loop exit, the runner stops the session sim (if any) then every module before returning. A stop failure is logged, exit code unchanged.

**CL-R-027** — Session-level scripts across multiple `--session` files are concatenated in file order; the session sim interval is the last file's, matching the TUI's multi-file resolution.

**CL-R-055** — Each module the headless runner stops during teardown (CL-R-026, CL-R-050) is reported on stderr as `Stopped '<name>'`, `<name>` = the module's deduped name, one line per module, after that module's stop completes.

**CL-R-056** — A module whose stop reports an error is reported on stderr as `Error: failed to stop '<name>': <detail>` instead of CL-R-055's line.

**CL-R-057** — The session sim, when stopped during teardown (CL-R-026), is reported under source name `session` by the same lines as a module (CL-R-055, CL-R-056).

---

## Exit codes

**CL-R-030** — `ferrowl run` returns **1** for any setup failure: a module's device config failed to load, a module's `start` reported an error, a `--session` file failed to load or parse, or `--log-file` could not be opened.

**CL-R-049** — On a setup failure (CL-R-030), `ferrowl run` writes a diagnostic beginning `Error:` to stderr.

**CL-R-050** — On a setup failure (CL-R-030), `ferrowl run` stops already-started modules before returning.

**CL-R-031** — `ferrowl run` returns **3** if and only if `--exit-on-error` is set **and** a drained log line has level Error. On detection the runner stops every module then exits 3. Without `--exit-on-error`, an error line never changes the exit code.

**CL-R-032** — `ferrowl run` returns **0** for a run reaching its `--duration` deadline or interrupted by Ctrl-C without CL-R-031's exit-code-3 condition having fired.

**CL-R-033** — `migrate` exits **0** on success and **1** on failure (unrecognized input/output extension, input parse failure, output write failure), writing a diagnostic beginning `error:` to stderr. Never exit code 2 or 3.

**CL-R-034** — A `C_Test:Assert` failure (or any Lua sim error) does **not** by itself fail a headless run: it surfaces only as an Error-level log line (`[sim]`-prefixed), influencing the exit code only with `--exit-on-error` (code 3, CL-R-031).

**CL-R-035** — An argument-parsing error (unknown flag, missing required subcommand option, malformed option value) aborts before any run, printing a usage diagnostic to stderr and exiting with the parser's standard usage code (2).

**CL-R-051** — `--help`/`--version` exit 0.

---

## Output contract (headless)

**CL-R-040** — Each drained log line prints to stdout as `[<timestamp>] <source> | <message>`, `<source>` = the module's (deduped) name, or `session` for session-sim lines.

**CL-R-041** — With `--log-file <path>`, every stdout line is also appended to that file, opened create-and-append.

**CL-R-042** — Setup and fatal diagnostics (`Error:`/`error:` of CL-R-030 and CL-R-033, the TUI's module-skip warnings) and the teardown lines of CL-R-055, CL-R-056 and CL-R-057 go to stderr, keeping stdout the machine-readable drained-log stream.

**CL-R-043** — Per-module draining is exact-by-count (tracking total lines written), so a message repeated verbatim within one drain window is emitted every time.

**CL-R-052** — If more lines were written to a module's log between ticks than the ring holds, the per-module drain (CL-R-043) reports the overflow with a synthetic `(<n> lines dropped: ring overflowed between ticks)` line.
