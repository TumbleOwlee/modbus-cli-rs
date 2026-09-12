# Modbus — Requirements

Register model and codec, register store, client/server roles, TCP/RTU transports, reconnect, Modbus device configuration.

IDs stable, append-only (`MB-R-nnn`). See [`../README.md`](../README.md). Companions: [`api-contract.md`](./api-contract.md) (function codes, config fields), [`data-contract.md`](./data-contract.md) (register tables, formats, addressing), [`edge-cases.md`](./edge-cases.md).

---

## Register model

**MB-R-001** — A register has exactly five properties: slave id, access direction, register table (kind), address, data format.

**MB-R-002** — Slave id is 8-bit (0–255), default 1; 0 is the RTU broadcast address (MB-R-101, MB-R-103).

**MB-R-003** — A register's address is a fixed 16-bit Modbus address (0–65535) or *virtual* (no wire address); default virtual.

**MB-R-004** — A register's kind is one of exactly four tables: coil, discrete input, holding register, input register.

**MB-R-005** — A register's access is `ReadOnly`, `WriteOnly`, or `ReadWrite`, default `ReadWrite`.

**MB-R-097** — A register definition's kind defaults to holding register.

**MB-R-006** — A register's format determines its width in 16-bit registers = consecutive addresses occupied from its address.

**MB-R-007** — Decoding converts raw 16-bit words to a typed value per format; encoding converts a string or typed value to raw words per the same format.

**MB-R-008** — Encoding a typed value whose type mismatches the format fails with a value/format mismatch error, never coerces.

**MB-R-009** — A register exposes a per-word write mask selecting exactly its own bits and a merge `(old & !mask) | (new & mask)` per word, so a bit-field write preserves sibling registers aliasing the same address. Words absent from `old` are zero.

---

## Data formats and codec

**MB-R-010** — Exactly thirteen formats: `Ascii`, `U8`, `U16`, `U32`, `U64`, `U128`, `I8`, `I16`, `I32`, `I64`, `I128`, `F32`, `F64`.

**MB-R-011** — Widths in 16-bit registers: `U8`/`I8`/`U16`/`I16` = 1; `U32`/`I32`/`F32` = 2; `U64`/`I64`/`F64` = 4; `U128`/`I128` = 8; `Ascii` = configured width. Byte length = 2 × register width.

**MB-R-012** — `U8` and `I8` occupy a whole 16-bit register; the byte sits in the low byte for big-endian, high byte for little-endian.

**MB-R-013** — Every integer and float format carries a byte order `Big` or `Little`. `Big` reads the words' byte stream in wire order; `Little` fully reversed.

**MB-R-099** — Every integer and float format also carries a register order `Normal` or `Reversed`, independent of byte order (MB-R-013), acting on the sequence of 16-bit words: `Normal` natural, `Reversed` whole sequence reversed. Default `Normal`.

**MB-R-160** — The register order (MB-R-099) is applied **before** the byte-order rule (MB-R-013) on decode and **after** it on encode.

**MB-R-100** — For a width-1 format (`U8`, `I8`, `U16`, `I16`) register order is a no-op.

**MB-R-014** — Every integer format carries a bit-field mask; the shift is *derived* as the mask's trailing-zero count, never configured.

**MB-R-015** — Integer decode yields `(raw & mask) >> shift`; encode places `(value << shift) & mask`, bits outside the mask zero.

**MB-R-016** — A mask setting any bit at or above the format's integer width is rejected with a bit-field-width error on decode and encode. The full-width default mask is always accepted.

**MB-R-017** — Float formats carry no bit-field; theirs behaves as the no-op full mask.

**MB-R-018** — Float formats encode/decode as their raw IEEE 754 bit pattern under the same byte-order rule as integers.

**MB-R-019** — `Ascii` packs two characters per register, carries no byte order and no bit-field, and carries an alignment `Left` or `Right`.

**MB-R-020** — Encoding `Ascii` zero-pads input to exactly `2 × width` bytes, on the right for `Left`, on the left for `Right`. Longer input is truncated, keeping the *first* bytes for `Left` and the *last* for `Right`.

**MB-R-021** — Every numeric format carries a display resolution (scale factor, default `1.0`), and its displayed value is `raw × resolution`.

**MB-R-212** — Encode and decode never apply a format's display resolution (MB-R-021); wire words are raw.

**MB-R-022** — Numeric string input accepts a plain decimal literal or a `0x`-prefixed hex literal.

**MB-R-206** — A signed integer format additionally accepts `-0x…` (MB-R-022), the negation of the hex bit pattern.

**MB-R-207** — A `0x` literal on a float format (MB-R-022) is the IEEE 754 bit pattern, not a decimal value.

**MB-R-023** — Decoding fewer words than the format's width fails with a too-few-bytes error. More words: only the first `width` consumed.

**MB-R-024** — Decoding `Ascii` bytes that are not valid UTF-8 fails with a packed-ASCII error.

**MB-R-025** — A decoded value is also renderable as its raw, unscaled, zero-padded hex bit pattern (two's complement for signed integers, IEEE 754 bits for floats, two hex digits per byte for ASCII).

---

## Register store

**MB-R-026** — The store partitions memory by device key; default key = (slave id, register table), so each slave's four tables are independent address spaces.

**MB-R-027** — A request's table derives from its function code: coil-family → coil table, discrete-input reads → discrete-input table, holding-register-family → holding table, input-register reads → input table. Any other code → holding table.

**MB-R-028** — Address ranges are half-open `[start, end)`. `end < start` is rejected on deserialization.

**MB-R-029** — A memory region must be declared before read or write; reads and writes succeed only on addresses fully covered by declared regions.

**MB-R-030** — Each declared cell carries a cell type (single-bit for coils/discrete inputs, 16-bit for registers) and an access direction (read, write, read/write).

**MB-R-031** — Declaring a range overlapping an existing region of the same cell type merges into it (MB-R-095); a read region overlapping a write cell (or vice versa) widens that cell to read/write.

**MB-R-032** — Declaring a range overlapping an existing region with an incompatible cell type or access combination fails and leaves the key's memory unchanged, including when earlier ranges of the same call were compatible.

**MB-R-033** — A checked read fails when the key is unregistered, the range is not fully covered, or any cell is not readable as the requested cell type. A checked write fails under the equivalent conditions and when value count ≠ range length.

**MB-R-034** — The store also offers unchecked read/write paths ignoring per-cell access direction (unchecked read returns write-only cells; unchecked write overwrites read-only cells). Both still require full coverage by declared regions.

**MB-R-095** — Declaring a range intersecting existing regions of a key merges **all** intersecting regions and the new range into one region spanning their union. Stored values are preserved; newly covered addresses are zero-initialized with the declared access direction.

**MB-R-096** — A key's declared regions never overlap: after any declaration every address is covered by at most one region.

---

## Client

**MB-R-035** — A client polls a list of operations, each a (slave id, read function code, address range) triple, writing every successful read into the shared store under the key per MB-R-027.

**MB-R-036** — The operation list is shared and mutable at runtime; a change takes effect on subsequent poll cycles without reconnect or respawn.

**MB-R-037** — Polling is round-robin, advancing after each successful read.

**MB-R-038** — Before the first poll on each connection, the client waits `delay_ms`.

**MB-R-039** — Polling runs on a fixed tick of `interval_ms`.

**MB-R-204** — `interval_ms` 0 polls on a 1 ms tick (MB-R-039).

**MB-R-205** — A missed poll tick (MB-R-039) delays the schedule, never fires catch-up ticks.

**MB-R-040** — Every individual request (read or write) is bounded by `timeout_ms`.

**MB-R-041** — The poll loop issues only read codes: read coils, read discrete inputs, read holding registers, read input registers.

**MB-R-042** — Coil and discrete-input reads are stored one word per bit, `1` set, `0` clear.

**MB-R-043** — A read returning a Modbus exception does not disconnect.

**MB-R-157** — After a read returns a Modbus exception (MB-R-043), the client retries the same operation on subsequent ticks; after 3 consecutive exceptions for that operation it logs it invalid, skips it, advances, and resets the counter.

**MB-R-149** — Before issuing a read, the client validates the range against the wire's `u16` fields: a start address or computed count (`end - start`) not fitting in `u16` is answered locally with `IllegalDataValue`, never sent, and follows MB-R-043's exception-retry path.

**MB-R-044** — The retry counter resets to zero on any successful read.

**MB-R-045** — A read timeout or transport error disconnects the client and ends the connection run.

**MB-R-046** — The client accepts write commands over a command channel concurrently with polling: write single coil, write multiple coils, write single register, write multiple registers, terminate.

**MB-R-047** — A write command returning a Modbus exception is logged, no disconnect. A write timeout or transport error disconnects and ends the connection run.

**MB-R-048** — Each read and write addresses the slave id carried by the operation or command, independent of any slave id configured on the transport.

**MB-R-049** — The terminate command, or the command channel closing, disconnects, ends the task with success, and emits a client-disconnected status.

**MB-R-101** — On RTU, RtuOverTcp, Ascii, or AsciiOverTcp, a poll operation addressed to slave id 0 (broadcast) fails locally without reaching the wire, treated as a Modbus exception per MB-R-043, no disconnect.

**MB-R-102** — On RTU, RtuOverTcp, Ascii, or AsciiOverTcp, a write command to slave id 0 is transmitted without awaiting a response, logged as executed, no disconnect.

---

## Reconnect

**MB-R-050** — With `reconnect` enabled (default), neither a failed connection attempt nor a transport error during a run ends the client task; the client waits a backoff and retries.

**MB-R-051** — Backoff starts at 1 s, doubles after each failed attempt, capped at 30 s.

**MB-R-052** — Backoff resets to 1 s after any connection run with at least one successful read.

**MB-R-053** — Terminate, or the command channel closing, aborts a backoff wait immediately and ends the task with success.

**MB-R-054** — Any command other than terminate arriving while disconnected and backing off is dropped with a log line, not queued.

**MB-R-055** — With `reconnect` disabled, a failed connection attempt or transport error ends the client task with that error, after emitting a client-disconnected status.

**MB-R-056** — Connection settings (`reconnect`, `timeout_ms`, `delay_ms`, `interval_ms`, transport endpoint) are re-read from shared config on every connection attempt, so an edit takes effect on the next reconnect.

**MB-R-137** — A Modbus client module's displayed status is one of three states driven by transport connectivity and task liveness: `CONNECTED` while the transport is connected; `RECONNECTING` while the task runs but the transport is not connected (dial in progress or backoff wait, MB-R-050–MB-R-056); `DISCONNECTED` while the task is not running (never started, stopped, or ended after a non-reconnecting failure, MB-R-055).

**MB-R-190** — A Modbus client module's status bar colors (MB-R-137): `CONNECTED` success, `RECONNECTING` warning, `DISCONNECTED` error.

**MB-R-130** — With `reconnect` enabled (default), a server's listener bind failure (TCP, `RtuOverTcp`, `Udp`, `AsciiOverTcp`) or serial-port open failure (RTU, `Ascii`) does not end the server task; it waits a backoff and retries per MB-R-051.

**MB-R-131** — With `reconnect` enabled, a mid-serve failure (listener or serial port failing after opening) retries the same way once the current serve loop ends.

**MB-R-132** — Backoff resets to 1 s after any serve loop during which at least one connection was accepted (TCP, `RtuOverTcp`, `AsciiOverTcp`) or at least one request/datagram was read (RTU, `Ascii`, `Udp`).

**MB-R-133** — Terminate, or the command channel closing, aborts a backoff wait immediately and ends the server task with success.

**MB-R-134** — With `reconnect` disabled, a bind failure, serial-open failure, or mid-serve failure ends the server task with that error, after emitting a server-stopped status.

**MB-R-153** — A Modbus server module's displayed status follows MB-R-137's three-state rule with "listener bound" (TCP-family) or "serial port open" (RTU/Ascii) for "transport connected": `CONNECTED` while bound/open; `RECONNECTING` while the task runs but is not bound/open (MB-R-071, MB-R-075, MB-R-120, MB-R-130–MB-R-134); `DISCONNECTED` while the task is not running.

**MB-R-220** — A terminate, or the command channel closing, arriving while a client connection attempt is in flight (TCP connect, TLS handshake, UDP local bind, or serial-port open) aborts that attempt immediately and ends the client task with success, without waiting for the attempt to succeed or fail (MB-R-053 covers the same arrival during a backoff wait).

**MB-R-221** — A terminate, or the command channel closing, arriving while a server's listener bind or serial-port open is in flight aborts it immediately and ends the server task with success, without waiting for the bind or the open to complete (MB-R-131 and MB-E-076 keep the separate rule that an already-running serve loop is not torn down early).

---

## Server

**MB-R-057** — A server answers every request directly from the shared store: no request queue, no simulated device logic.

**MB-R-058** — A server answers read coils, read discrete inputs, read holding registers, read input registers, write single coil, write single register, write multiple coils, write multiple registers, read/write multiple registers.

**MB-R-059** — Every other function code (report-server-id, mask-write-register, read-device-identification, diagnostics, get-comm-event-counter, get-comm-event-log, read/write file record, read-FIFO-queue, any custom code) is rejected with `IllegalFunction`.

**MB-R-060** — A read whose range is not fully covered, or whose cells are not readable as the requested cell type, is answered `IllegalDataAddress`; likewise a write whose range is not writable. `IllegalFunction` is reserved for an unsupported code (MB-R-059).

**MB-R-061** — Coil reads report a stored word as set when non-zero; coil writes store set as `1`, clear as `0`.

**MB-R-062** — A multi-register or multi-coil write is answered with the address written and the value count.

**MB-R-063** — Read/write-multiple-registers performs read check, write check, read, and write under a single exclusive hold on the store; the response carries values read *before* the write.

**MB-R-064** — Read/write-multiple-registers whose read range is not readable or write range not writable is answered `IllegalDataAddress` with no write applied.

**MB-R-065** — A server serves any slave id with declared regions; it does not filter by a configured slave id.

**MB-R-103** — An RTU, RtuOverTcp, Ascii, or AsciiOverTcp server applies a request addressed to slave id 0 (broadcast) to the store as any other but emits no response frame, not even an exception.

**MB-R-128** — On `Rtu` or `Ascii` (not `RtuOverTcp`/`AsciiOverTcp`), a request for a slave id with no declared region in any table is applied as any other (store lookup fails per MB-R-065) but emits no response, not even an exception, matching MB-R-103. A slave id with ≥1 declared region but an address outside all of them still gets `IllegalDataAddress`.

**MB-R-066** — A server logs a "request received" line for every request, including rejected function codes.

**MB-R-067** — A server logs the per-request outcome (success/failure) on every transport.

---

## Transport — TCP

**MB-R-068** — A TCP client connects to `ip:port`, the attempt bounded by `timeout_ms`.

**MB-R-069** — An `ip`/`port` pair not parsing as a socket address fails with a TCP address error, client and server.

**MB-R-070** — A TCP server binds `ip:port` and accepts in a loop, serving each connection concurrently against the same store.

**MB-R-071** — With `reconnect` enabled (default), a bind failure does not fail the server's start; it retries per MB-R-051, MB-R-130–MB-R-134. Disabled: bind failure fails the start and the error surfaces to the caller.

---

## Transport — RTU

**MB-R-072** — An RTU client opens the serial port at `path` with `baud_rate`, applying `parity`, `data_bits`, `stop_bits` when set. Unset = Modbus serial-line default: 8 data bits, even parity, one stop bit.

**MB-R-073** — `data_bits` accepts exactly 5, 6, 7, 8; `stop_bits` exactly 1, 2; `parity` exactly `even`, `odd`, `none`, case-insensitive. Any other value fails with a serial configuration error before the port opens.

**MB-R-074** — An RTU server opens the port once and serves it as a single persistent point-to-point connection, no accept loop.

**MB-R-075** — With `reconnect` enabled (default), a serial-open failure does not fail a server's start; it retries per MB-R-051, MB-R-130–MB-R-134.

**MB-R-208** — With `reconnect` disabled, a serial-open failure fails a server's start with a serial error (MB-R-075).

**MB-R-209** — For a client, a serial-open failure is a failed connection attempt under MB-R-050–MB-R-055 (MB-R-075).

---

## Transport — RtuOverTcp

**MB-R-113** — Transport config offers `RtuOverTcp` (tag `rtu_over_tcp`), carrying exactly the TCP option's parameters (`ip`, `port`, `timeout_ms`, `delay_ms`, `interval_ms`, `reconnect`, `tls`) and no RTU serial parameters.

**MB-R-114** — An `RtuOverTcp` connection is established exactly as TCP (MB-R-068–MB-R-071), but requests and responses use RTU framing (unit id + CRC, no MBAP header).

**MB-R-115** — TLS (MB-R-104–MB-R-111, MB-R-161–MB-R-178) applies to `RtuOverTcp` exactly as to Modbus-TCP: same `tls` field, certificate resolution, self-signed fallback, mTLS rules, handshake-failure logging; only post-handshake framing differs.

---

## Transport — UDP

**MB-R-116** — Transport config offers `Udp` (tag `udp`), carrying `ip`, `port`, `timeout_ms`, `delay_ms`, `interval_ms`, `reconnect`: the TCP parameters minus `tls` (the underlying transport has no handshake and no TLS/DTLS). No RTU serial parameters.

**MB-R-117** — A `Udp` client associates with `ip:port` by binding an ephemeral local UDP socket and connecting it to that peer (no network handshake).

**MB-R-179** — A `Udp` client's unparseable `ip`/`port` (MB-R-117) fails with MB-R-069's error.

**MB-R-180** — A `Udp` client's `timeout_ms` bounds each request (MB-R-040), not the local bind/associate (MB-R-117).

**MB-R-118** — Reconnect rules (MB-R-050–MB-R-056) apply to a `Udp` client verbatim, "local bind/associate attempt" substituting for "connection attempt".

**MB-R-181** — MB-R-101–MB-R-103 (broadcast slave id 0) do not extend to a `Udp` client: slave id 0 is an ordinary slave id under MB-R-043, MB-R-045, MB-R-047.

**MB-R-119** — A `Udp` server binds `ip:port` once and serves datagrams from any peer, each independently against the same store (MB-R-057–MB-R-065); no accept loop, no per-peer lifecycle, no `on_connect`/`on_disconnect`.

**MB-R-182** — MB-R-101–MB-R-103 do not extend to a `Udp` server: a request for slave id 0 is answered as any other (MB-R-065), including its response.

**MB-R-120** — With `reconnect` enabled (default), a `Udp` bind failure does not fail the start; it retries per MB-R-051, MB-R-130–MB-R-134 (as MB-R-071). Disabled: start fails, error surfaced.

**MB-R-183** — A `Udp` server datagram failing to receive or decode is logged as a failed request (MB-R-066–MB-R-067) and neither ends serving nor affects any other datagram.

---

## Transport — Ascii

**MB-R-121** — Transport config offers `Ascii` (tag `ascii`), carrying exactly the RTU option's parameters (`path`, `baud_rate`, `parity`, `data_bits`, `stop_bits`); `rtu::Config` reused verbatim.

**MB-R-122** — An `Ascii` client opens the port exactly as RTU (MB-R-072–MB-R-073) but uses Modbus ASCII framing: `:` start, hex-encoded PDU bytes, LRC checksum, CR LF terminator.

**MB-R-123** — An `Ascii` server opens the port once and serves a single persistent point-to-point connection, no accept loop (as MB-R-074), with ASCII framing.

**MB-R-124** — `Ascii` serial-open failure is handled exactly as MB-R-075: `reconnect` enabled → retry with the shared backoff; disabled → start fails with a serial error. Client: failed connection attempt under MB-R-050–MB-R-055.

---

## Transport — AsciiOverTcp

**MB-R-125** — Transport config offers `AsciiOverTcp` (tag `ascii_over_tcp`), carrying exactly the TCP option's parameters (`ip`, `port`, `timeout_ms`, `delay_ms`, `interval_ms`, `reconnect`, `tls`) and no serial parameters.

**MB-R-126** — An `AsciiOverTcp` connection is established exactly as TCP (MB-R-068–MB-R-071) but uses Modbus ASCII framing (`:` start, hex PDU, LRC, CR LF).

**MB-R-127** — TLS (MB-R-104–MB-R-111, MB-R-161–MB-R-178) applies to `AsciiOverTcp` exactly as to Modbus-TCP or `RtuOverTcp`: same `tls` field, certificate resolution, self-signed fallback, mTLS rules, handshake-failure logging; only post-handshake framing differs.

## TLS setup dialog

**MB-R-135** — The Modbus TCP setup dialog (`Tcp`, `RtuOverTcp` per MB-R-115, `AsciiOverTcp` per MB-R-127) resolves its transport config to the variant its toggles select, contributing only that variant's fields: Self-Signed On → identity `CertSource::SelfSigned`, no cert/key paths; Skip-Verify On → verification `CertVerification::Skip`, no Root Store selection and no CA entry.

**MB-R-186** — The Modbus TCP setup dialog leaves hidden widgets' stored state (input text, toggle position, list entries) unmodified when resolving per MB-R-135, so toggling back Off restores what was entered.

**MB-R-136** — The Modbus TCP setup dialog presents CA file paths through one shared list widget (server role: client-CA list, shown whenever mTLS is selected; client role: server-CA list, MB-R-156) allowing zero or more paths added, edited, or removed individually, mirroring a register's predefined named-values list.

**MB-R-187** — An add-entry confirm on the shared CA list widget (MB-R-136) is rejected (sub-dialog stays open with an inline error, nothing appended) unless the path is non-empty, exists on disk, is not a directory, and has extension `pem`/`crt`/`key` (case-insensitive).

**MB-R-188** — Server role of the Modbus TCP setup dialog: mTLS with a non-empty client-CA list (MB-R-136) and Skip Verify Off → `ServerTlsPolicy::Mutual` with `CertVerification::CaFiles` holding exactly those files; an empty list there is a validation error (mirroring MB-R-108), never a silent fallback to `Tls`.

**MB-R-189** — The Modbus TCP setup dialog also offers a server-role Skip Verify toggle, shown whenever mTLS is selected: On hides the client-CA list (MB-R-136) and resolves verification to `CertVerification::Skip` regardless of entries, per MB-R-186's hidden-field pattern.

**MB-R-156** — The Modbus TCP setup dialog's client role offers, whenever Skip Verify is Off, a Root Store toggle (default On) plus the shared CA list widget (MB-R-136) holding zero or more server-CA paths, replacing the single `ca_file` input. Root Store On → `CertVerification::RootStore` with the list as `extra_ca_files`, empty or not; Off → `CertVerification::CaFiles` with the list as `ca_files`.

**MB-R-202** — Root Store Off with an empty CA list in the Modbus TCP setup dialog's client role (MB-R-156) is a validation error refusing to close the dialog (mirroring MB-R-188), since a verification naming no trust anchor rejects every server certificate.

**MB-R-203** — Skip Verify On hides both the Root Store toggle and the CA list of the Modbus TCP setup dialog's client role (MB-R-156) per MB-R-135.

**MB-R-138** — Under `ClientTlsPolicy::Mutual` with `identity: CertSource::SelfSigned`, a client presents an ephemeral self-signed certificate/key pair as its mTLS identity, generated and cached per MB-R-169/MB-R-170 (once per module instance, reused across reconnects/restarts/config edits, regenerated only on a transition into self-signed), never written to disk.

**MB-R-139** — The dialog offers a client-role Self Signed toggle, shown whenever mTLS is selected.

**MB-R-213** — With the client-role Self Signed toggle On (MB-R-139), the Client Cert/Key inputs are hidden and excluded from the resolved config regardless of their text, which resolves to `ClientTlsPolicy::Mutual` with `identity: CertSource::SelfSigned`.

**MB-R-214** — With the client-role Self Signed toggle On (MB-R-139), validation does not require the Client Cert/Key files.

**MB-R-215** — The client-role Self Signed toggle (MB-R-139) leaves the stored Client Cert/Key text unmodified, so switching it Off restores those paths and re-requires them.

---

## Module lifecycle and device configuration

**MB-R-076** — Each Modbus module instance is a client, a server, or a monitor (never more than one), over TCP, RTU, RtuOverTcp, Udp, Ascii, or AsciiOverTcp.

**MB-R-210** — A client or server instance (MB-R-076) owns one shared register store, one register set, and one log.

**MB-R-211** — A monitor instance (MB-R-076, MB-R-140–MB-R-145) owns one log and one observed-value table instead of a store, and user-authored display interpretations instead of an access-checked register set.

**MB-R-077** — A module's store is built from its device config's register definitions: each fixed-address register declares `[address, address + format width)` under key (slave id, kind).

**MB-R-078** — Coil and holding-register definitions declare read/write cells; discrete-input and input-register definitions declare read-only cells. The register's own `access` does not change the declared cell direction.

**MB-R-079** — A definition with a `default` has it encoded and written into the store at module construction, bypassing cell access checks.

**MB-R-080** — A virtual register never occupies store memory; its value lives in a per-module, name-keyed virtual store. Without `default` it is seeded with its format's decoding of all-zero words.

**MB-R-081** — A client's poll operations derive from the definitions: write-only and virtual registers excluded; the rest grouped by (slave id, read function code).

**MB-R-082** — Without explicit `read_ranges` for a function code, each register in that group is read by its own request; no merging across gaps.

**MB-R-083** — With explicit `read_ranges` for a function code, all registers inside one configured range are read by a single request bridging their gaps, trimmed to the first and last register's extent. Registers outside every range get their own requests.

**MB-R-084** — Gap addresses inside a configured `read_range` backed by no register are declared read-only cells, so a batched read spanning them can be stored.

**MB-R-085** — No generated read request exceeds 125 registers, or 2000 bits for coils/discrete inputs. A batch exceeding the limit is split.

**MB-R-086** — A split point falling inside a register moves back to that register's start, so no request reads a register in half.

**MB-R-087** — Effective timing: device config's `timeout_ms` / `delay_ms` / `interval_ms` / `reconnect` when set, else 3000 ms / 1000 ms / 1000 ms / enabled. Timing is a property of the device config, never the session's per-instance spec.

**MB-R-088** — Adding, editing, or deleting a register at runtime rebuilds the shared operation list; the running client picks it up on its next poll cycle without reconnect.

**MB-R-089** — Reconfiguring a module's endpoint or role stops the running instance, rebuilds it against the same store and register set, and preserves stored values.

**MB-R-090** — Writing a value to a fixed-address register on a **server** read-modify-writes its words into the store per MB-R-009, bypassing cell access checks.

**MB-R-091** — Writing a value to a fixed-address register on a **client** read-modify-writes per MB-R-009 and sends a Modbus write command: single-coil/single-register when the encoded value is one word, multiple otherwise.

**MB-R-158** — A client write to a fixed-address register (MB-R-091) does not update the store directly, except for a write-only register (value not otherwise observable).

**MB-R-159** — Writing a value to a `ReadOnly` fixed-address register on a **client** sends no write command and touches no store; the write is silently accepted.

**MB-R-092** — Writing a virtual register is accepted on a server (updating the virtual store) and rejected on a client.

**MB-R-093** — Sending a write command to a module whose instance is a server, or not running, fails with an error, never silently dropped.

**MB-R-094** — Stopping a client first requests graceful termination and aborts the task only if it has not finished within the grace period; a stopped instance is restartable.

**MB-R-098** — When a Modbus module view stops its instance for `:restart` or `:reload`, a stop failure other than "not running" is reported in the module message log at Error level, not discarded.

## TLS policy

**MB-R-104** — The Modbus TCP connection config carries a `tls` field of type `ModbusTlsConfig`, present unconditionally, holding one `ServerTlsPolicy` under `server` and one `ClientTlsPolicy` under `client`, serialized `[tls.server]`/`[tls.client]`.

**MB-R-161** — Both role policies of the `tls` field (MB-R-104) are kept because a device config records no role, so a dialog's role toggle discards neither.

**MB-R-162** — Each policy of the `tls` field (MB-R-104) independently defaults to `None`, so an absent `tls` block, an empty one, and one with both `mode = "none"` are the same state: plain TCP in whichever role.

**MB-R-163** — An endpoint consults only the `tls` policy (MB-R-104) matching its role and treats the other as inert, never validated against this role's rules. Policy other than `None`: client connects over TLS, server listens over TLS.

**MB-R-105** — A Modbus TCP/`RtuOverTcp`/`AsciiOverTcp` endpoint's TLS configuration is one of two role-specific policy enums, each variant carrying exactly its state's fields, sharing one certificate-source type and one peer-verification type: `enum ServerTlsPolicy { None, Tls { identity: CertSource }, Mutual { identity: CertSource, verification: CertVerification } }`, `enum ClientTlsPolicy { None, Tls { verification: CertVerification }, Mutual { verification: CertVerification, identity: CertSource } }`, `enum CertSource { Ephemeral, SelfSigned, Files { cert_file: String, key_file: String } }`, `enum CertVerification { Skip, RootStore { extra_ca_files: Vec<String> }, CaFiles { ca_files: Vec<String> } }`.

**MB-R-164** — Each TLS policy enum and shared type of MB-R-105 is internally tagged on the wire (`mode` for the policies, `source` for `CertSource`, `verify` for `CertVerification`), kebab-case variant names, no shadow struct or hand-written `Serialize`/`Deserialize`.

**MB-R-165** — A server-role endpoint's TLS field is a `ServerTlsPolicy`; a client-role's a `ClientTlsPolicy` (MB-R-105).

**MB-R-166** — `CertSource::Ephemeral` (MB-R-105) = "no TLS material configured", distinct from `SelfSigned` in that only `Ephemeral` logs the fallback (MB-R-106/OC-R-095).

**MB-R-167** — Of MB-R-105's TLS configuration, exactly three conditions remain checkable rather than structural, each one condition on one variant: `CertVerification::CaFiles` carries non-empty `ca_files`; `CertSource::Ephemeral` rejected as a client identity; `CertVerification::RootStore` rejected as a server's client-certificate verification. Every other former rule is unrepresentable and therefore unspecified.

**MB-R-168** — OCPP CS and CSMS adopt the same policy enums and shared types as MB-R-105 (OC-R-039/OC-R-096) in the two-role container of OC-R-126.

**MB-R-106** — With `tls` set, a server's presented certificate follows its `identity`: `CertSource::Files` presents the PEM files at `cert_file`/`key_file`; `CertSource::SelfSigned` presents an ephemeral self-signed certificate; `CertSource::Ephemeral` presents the same with the fallback logged.

**MB-R-169** — A server's self-signed pair (MB-R-106) is generated once and cached for the module instance's life; every subsequent bind, reconnect-driven rebind, `:restart`/`:reload` reuses it, including across a config edit leaving the source self-signed. A torn-down and freshly constructed instance (not `:restart`/`:reload`) discards the cache; the pair is never persisted.

**MB-R-170** — A server's cached self-signed pair (MB-R-169) is regenerated only when the variant changes *to* `SelfSigned`/`Ephemeral` from `Files`, or `tls` transitions from unset to set resolving to either.

**MB-R-107** — A half-configured certificate pair is not representable in a config file: `CertSource::Files` carries `cert_file` and `key_file` as required fields, so naming one without the other fails to deserialize.

**MB-R-171** — The complete-certificate-pair rule (MB-R-107) survives only in the setup dialog, whose inputs are independently editable: the dialog shows an error border on a blank cert/key input while shown and refuses to close on submit while either of a role's pair is blank, for server identity and client mTLS identity alike.

**MB-R-108** — With a server's `ServerTlsPolicy::Mutual`, client-certificate verification is governed by `verification`. `CertVerification::CaFiles`: a connection presenting no client certificate, or one not signed by any configured `ca_files`, fails the handshake and never reaches the handler; any one configured CA suffices; `ca_files` non-empty, construction failing otherwise.

**MB-R-172** — `CertVerification::RootStore` as a server's `ServerTlsPolicy::Mutual` verification (MB-R-108) is rejected at construction as client-only.

**MB-R-173** — With a server's `ServerTlsPolicy::Mutual` and `CertVerification::Skip` (MB-R-108): no client certificate still fails the handshake, but a presented one is accepted without CA validation (signature check performed, chain/identity check skipped).

**MB-R-174** — A server's `ServerTlsPolicy::Tls` never requests a client certificate.

**MB-R-109** — With `tls` set on a client, the server certificate is verified per `verification`: `RootStore` against the native root store plus every `extra_ca_files` entry, any one anchor sufficing; `CaFiles` against exactly the named `ca_files`, not the native store, `ca_files` non-empty; `Skip` accepts any server certificate unauthenticated.

**MB-R-110** — A client presents a TLS identity when, and only when, its policy is `ClientTlsPolicy::Mutual`; `Tls` and `None` present none.

**MB-R-175** — A client's presented TLS identity (MB-R-110) follows its `identity`: `CertSource::Files` presents the PEM pair at `cert_file`/`key_file`; `CertSource::SelfSigned` presents MB-R-138's cached ephemeral pair.

**MB-R-176** — `CertSource::Ephemeral` is rejected at construction as a client identity (MB-R-110).

**MB-R-111** — A TLS handshake failure is distinguished from a connection-refused/transport error and from a request timeout. Client: a failed connection attempt under MB-R-050–MB-R-055.

**MB-R-177** — A server-side TLS handshake failure (MB-R-111) ends only that connection attempt, not the accept loop or other connections.

**MB-R-178** — A server logs a TLS handshake failure (MB-R-111) at Error level with the peer's socket address, its offered certificate identity where the handshake exposed one, and the error description.

**MB-R-112** — The RTU connection config carries no `tls` field; TLS applies to TCP transports only.

## Region-declaration diagnostics

**MB-R-129** — When `Memory::add_ranges` returns `false` at a module-construction, module-reconfiguration, or runtime register-edit call site, the module logs the rejection to its message log at Warning level.

**MB-R-184** — The MB-R-129 rejection log entry identifies the register name (single named register) or "read-range gap cell" plus slave id and register kind (explicit-read-range gap), the rejected address/range, and the incompatible-overlap cause.

**MB-R-185** — `Memory::add_ranges`'s return value and the silent non-application are unchanged by the MB-R-129 logging.

## Monitor

**MB-R-140** — A monitor module is configurable only on `Rtu` or `Ascii`; `RtuOverTcp`, `AsciiOverTcp`, `Tcp`, `Udp` are invalid for the monitor role, since only a physical serial bus carries traffic between other devices.

**MB-R-191** — Resolving a module instance spec with `role = monitor` on any transport other than `Rtu` or `Ascii` (MB-R-140) fails configuration resolution with a role/transport compatibility error.

**MB-R-141** — A monitor never writes to its serial port. It opens receive-only and decodes whatever RTU (or Ascii) frames pass without participating.

**MB-R-192** — A monitor's serial-port open or read failure retries with the server's serial-open backoff policy (MB-R-130–MB-R-134), gated by the monitor device config's own `reconnect` (default enabled).

**MB-R-142** — A monitor decodes each frame per the transport's framing (MB-R-114 for RTU-style, the Ascii equivalent) into slave id, function code, address, quantity/value(s), and, for an exception response (function code with bit `0x80` set), an exception code.

**MB-R-193** — The first frame a monitor decodes (MB-R-142) after the bus falls idle is a request; the next frame carrying the same slave id and function code (or that code with `0x80` set) is its matched response. A request with no such frame before the next request begins is logged unmatched.

**MB-R-194** — A frame failing CRC (RTU) or LRC (Ascii), or otherwise malformed, is logged by the monitor at Warning level and discarded; decoding resumes at the next frame boundary (MB-R-142).

**MB-R-143** — The monitor's log carries one entry per completed request/response pairing and one per unmatched request, each with MB-R-142's decoded fields plus a timestamp. A request to slave id 0 (broadcast) is logged complete on its own (MB-R-102's fire-and-forget semantics), never marked unmatched.

**MB-R-144** — A monitor maintains an observed-value table keyed by (slave id, table kind), MB-R-026's key shape, for the nine table-shaping operations `ReadCoils`, `ReadDiscreteInputs`, `ReadHoldingRegisters`, `ReadInputRegisters`, `WriteSingleCoil`, `WriteSingleRegister`, `WriteMultipleCoils`, `WriteMultipleRegisters`, `ReadWriteMultipleRegisters`.

**MB-R-195** — A successful (non-exception) read among MB-R-144's operations writes the response's words into the monitor's observed-value table at the request's range.

**MB-R-196** — A write among MB-R-144's operations writes the request's own value(s) into the monitor's observed-value table at its range (matched or, for a broadcast, immediately), independent of any response.

**MB-R-197** — A matched pair with an exception code, and an unmatched request, write no value into the monitor's observed-value table (MB-R-144), but every slave id reaching an MB-R-143 entry is marked seen, so any slave with recorded traffic appears in the unit id listing (UI-R-060) before its first value.

**MB-R-145** — A monitor lets the user author display-only register definitions against its observed-value table: (slave id, table kind, address, format), the same format machinery as a `RegisterDef` (data-contract.md `## Address ranges in the store`) with no access-direction field.

**MB-R-198** — Applying a monitor interpretation (MB-R-145) decodes the observed-value table's current raw words at that address as a store cell would be decoded; an address with no value observed yet renders as not-yet-observed, never decoding zeroed memory as a value.

**MB-R-146** — A monitor captures each MB-R-143 entry into a structured, per-slave-id message record: timestamp, status (`OK`, `Unmatched`, `Exception(code)`), operation (request's function code). For MB-R-144's nine operations the record also carries address, quantity, and value(s); every other operation carries none.

**MB-R-199** — Each slave id keeps its own bounded ring of the 200 most recent monitor message records (MB-R-146), oldest evicted first, independent of other slave ids' rings and of MB-R-143's free-text log.

**MB-R-147** — For each slave id, a monitor derives a recency marker for every (table kind, address) touched by an MB-R-146 record's address/quantity range, timestamped at that record's timestamp. A marker is active for 2 seconds, the register table's change-highlight duration (`ferrowl::module::modbus::table::CHANGE_HIGHLIGHT`), then lapses.

**MB-R-148** — A monitor lets the user edit an existing interpretation in a slave id's set (MB-R-145), replacing kind/address/format in place, under the existing name or a new one.

**MB-R-216** — A monitor lets the user remove an existing interpretation from a slave id's set (MB-R-145, MB-R-148), deleting it.

**MB-R-217** — Neither editing (MB-R-148) nor removing (MB-R-216) an interpretation writes to the bus or touches the observed-value table (MB-R-144).

**MB-R-150** — Before opening its RTU or Ascii serial port, on initial start or reconnect (MB-R-050–MB-R-055 client, MB-R-130–MB-R-134 server, MB-R-192 monitor), a module instance checks every other configured instance in the session for an Rtu/Ascii endpoint on the same path (after `~` expansion).

**MB-R-200** — On an MB-R-150 path match the module instance skips the OS-level open for that attempt, reports a distinct path-conflict status/log entry, then retries on the ordinary open-failure backoff cadence, recovering once the conflicting instance stops or moves off that path.

**MB-R-201** — With `reconnect` disabled, an MB-R-150 path match makes the single attempt report the same path-conflict status (MB-R-200) before stopping.

**MB-R-151** — The add/edit register dialog's Value and Default Value inputs are hidden and unfocusable for a `ReadOnly` register on a **client** module (MB-R-159 excludes it from client writes). On a **server**, where MB-R-090 bypasses access checks, they stay shown and editable regardless of access.

**MB-R-152** — A monitor module's displayed status follows MB-R-137's three-state rule with "serial port open" for "transport connected": `CONNECTED` while the port is open and read; `RECONNECTING` while the task runs but the port is not open (MB-R-130–MB-R-134, MB-R-192); `DISCONNECTED` while the task is not running.

**MB-R-154** — A format's display text is its name followed by a parenthesized qualifier: numeric → byte order (`Big Endian` or `Little Endian`); `Ascii` → alignment (`Left` or `Right`).

**MB-R-218** — In a format's display text (MB-R-154) the name part renders `Ascii` as `ASCII` and every other format as named in the data contract's format table.

**MB-R-219** — A format's display text (MB-R-154) never shows register order, resolution, or bit-field selector.

**MB-R-155** — A codec error naming a format renders it with the format's display text (MB-R-154), not a derived debug form.
