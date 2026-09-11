# Scripting — Edge Cases and Known Limitations

Boundary behavior, error semantics, intentional or known constraints. The known-limitations section below (`## Known limitations and findings`) is working as implemented; recorded so it is not "fixed".

---

## Load-time vs run-time errors

| ID | Condition | Behavior |
|---|---|---|
| **SC-E-001** | Script contains a Lua **syntax** error | whole context fails to build; sim thread logs one "failed to build Lua context" error, does not loop. **No** script in that context runs, including valid ones |
| **SC-E-002** | Two enabled scripts share a name | context build fails the same way (all-or-nothing); the in-TUI editor prevents duplicate names, a hand-edited file can still trigger it |
| **SC-E-003** | Script raises at **run time** (`error`, failed `C_Test:Assert`/`Fail`, rejected write, malformed override table) | only that script's cycle aborted; logged at Error level with a sim/Lua prefix; every other script still runs that cycle |
| **SC-E-004** | Script raises every cycle | logged every cycle; never disabled automatically |
| **SC-E-005** | Script references a `C_*` module not registered in its context (e.g. `C_Register` from an OCPP sim) | run-time error: indexing a nil global. Logged, cycle continues |

## State access and type coercion

| ID | Condition | Behavior |
|---|---|---|
| **SC-E-006** | `C_Register:Get` on an unknown name | error `unknown register '<name>'` |
| **SC-E-007** | `C_Register:Get` on a virtual register never written | error `virtual register '<name>' not set` |
| **SC-E-008** | `C_Register:Get` on a fixed register whose cells are unreadable | error `register '<name>' not readable` |
| **SC-E-009** | `C_Register:Set` with `nil` | error `cannot Set nil value` |
| **SC-E-010** | `C_Register:Set` integer out of the format's range (e.g. `100000` onto `U16`) | error, no truncation |
| **SC-E-011** | `C_Register:Set` fractional float onto an integer format (e.g. `3.5`) | error `not a whole number` |
| **SC-E-012** | `C_Register:Set` whole-number float onto an integer format (e.g. `42.0`) | accepted, stored as the integer |
| **SC-E-013** | `C_Register:Set` boolean | treated as integer `0`/`1`, then coerced to the format |
| **SC-E-014** | `C_Register:Set` string | parsed through the register's string-input codec (numeric-literal rules apply) |
| **SC-E-015** | `C_Register:Set` on a **virtual** register | declared format ignored: in-range integer stored as 64-bit int, out-of-range integer falls back to float, float stays float, string codec-parsed |
| **SC-E-016** | `C_Register:Has` on any name | `true`/`false`, never errors; reflects *definition*, not readability |
| **SC-E-017** | Lua table/function where a scalar is expected (`Set`, `C_Statics:Get` arg, action override value) | conversion error `expected number, string or boolean` |
| **SC-E-018** | `C_OCPP` action override table with a nested table value | whole action call raises |
| **SC-E-019** | `C_OCPP` server `ChargingStation`/`Connector` for an unknown station/connector | returns `nil`; indexing that `nil` is the script's own error |
| **SC-E-020** | `C_Module:Get` for an unknown/removed module | raises `unknown module '<name>'` |
| **SC-E-021** | `ModuleHandle:Register()` on a non-modbus module / `:OCPP()` on a non-ocpp module | raises `is not a modbus module` / `is not an ocpp module` |

## Concurrency with the network task

| ID | Condition | Behavior |
|---|---|---|
| **SC-E-022** | Script reads/writes register state while the client polls or the server answers | each `Get`/`Set` individually lock-guarded and atomic; sim thread and network task share the same locked store |
| **SC-E-023** | Script does read-modify-write across two calls | not transactional: a concurrent host update can land between |
| **SC-E-024** | Lua write to a register on a Modbus **client** | writes the in-memory store only; **no** Modbus write command; next poll may overwrite |
| **SC-E-025** | Lua write to a register on a Modbus **server** | updates the served store; a remote master reads the new value |
| **SC-E-026** | Script runs while its network instance is stopped/disconnected | sim keeps running; writes land in the store (nothing on the wire) |

## Sim lifecycle

| ID | Condition | Behavior |
|---|---|---|
| **SC-E-027** | All scripts disabled (or none) | no sim thread |
| **SC-E-028** | Script edited / toggled / interval changed | sim thread stopped and a fresh one started; **all globals reset** |
| **SC-E-029** | Cycle interval non-finite/≤0 in config | falls back to 1.0 s |
| **SC-E-030** | Per-module interval below 0.05 s | floored to 0.05 s; session-level interval has no floor |
| **SC-E-031** | `C_Time:Get`/`GetMs` right after a restart | counts from ~0 again (origin = context build time) |
| **SC-E-040** | Script enabled with an empty code body | handed to a sim thread like any other enabled script (SC-R-061); the thread runs and its cycles execute nothing |

---

## Known limitations and findings

### Execution ceiling — an infinite loop is interrupted mid-cycle

**SC-E-032** — Every context installs an execution hook (SC-R-034, `every_nth_instruction` every 1,000 instructions) checking the stop flag (SC-R-046) and enforcing a fixed 1,000 ms wall-clock cap (SC-R-047). A script looping forever is interrupted the first time the hook fires after either condition is met: a pending stop-and-join (script edit, tab close, module reconfigure, app shutdown) makes the hook raise to unwind the runaway script, so the cycle ends and the join completes promptly — before this hook the stop flag was observed only *between* cycles, so a runaway script blocked any thread joining it, typically the UI thread; independent of any stop request, the 1,000 ms cap aborts a cycle (or on-demand run, SC-R-035) that runs long, bounding worst-case CPU pinning to just over the cap with no operator action. Both constants fixed (SC-R-034, SC-R-047), not configurable. No memory ceiling (SC-R-048): a script allocating without bound (ever-growing table) is not stopped — the hook checks instruction count and wall-clock only.

### Lua register writes are store-only (client)

**SC-E-033** — `C_Register:Set` on a Modbus client updates the in-memory store and never emits a Modbus write command, unlike `:set`. The value is transient; the next successful poll of that address overwrites it. Intended (a sim script models the device's *own* state), but a client-side Lua write is not observable by the remote peer.

### Script execution order within a cycle is unspecified

**SC-E-034** — A context stores scripts in a hash map and runs them in hash-iteration order, not definition order, not stable. Scripts must not depend on running before or after a sibling within a cycle.

### A fresh Lua state on every restart

**SC-E-035** — Every script/interval edit rebuilds the context from scratch: no persistent Lua state across restarts or config reload. The only durable state is what a script writes into host register/OCPP state via `C_*`.

### No script return value, no scheduling primitives

**SC-E-036** — A script is a nullary function whose return is ignored. No per-script scheduling beyond the context's single cycle interval, no timers, no callbacks, no cooperative yield or sleep; a script acting less often than every cycle must gate itself with `C_Time`.

### `C_Statics` is unreachable

**SC-E-037** — `C_Statics` is in the scripting library but no ferrowl sim registers it, so no ferrowl script can call it. Specified in the API contract for completeness only.

### Session `C_Module` staleness is surfaced, not cached

**SC-E-038** — A `ModuleHandle` from `C_Module:Get` re-resolves its target on every method call. If the module is removed between obtaining and using the handle, the next call raises `unknown module` rather than returning a stale accessor.

### A run-once (`e`) executes in an isolated Lua VM

**SC-E-039** — The on-demand single-script execution (SC-R-035, `e` in the script-manager dialog) builds a **fresh** context on its own thread and shares no Lua state with the owner's running sim: sim globals are invisible to the run, the run's globals are discarded when its thread exits, `C_Time` restarts from zero. A script depending on state built over previous sim cycles behaves differently under `e`. The run touches the same shared register/charging-station state as a concurrent sim, serialized only by per-operation locks; a run-once and a sim cycle interleaving writes to the same register is possible and not prevented.
