# Ferrowl Specs

Authoritative spec of `ferrowl` behavior, by capability area. Areas, ID prefixes and the rules of engagement (code conforms to spec, behavior change needs a spec change, the gated workflow): [`AGENTS.md`](../../AGENTS.md) `## Spec-driven` and `## Where to look for task X`.

## Rules for writing specs

1. **No code pointers.** Never cite file:line, function/type names, crate-internal identifiers — specs state *what must be true*, not where (pointers rot on refactor). Exception: public, user-facing surface (config keys, `:` commands, Lua `C_*` API, CLI flags, OCPP action names, exported signatures, error variants, feature flags) IS spec → area's `api-contract.md`.
2. **IDs stable, append-only.** `<PREFIX>-R-nnn` for a requirement, `<PREFIX>-E-nnn` for an `edge-cases.md` entry; prefix per area from `AGENTS.md`'s routing table (`NF` has no `-E` series). The two series are numbered independently per area; never renumber, never reuse a retired ID. An `-E` entry derived from a requirement cites that `-R` ID in its text and still carries its own `-E` ID. Tests cite `-E` IDs exactly as `-R` IDs (rule 8).
3. **Owner = the behavior, not the surface.** A Modbus RTU config field is specified in `modbus/`, not `config-session/` (one change → one file). `config-session/` owns only the *envelope*: file format, `version`, session→module list, save/load, `migrate`. `tui/` owns the command mechanism and generic commands; protocol-specific commands live in their protocol's area. Shared behavior → owning area, stated once.
4. **Requirements are testable.** Normative indicative statements, observable outcomes. Good: "The client retries with exponential backoff bounded to 1s–30s." Bad: "The client is robust." Format requirements name exact bytes where possible.
5. **Known gaps are specified, not hidden.** Intentional-but-ugly behavior (no Lua execution ceiling, no OCPP auto-reconnect) → area's `edge-cases.md` as a stated constraint, so it isn't "fixed".
6. **One entry, one physical line** (`AGENTS.md` `## Conventions — text`), table rows included. Headings are unnumbered: `extract-section.sh` matches heading text verbatim, and a number churns every reference when a section is inserted.
7. **Contract-file citations.** `api-contract.md`/`data-contract.md` carry no ID series of their own; every table row carries a `Req` column naming the owning requirement ID(s), prose entries cite inline. IDs are always enumerated, never given as a range. A row with no owning requirement is marked `—` and raised as a missing requirement, never given an invented ID. Cross-references between spec files use an `-R`/`-E` ID when the target has one, and the target's exact heading text (the string `extract-section.sh` accepts) otherwise — section numbers (`§n.m`) are not used.
8. **Cite every ID a test pins, and only those.** A test's doc comment lists every requirement or edge-case ID its assertions directly verify, at least one, comma-separated after `///` and before the ` — ` summary (`/// MB-R-012, MB-R-157 — …`). An ID the test merely exercises on the way without asserting its outcome is not cited. A test citing many IDs is a review prompt that it does too much, not a violation. `grep -rn <ID>` over the sources therefore returns every test that pins that rule.
9. **One entry, one rule.** A requirement states exactly one observable rule: one subject, one condition, one outcome. A statement that needs "and also", "additionally", a second sentence introducing a different subject, or an enumeration of independent behaviors is several requirements and gets one ID each, cross-citing where one depends on another. Test: can a single test pin the whole entry, and can the entry be falsified by one counterexample? If not, split before landing. A large entry is a review prompt, never a stylistic choice.

## Per-area files

Add/drop per need.

| File | Contains |
|---|---|
| `requirements.md` | Numbered, testable normative statements, one rule each (rule 9). Every area has one. |
| `api-contract.md` | Stable public surface: OCPP actions, Lua `C_*` methods, `:` commands, keybindings, CLI flags, config fields, error variants, feature flags. No ID series of its own; each table row's `Req` column names the owning requirement ID(s). |
| `data-contract.md` | Formats: register model and data formats, payload shapes, config schema, field widths, ordering, ranges. No ID series of its own; each table row's `Req` column names the owning requirement ID(s). |
| `edge-cases.md` | Boundary behavior, error semantics, stated known limitations. Entries carry their own `-E-nnn` IDs. |

## Requirements intentionally not unit-tested

Most requirements are pinned by a test citing the ID. This list records the deliberate exceptions. Four kinds qualify:

1. **Design-posture/platform/toolchain/versioning statements** — facts about build/design, not runtime behavior a `shall` test observes. Each names its enforcement point (CI job, manifest field, lint config).
2. **Cross-cutting restatements asserted under the owning area** — the test lives with the per-area requirement, cited by *that* ID.
3. **Structural/shape requirements collectively exercised by one round-trip or property test** — a group describing a data structure's shape (fields, optionality, default omission), covered by one save→load→compare; behavioral requirements around them tested on their own.
4. **Stated known limitations (`-E` entries)** — class-wide exemption, rules under Kind 4 below.

Anything not listed below, and no `-E` entry outside kind 4, must carry a citing test. A listed requirement that gains observable behavior: remove from the list, add the test.

**Kind 1 — design posture/platform/toolchain/versioning**

| Requirement | Enforced by |
|---|---|
| `NF-R-001`, `NF-R-002`, `NF-R-003` — platforms/toolchain CI builds | CI job matrix |
| `NF-R-010` — no benchmarks asserted; hot path stays on `parking_lot` | design posture, dependency manifest |
| `NF-R-040` — crates versioned in lockstep | workspace manifest |
| `NF-R-041`, `NF-R-055`, `NF-R-056` — the testing conventions, CI steps, lefthook | `AGENTS.md` conventions, lefthook reminder |
| `NF-R-045` — dev-only fixture crate, `publish = false`, versioned in lockstep | workspace manifest, dev-dependency edges |
| `UI-R-001` — alt-screen + raw-mode entry, terminal restore on normal/error/panic exit | raw-mode calls need a controlling tty, absent under `cargo test`; `App` renders through the `DrawSurface` seam, exercised by `ut_app_draws_onto_mock_screen` |
| `UI-R-289` — four per-scheme diff background colors on the scheme | `struct ColorScheme` and the three `#[cfg(feature = …)] const COLOR_SCHEME` initializers in `ferrowl-ui/src/lib.rs`; a missing field or scheme fails to compile |
| `UI-R-292` — the four diff colors are literals, computed from nothing | same initializers: literal `Color::Rgb` values, no call and no `const fn` among them |

**Kind 2 — cross-cutting restatements**

| Requirement | Asserted under |
|---|---|
| `NF-R-011` — Lua sim on its own OS thread | `scripting/` sim tests |
| `NF-R-020` — Modbus reconnect backoff | `MB-R-050`/`MB-R-051`/`MB-R-052` |
| `NF-R-021`, `NF-R-047` — OCPP reconnect and CSMS bind retry | `OC-R-048`, `OC-R-139` |
| `NF-R-022` — a Lua error never crashes its host | `SC-R-032` |
| `NF-R-030`, `NF-R-048`, `NF-R-049` — OCPP TLS / Basic Auth, Modbus TCP TLS, RTU none | OCPP security tests (`OC-R-029`–`041`), `MB-R-104`–`MB-R-111` |
| `NF-R-031`, `NF-R-050`, `NF-R-051` — Lua sandbox, wall-clock cap, no memory ceiling | `SC-R-006`/`SC-R-007`/`SC-R-040`, `SC-R-047`, `SC-R-048` |

**Kind 3 — structural/shape (collectively exercised)**

| Requirements | Exercised by |
|---|---|
| `CS-R-001`, `CS-R-010`, `CS-R-015`, `CS-R-016`, `CS-R-018`, `CS-R-020`, `CS-R-021`, `CS-R-022`, `CS-R-034` — shape of a valid session/device file (fields, informational fields, default omission) | config round-trip (serde) tests: save→load→compare. Envelope behavior (save/load, `migrate`) tested on its own |

**Kind 4 — stated known limitations (`-E` entries)**

No ID table — this kind is a class-wide exemption, not an enumerated list. An `-E` entry that records a known limitation or an observed constraint rather than an asserted behavior is exempt as a class. An `-E` entry that does assert behavior (most boundary-table rows) is outside the exemption and wants a citing test. This obligation binds `-E` entries added after the backfill that introduced the `-E` series; every entry minted by that backfill is grandfathered, and an asserting entry acquires its citing test when its area is next touched for behavior.
