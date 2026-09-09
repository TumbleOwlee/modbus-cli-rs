# Bridge Mode — Edge Cases and Known Limitations

Boundary behavior, error semantics, intentional constraints. Working as implemented; recorded so it is not "fixed".

---

## Link loss and reconnect

| ID | Condition | Behavior |
|---|---|---|
| **BR-E-001** | Upstream RTU serial link lost | ends the bridge task with an error; no upstream reconnect, unlike an ordinary Modbus server's backoff retry (MB-E-076): bridge holds no store and no session to keep serving from, so a lost upstream link ends the whole relay |
| **BR-E-002** | Downstream connection/link lost or unavailable | 1s–30s backoff reconnect while upstream keeps accepting/serving; a request arriving during downstream backoff gets the BR-R-010 exception rather than blocking |
| **BR-E-003** | Downstream connect fails at startup | not a setup failure: process starts normally, every forwarded request answered `GatewayPathUnavailable` until downstream connects (BR-R-010) |
| **BR-E-004** | Downstream `reconnect` unset (`false`) and a connect/exchange failure occurs | never retries; every subsequent forwarded request answers `GatewayPathUnavailable` indefinitely (BR-R-006, BR-R-010) |

## Pass-through and filtering

| ID | Condition | Behavior |
|---|---|---|
| **BR-E-005** | A request's register/bit count | no per-request count/PDU-size enforcement beyond each transport's wire format (MB-E-073; BR-R-007) |
| **BR-E-006** | RTU descriptor's `slave` key, if given | inert, as for an ordinary RTU server (MB-E-075): bridge answers whichever slave ids arrive |
| **BR-E-007** | Downstream descriptor's `unit_ids` key, if given | parsed but never consulted; `unit_ids` filtering (BR-R-015) is upstream-only |

## Multidrop bus safety

- **BR-E-010** — **Unfiltered bridge on a multidrop bus** — on a shared/multidrop RTU upstream bus, a bridge with no `unit_ids` filter (BR-R-015) forwards a request meant for another device downstream, gets it rejected or timed out, and answers upstream with a failure that collides on the wire with the real device's own correct answer; omitting `unit_ids` is therefore safe only on a dedicated point-to-point upstream link.

## Exit codes

- **BR-E-011** — **`--exit-on-error` exit code** — `--exit-on-error` exits 3, distinct from the clap usage-error code 2, mirroring `run` (CL-E-003).

- **BR-E-008** — **`;` inside a descriptor value** — a multi-file CA list is the one descriptor value carrying its own delimiter, because `,` separates keys and the merged `CertVerification` takes a list where the retired `ca_file`/`client_ca_file` took one path. A path containing a literal `;` is unreachable through a descriptor; bridge is CLI-only (BR-R-002) with no config file to fall back on, and a repeated key would break BR-R-004's one-key-one-value grammar for every other key.
- **BR-E-009** — **`tls.*` on an `rtu` descriptor** — rejected outright (exit 1) rather than ignored, even `tls.mode=none`, which would be a no-op if honoured. A serial link has no TLS layer, so such a key can only express a mistake about which descriptor is being written; failing at setup says so when it is cheap to fix, where silent acceptance would leave the operator believing a plaintext bus was protected (BR-R-023).
