# Modbus — Edge Cases and Known Limitations

Boundary behavior, error semantics, intentional constraints. The known-limitations section below (`## Known limitations — intentional constraints`) is working as implemented; recorded so it is not "fixed".

---

## Codec boundaries

| ID | Condition | Behavior |
|---|---|---|
| **MB-E-001** | Fewer words than the format's width | decode fails with a too-few-bytes error |
| **MB-E-002** | More words than the format's width | surplus ignored; only the first `width` words consumed |
| **MB-E-003** | `Ascii` bytes not valid UTF-8 | decode fails with a packed-ASCII error |
| **MB-E-004** | `Ascii` input longer than `2 × length` bytes | truncated: `Left` keeps first bytes, `Right` keeps last; no error |
| **MB-E-005** | `Ascii` decoded from a padded block | zero-padding **not** stripped; decoded string contains the padding bytes |
| **MB-E-006** | Bit-field mask exceeding the format's width (e.g. `0x1FF` on `U8`) | decode and encode fail with a bit-field-width error |
| **MB-E-007** | Bit-field mask `0` | degenerate, not an error: shift 0, encode produces all-zero words, decode yields 0 |
| **MB-E-008** | Non-numeric text on a numeric format | encode fails with a parse error |
| **MB-E-009** | Typed value whose variant does not match the format | encode fails with a value/format mismatch error |
| **MB-E-010** | `Ascii` with `length = 0` | zero-width: encodes to no words, decodes to empty string |
| **MB-E-011** | Device-config `bitmask` string that does not parse | silently falls back to the full (no-op) mask; no error, no warning |
| **MB-E-012** | Malformed or reversed entry in a `read_ranges` string | silently skipped; no error, no warning |
| **MB-E-013** | `word_order = Reversed` on a width-1 format (`U8`/`I8`/`U16`/`I16`) | inert no-op |
| **MB-E-014** | `word_order` on `Ascii` | ignored; ASCII carries no register order |

---

## Store boundaries

| ID | Condition | Behavior |
|---|---|---|
| **MB-E-021** | Read/write against a range not fully covered by declared regions | fails as a whole (`AddressNotReadable` / `AddressNotWritable`); no partial result, no partial write |
| **MB-E-022** | Read/write against a key with no declared regions | fails with `UnknownKey` |
| **MB-E-023** | Checked write whose value count ≠ range length | fails with a length mismatch, writes nothing |
| **MB-E-024** | Checked read of a write-only cell / write of a read-only cell | fails; unchecked paths bypass this |
| **MB-E-025** | Coil request against register cells (or reverse) | fails; cell type must match |
| **MB-E-026** | Declaring a range intersecting existing regions | all intersecting regions and the new range merge into one spanning their union; values preserved, new addresses zero-initialized |
| **MB-E-027** | Declaring a range merely adjacent to an existing region (zero-length overlap) | not an overlap; may stay separate regions. Coverage still contiguous, so a spanning read/write succeeds |
| **MB-E-028** | Declaring a range whose overlap is incompatible (mismatched cell type, or an access combination that is not a `Read`+`Write` widening) | the **whole** call rejected, including compatible ranges in the same call; key's memory unchanged |
| **MB-E-029** | Range whose `start + size` overflows `usize` | panics. Unreachable from Modbus addressing (`u16` addresses, widths ≤ 8); hand-edited config only |
| **MB-E-030** | Deserialized range with `end < start` | rejected at load |

---

## Client boundaries

| ID | Condition | Behavior |
|---|---|---|
| **MB-E-031** | Modbus exception on a read | logged, no disconnect. Retried on following ticks; after **3 consecutive** the operation is logged invalid and skipped |
| **MB-E-032** | Timeout on a read or write | disconnect, end the run. Subject to reconnect |
| **MB-E-033** | Transport error on a read or write | disconnect, end the run. Subject to reconnect |
| **MB-E-034** | Modbus exception on a write command | logged, no disconnect, no retry — write lost |
| **MB-E-035** | Store write-back of a poll result fails (range not covered) | logged; result discarded. Client stays connected, advances |
| **MB-E-036** | Operation whose start address exceeds 65535 | answered locally with `IllegalDataValue`, never sent (MB-R-149). Unreachable via the planner (`addr: u16`); hand-edited config only |
| **MB-E-037** | Operation whose range length exceeds 65535 | answered locally with `IllegalDataValue`, never sent, then exception-retry path (MB-R-149). Unreachable via the planner, which caps at 125 registers / 2000 bits |
| **MB-E-038** | Empty operation list | tick fires, does nothing; client stays connected |
| **MB-E-039** | Command sent while disconnected and backing off | dropped with a log line, **not** queued |
| **MB-E-040** | Command channel full (10 pending) | sender waits; never silently dropped at the channel |
| **MB-E-041** | Command to a server-role module, or a module not running | rejected with an error |
| **MB-E-042** | `interval_ms = 0` | 1 ms tick, not a busy loop |
| **MB-E-043** | Ticks missed while a slow request is in flight | schedule delayed; no burst of catch-up requests |
| **MB-E-044** | `delay_ms` | applied on **every** (re)connection, not only the first |
| **MB-E-045** | Read operation to slave id 0 on RTU | fails locally, never sent; exception-retry path (MB-R-101). On TCP, unit 0 is ordinary |
| **MB-E-046** | Write command to slave id 0 on RTU | fire-and-forget: written, not awaited, logged as executed even if no device applied it (MB-R-102) |

---

## Server boundaries

| ID | Condition | Behavior |
|---|---|---|
| **MB-E-047** | Read of a range not declared in the store | `IllegalDataAddress` |
| **MB-E-048** | Write to a range not writable | `IllegalDataAddress` |
| **MB-E-049** | Coil request against register cells, or reverse | `IllegalDataAddress` |
| **MB-E-050** | Function code outside MB-R-058's nine | `IllegalFunction`, request logged |
| **MB-E-051** | Read/write-multiple-registers whose read range is unreadable or write range unwritable | `IllegalDataAddress`, **no** write applied |
| **MB-E-052** | Read/write-multiple-registers under concurrent load | read-check, write-check, read, write under a single exclusive hold; no interleaving |
| **MB-E-053** | Request for a slave id with no declared regions, on `Tcp`/`RtuOverTcp`/`AsciiOverTcp`/`Udp` | store lookup fails → `IllegalDataAddress`. No up-front slave id filter |
| **MB-E-054** | Request for a slave id with no declared regions, on `Rtu`/`Ascii` (physical serial) | store lookup fails, answered with silence: another device on the bus may own that id; server must not contend (MB-R-128) |
| **MB-E-055** | Request for a slave id with ≥1 declared region but address outside all, on `Rtu`/`Ascii` | `IllegalDataAddress`; this id is this server's own |
| **MB-E-056** | RTU request to slave id 0 | applied to the store, answered with silence; a store failure that would be `IllegalDataAddress` is invisible to the sender (MB-R-103) |
| **MB-E-057** | Malformed frame / framing error | rejected by the protocol layer before the handler; TCP server logs a processing failure and drops the connection, accept loop continues |
| **MB-E-058** | TCP client disconnects mid-request | that connection's serve task ends; accept loop and store unaffected |
| **MB-E-059** | RTU serial port disappears mid-serve | serve loop ends, server task ends with an error; retry per MB-E-076 |

---

## TLS boundaries

| ID | Condition | Behavior |
|---|---|---|
| **MB-E-060** | Dialog Self-Signed toggled On after `cert_file`/`key_file` text entered | resolved config excludes both files; stored text preserved for Off (MB-R-135/MB-R-186) |
| **MB-E-061** | Dialog client-role Skip-Verify toggled On after Root Store/CA list state entered | resolved verification excludes both, becoming `CertVerification::Skip` (MB-R-135) |
| **MB-E-062** | `tls` set in an RTU device config | ignored; RTU `Config` has no `tls` field (MB-R-112) |
| **MB-E-063** | `cert_file`/`key_file`/`ca_files`/`extra_ca_files`/client-identity path malformed PEM or unreadable | server or client start fails with a TLS configuration error (MB-R-107/MB-R-108 tier) |
| **MB-E-064** | `ServerTlsPolicy::Mutual` client certificate signed by any one of several `ca_files` | accepted: `ca_files` is a trust-anchor set, not an ordered chain (MB-R-108) |
| **MB-E-065** | `CertVerification::CaFiles` with empty `ca_files` | rejected at construction, never at handshake (MB-R-108/MB-R-109) |
| **MB-E-066** | `ServerTlsPolicy::Mutual` with `CertVerification::Skip`, connection presents no client certificate | handshake still fails: `Skip` skips the CA/identity check on a *presented* cert, does not make presenting optional (MB-R-173) |
| **MB-E-067** | Dialog server-role Skip Verify toggled On while mTLS selected | shared CA list hidden, resolved verification `CertVerification::Skip` regardless of entries; list preserved for Off (MB-R-189) |
| **MB-E-068** | Dialog client-role Self Signed toggled On while mTLS selected | Client Cert/Key inputs hidden, resolved identity `CertSource::SelfSigned` regardless of text; text preserved for Off (MB-R-139) |
| **MB-E-069** | Self-signed pair, once generated for a module instance | cached and reused across every bind/connect/reconnect and `:restart`/`:reload`, including a config edit leaving the source self-signed; regenerated only on a transition *into* self-signed; a fresh instance discards the cache; pair never on disk (MB-R-169/MB-R-170/MB-R-138) |
| **MB-E-070** | Client-role `CertVerification::CaFiles` with empty `ca_files` | refused at construction and dialog submit (MB-R-109/MB-R-202): the trust store would reject every server certificate. `RootStore` needs no such rule; `extra_ca_files` may be empty |
| **MB-E-071** | `CertVerification::RootStore` on a server's client-certificate verification | rejected at construction; client-only, no dialog offers it (MB-R-172). Many publicly trusted end-entity certificates carry clientAuth EKU, so honoring the root store would accept any client holding a public certificate: an mTLS bypass. Private-CA anchors are the only sanctioned client-certificate source |
| **MB-E-072** | `CertSource::Ephemeral` as a client's mTLS identity | rejected at construction: `Ephemeral` means "nothing configured, fall back and log", a listener behavior. A client with no identity is `ClientTlsPolicy::Tls`, not `Mutual` |

---

## Monitor boundaries

| ID | Condition | Behavior |
|---|---|---|
| **MB-E-015** | Frame fails CRC (RTU) or LRC (Ascii), or otherwise malformed | logged at Warning level and discarded. Expected on a live multi-drop bus (noise, a device's retry, monitor attaching mid-frame) |
| **MB-E-016** | Completed pairing's operation (MB-R-146) is `ReadWriteMultipleRegisters` (two addresses, two quantities) | Address renders `read_address/write_address`; Quantity `read_quantity/write_quantity`; Values/Payload shows the read response's registers only. The write's values are visible in Memory layout once applied |
| **MB-E-017** | No traffic at all for a table kind on the selected unit id | that kind's hex-editor block (UI-R-063) omitted from Memory layout, not shown empty (MB-R-144) |
| **MB-E-018** | Retransmitted `WriteSingleRegister`/`WriteSingleCoil` request arrives while awaiting that request's response | decodes as the response to itself (byte-identical on the wire): false `Ok` record and phantom observed-table write. `WriteMultiple*` and reads unaffected |
| **MB-E-019** | Two configured instances share the same Rtu/Ascii path | MB-R-150 catches it before the OS-level open, reports a distinct conflict status, recovers once one instance stops or moves |
| **MB-E-020** | Serial path held by something outside the session (external process, bridge-mode leg) | MB-R-150 sees only same-session instances; external holder surfaces as an ordinary OS-level open failure/retry |

---

## Known limitations — intentional constraints

### No max-registers-per-request bound at the protocol layer

**MB-E-073** — Neither client core nor server core enforces the Modbus per-request limits (125 registers / 2000 bits). The **only** enforcement is the application-level read-operation planner, which splits generated poll batches at those limits. A poll operation constructed directly (bypassing the planner) may exceed 125 registers and is sent as-is; the only guard is the `u16` count field, which fabricates `IllegalDataValue` above 65535. The **server** answers any count the peer sends, limited only by the `u16` count field and declared addresses, and does not reject an over-long request with `IllegalDataValue`. A write command is never split: a register wider than the limit would go as one write — unreachable, since the widest format is 8 registers.

### The RTU `Config` cannot be flattened into a `clap` command

**MB-E-074** — The RTU connection config doubles as a `clap` argument group whose short flags collide: `-s` claimed by `slave` and `stop_bits`, `-d` by `data_bits` and `delay_ms`. Flattening it into a `clap::Parser` command panics at parse time via clap's debug assertions. The config is reached only through serde (session and device config files, `--module` key/value form), which is unaffected.

### The RTU `slave` config field is inert

**MB-E-075** — The RTU config carries `slave` (default 1), read by no code path: the client carries a slave id on every request and never attaches the link to one slave. Kept so existing config files keep parsing. An RTU **server** ignores it entirely: it answers whichever slave ids have declared regions.

### Server-side reconnect retries only after the current serve loop ends

**MB-E-076** — Every server transport (TCP, RTU, `RtuOverTcp`, `Udp`, `Ascii`, `AsciiOverTcp`) honors `reconnect` — bind failure, serial-open failure, or mid-serve failure retries with the shared backoff driver (MB-R-051, MB-R-071, MB-R-075, MB-R-120, MB-R-124, MB-R-130–134); a mid-serve failure does not retry immediately — the server waits for the *current* serve loop to end on its own before the backoff wait, since an in-flight connection is never torn down early to reach a retry sooner.

### Unbounded TCP server connections

**MB-E-077** — A TCP server spawns one task per accepted connection, no cap, no idle timeout.

### Only six transports

**MB-E-078** — `RtuOverTcp` reuses the TCP config verbatim; only wire framing differs. `Udp` reuses the TCP config minus `tls` (no handshake, no DTLS); `Udp` does not inherit the RTU-family broadcast slave id 0 handling (MB-R-101–MB-R-103), so on `Udp`, slave id 0 is ordinary. `Ascii` reuses the RTU config verbatim; only framing differs — LRC checksum and `:`/CR LF delimiters instead of CRC and silence-delimited binary. `AsciiOverTcp` reuses the TCP config verbatim, same framing swap as `RtuOverTcp`. Both `Ascii` and `AsciiOverTcp` inherit the RTU-family broadcast handling (MB-R-101–MB-R-103), unlike `Udp`.

### Display resolution is one-way

**MB-E-079** — `resolution` scales for *display only*. Encoding does not divide by it, so input is raw. Entering `10` with `resolution = 0.5` stores raw word `10` and displays `5`; the typed string is not the string read back.

### Declaration failures are warned, not silent

**MB-E-080** — A rejected `Memory::add_ranges` declaration leaves the register or gap cell without backing memory, so runtime reads/writes against it still fail. Every module-construction, module-reconfiguration, and runtime register-edit call site logs a Warning (MB-R-129/MB-R-184) naming the register (or, for a gap cell, slave id and register kind) and the rejected range at rejection time, so the eventual runtime failure is traceable. Reachable case: a register added at runtime at an address a `read_ranges` gap already declared read-only — overlap (existing `Read` cell, requested `ReadWrite` region) is not a widening combination, so the declaration is rejected and dropped, with a Warning naming register and range.

### Client writes are fire-and-forget

**MB-E-081** — A client-side write is dispatched to the client task's command channel; the caller is told "sent" once queued. The Modbus response (including an exception) is logged by the client task but not reported back, and the store is not updated from it. The polled value eventually reflects the truth, except write-only registers, whose written value is mirrored into the store locally.

### The register's `access` does not gate store access

**MB-E-082** — A register's `access` does **not** determine its cells' direction; its *kind* does (coils and holding registers read/write, discrete inputs and input registers read-only). `access` governs whether the register is polled (write-only excluded), whether a client-side write is mirrored into the store, and (MB-R-159/MB-R-151) whether the client attempts a write at all: a `ReadOnly` register on a **client** rejects `:set`/dialog writes locally; on a **server** MB-R-090 bypasses cell access checks — so a `ReadOnly` holding register is therefore still writable by a remote master against a server module, even though the local UI cannot write it from a client module.

### Bit-field mask absent from the width error

**MB-E-083** — `BitFieldWidth`'s message names the format by display text (MB-R-155), which carries byte order but not the mask, so the offending mask does not appear; the user supplied it, and the alternative is dumping the whole format struct.
