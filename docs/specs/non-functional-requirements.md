# Non-Functional Requirements

Cross-cutting properties holding across every area. Per-area behavior: each area's `requirements.md`.

IDs stable, append-only (`NF-R-nnn`). See [`README.md`](./README.md).

## Platforms

**NF-R-001** — Linux and Windows prebuilt binaries are produced by the nightly pipeline. Windows is cross-compiled to `x86_64-pc-windows-gnu`; development and CI run primarily on Linux.

**NF-R-002** — No macOS binaries are built by CI. Nothing in the stack is Linux-specific beyond the build pipeline itself.

**NF-R-003** — The toolchain is stable Rust, edition 2024, pinned by `rust-toolchain.toml` (patch version unpinned).

## Performance posture

**NF-R-010** — No explicit performance targets or benchmarks are asserted. The hot register read/write path stays on `parking_lot` synchronous locks, not `tokio::sync`.

**NF-R-011** — Each Lua sim runs on its own dedicated OS thread, isolated from the tokio runtime and the UI redraw loop, so a slow script cannot stall polling or rendering.

**NF-R-046** — A Lua sim has no execution ceiling; this is a known limitation ([`scripting/edge-cases.md`](./scripting/edge-cases.md)).

**NF-R-067** — No module lifecycle `:` command blocks the TUI's input and redraw loop; it completes asynchronously, and a stop-bearing command's outcome reports into the module's message log rather than its immediate result, the one bounded exception being the tab-close settle of UI-R-316 ([`tui/`](./tui/), UI-R-314, UI-R-315).

## Reliability

**NF-R-020** — A Modbus client auto-reconnects with exponential backoff bounded to 1s–30s ([`modbus/`](./modbus/)).

**NF-R-021** — An OCPP CS connection auto-reconnects using the Modbus client's bounded exponential-backoff policy (MB-R-051), gated by the CS's `reconnect` config field (default enabled) (OC-R-048).

**NF-R-047** — An OCPP CSMS applies the same bounded exponential-backoff policy (MB-R-051) to a failed listener bind (OC-R-139).

**NF-R-022** — A Lua script error never crashes its host module (SC-R-032).

**NF-R-068** — Stopping a module abandons an in-flight connection attempt, listener bind, or serial-port open rather than waiting for it to succeed or time out (MB-R-220, MB-R-221, OC-R-175, OC-R-176, OC-R-177; a synchronous serial open is honored at the first surrounding await, MB-E-091).

## Security posture

**NF-R-030** — OCPP supports TLS (including mutual TLS) and HTTP Basic Auth.

**NF-R-048** — Modbus/TCP optionally supports TLS, including mutual TLS, via an opt-in `tls` config field (MB-R-104–MB-R-111, MB-R-161–MB-R-178).

**NF-R-049** — Modbus RTU has no transport security.

**NF-R-031** — Lua sim scripts run in a restricted sandbox with no access to host filesystem, shell, environment, or dynamic code loading (SC-R-040).

**NF-R-050** — A Lua sim script's wall-clock execution time is capped (SC-R-047).

**NF-R-051** — A Lua sim script has no memory ceiling; this is a known limitation (SC-R-048).

**NF-R-032** — A credential comparison during peer authentication (e.g. OCPP CSMS Basic Auth) runs in constant time with respect to the secret.

## Path handling

**NF-R-042** — Any user-supplied filesystem path (CLI flag argument, path-valued config/session/device field, TUI dialog path field) has a leading `~` expanded to the current user's home directory before it is opened, read, written, or checked for existence: bare `~` = home directory, `~/rest` = `<home>/rest`.

**NF-R-052** — In `~` expansion (NF-R-042), a path not starting with `~` (including `~otheruser/...`, unsupported) passes through unchanged, as does any path when the home directory is undeterminable.

**NF-R-053** — `~` expansion (NF-R-042) is performed once by a single shared resolver, applied at every filesystem-touching call site.

**NF-R-054** — The filesystem-touching call sites that apply `~` expansion (NF-R-042) are: config/session/device config files (`ferrowl-util::convert::Converter`), CLI `--session`/`--device`/`--module`/`--log-file`, per-module log files, Modbus/OCPP TLS cert/key/CA files (including the setup dialogs' path-existence validation).

## Versioning & testing

**NF-R-040** — All workspace crates are versioned in lockstep; no crate is published independently.

**NF-R-041** — Unit tests are colocated with the code under test (`#[cfg(test)] mod tests`, `ut_*` naming where practical); integration tests in each crate's `tests/`.

**NF-R-055** — CI runs `cargo check` + `cargo test` on every push; a tag-triggered nightly workflow additionally builds and publishes release binaries.

**NF-R-056** — `lefthook` enforces `cargo fmt --check` and `cargo clippy -D warnings` pre-commit.

**NF-R-043** — A test that needs a TCP or UDP port obtains it from `reserve_tcp_port() -> TcpPortGuard` or `reserve_udp_port() -> UdpPortGuard`, which bind `127.0.0.1:0` and **hold the binding** open until the code under test takes it over.

**NF-R-064** — A port guard (NF-R-043) exposes `port() -> u16`, the port of the binding it holds.

**NF-R-065** — A port guard (NF-R-043) exposes `into_listener() -> std::net::TcpListener` (TCP) / `into_socket() -> std::net::UdpSocket` (UDP), handing the already-bound socket to a server that can adopt one.

**NF-R-066** — A port guard (NF-R-043) exposes `release() -> u16`, yielding the port for a server that can only bind by port number.

**NF-R-057** — The port guard's `release()` (NF-R-043) is the sole sanctioned path that reopens a time-of-check/time-of-use window; every call site uses `into_listener()`/`into_socket()` where the server accepts a bound socket, and `release()` only where it does not.

**NF-R-058** — No test computes a port by binding `:0`, reading `local_addr()`, and dropping the socket inline; the bare `free_port() -> u16` shape is removed from every crate.

**NF-R-059** — Exempt from the port-guard rule (NF-R-043, NF-R-058) is the deliberate port-**occupier** pattern (bind ephemerally, keep the binding alive, point a server at the same port to exercise bind-failure handling), where the collision is the assertion.

**NF-R-044** — No test reads or writes a path directly under `std::env::temp_dir()`. A test needing filesystem scratch space obtains it from `reserve_temp_dir(prefix: &str) -> TempDirGuard`, which creates a directory under `std::env::temp_dir()` whose name is unique per run (`prefix`, the current process id, and a monotonically increasing per-process counter) and exposes `path() -> &std::path::Path` and `join(name: impl AsRef<Path>) -> std::path::PathBuf`.

**NF-R-060** — Creation of the temp-dir guard's directory (NF-R-044) is exclusive: the guard fails if the derived directory already exists and retries with the next counter value, so a stale directory left by an earlier process whose id has been reused is never adopted, and panics rather than hand back a directory it did not create.

**NF-R-061** — Dropping the temp-dir guard (NF-R-044) removes the directory and its contents best-effort; a removal failure is ignored, never a panic.

**NF-R-062** — Fixed filenames are joined onto the temp-dir guard's unique directory (NF-R-044), so concurrent test binaries, two checkouts, and a rerun overlapping the previous run's cleanup never address the same absolute path.

**NF-R-063** — Exempt from the temp-dir rule (NF-R-044) are `ferrowl-test-support`'s own tests of `reserve_temp_dir`/`TempDirGuard`, which address paths directly under `std::env::temp_dir()` because there the raw temp path is the assertion.

**NF-R-045** — The fixtures of NF-R-043 and NF-R-044 live in a single workspace crate, `ferrowl-test-support`, consumed only as a `dev-dependency`; no crate redefines them locally. The crate carries `publish = false` and is versioned in lockstep (NF-R-040).
