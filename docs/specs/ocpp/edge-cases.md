# OCPP — Edge Cases and Known Limitations

Boundary behavior, error semantics, intentional constraints. The known-limitations section below (`## Known limitations — intentional constraints`) is working as implemented; recorded so it is not "fixed".

---

## Framing boundaries

| ID | Condition | Behavior |
|---|---|---|
| **OC-E-001** | Frame not valid JSON | framing error: logged, skipped, **connection stays up**. No id to answer |
| **OC-E-002** | Valid JSON, not an array | framing error: logged, skipped. No id to answer |
| **OC-E-003** | Message-type id not 2, 3, or 4 (or not an integer) | framing error: logged, skipped. No id to answer |
| **OC-E-004** | Malformed **Call** (type 2) with string `uniqueId`: wrong arity, or non-string `action` | logged, answered CallError `FormationViolation` with the recovered id |
| **OC-E-005** | Malformed Call, `uniqueId` not a string | logged, skipped; nothing to address a CallError to |
| **OC-E-006** | Malformed **CallResult** or **CallError** (types 3, 4) | logged, skipped; never answered, even with a readable id |
| **OC-E-007** | Extra elements beyond the expected arity | rejected; arity exact |
| **OC-E-008** | Unrecognized `errorCode` on an inbound CallError | accepted, read as `GenericError`. No error |
| **OC-E-009** | Binary / ping / pong frame | ignored; not OCPP-J |
| **OC-E-010** | WebSocket close frame | ends the connection cleanly |
| **OC-E-011** | Transport error while reading | logged; ends the connection |

---

## Call and reply boundaries

| ID | Condition | Behavior |
|---|---|---|
| **OC-E-012** | Inbound Call names an action the negotiated version lacks | CallError `NotImplemented`; connection up |
| **OC-E-013** | Inbound Call payload fails to deserialize | CallError `FormationViolation` |
| **OC-E-014** | Inbound Call payload fails the version's validation rules | CallError `FormationViolation` |
| **OC-E-015** | Handler rejects an inbound Call | CallError with the handler's code, description, details; connection up |
| **OC-E-016** | Handler's response fails to encode | CallError `FormationViolation` (serialization failure) |
| **OC-E-017** | Awaited outbound Call exceeds the reply timeout | entry discarded; caller gets `GenericError` ("call timed out"); a later reply is discarded |
| **OC-E-018** | Awaited outbound Call whose action fails to encode | caller gets the encoding failure as a rejection; nothing sent |
| **OC-E-019** | Outbound Call on a closed connection | caller gets `GenericError` ("connection closed") |
| **OC-E-020** | Connection torn down with Calls in flight | **every** pending caller failed with `GenericError` ("connection terminated") |
| **OC-E-021** | Inbound CallResult/CallError for an unknown id | discarded silently |
| **OC-E-022** | Inbound reply for a fire-and-forget Call | discarded silently (no entry registered) |
| **OC-E-023** | Two replies for one id | first completes and removes the entry; second discarded |
| **OC-E-024** | Slow or blocking inbound handler | own task; cannot stall the read pump or delay other Calls |
| **OC-E-025** | Outbound frame channel full (64 pending) | sender waits; never silently dropped |
| **OC-E-026** | Command channel full (32 pending) | sender waits |

---

## Connection-drop boundaries

| ID | Condition | Behavior |
|---|---|---|
| **OC-E-027** | Connection drops mid-call (CS or CSMS) | reader ends, role's command loop signalled, connection torn down, every pending Call failed, disconnect hook fires |
| **OC-E-028** | Connection drops on the CS | module goes offline, auto-Heartbeat and auto-MeterValues halt, heartbeat counter resets, warning logged; reconnect per OC-E-084 |
| **OC-E-029** | Connection drops on the CSMS | that connection deregistered; accept loop and other connections unaffected |
| **OC-E-030** | CS socket dropped without explicit `:stop`, then `:start` | stale handle torn down first, then a fresh dial |
| **OC-E-031** | `:start` on an already-connected CS | no-op |
| **OC-E-032** | CSMS listener fails to bind | logged as an error; retry per OC-E-085 |
| **OC-E-033** | `accept()` itself errors | logged; accept loop keeps running |

---

## Security boundaries

| ID | Condition | Behavior |
|---|---|---|
| **OC-E-034** | CSMS has Basic Auth, request has no `Authorization` header | HTTP **401**, handshake refused; expected credential never disclosed |
| **OC-E-035** | CSMS has Basic Auth, header mismatch | HTTP **401** |
| **OC-E-036** | CSMS has no Basic Auth, request sends one | accepted; header ignored |
| **OC-E-037** | Request lacks the version's subprotocol token | HTTP **400**, handshake refused |
| **OC-E-038** | TLS handshake fails on an accepted socket | logged with peer address; socket dropped. Listener keeps accepting |
| **OC-E-039** | CS connects to a TLS CSMS whose certificate is not trusted | dial fails; module reports a connect failure |
| **OC-E-040** | Dialog client-side Root Store toggle and CA list after Skip-Verify toggled On | both hidden and excluded regardless of state, resolving to `CertVerification::Skip` (OC-R-111, OC-R-145) |
| **OC-E-041** | Dialog client-role Skip-Verify toggle while TLS selector is Off | hidden entirely; a plain or Basic-Auth-only connection has no certificate to verify (OC-R-111) |
| **OC-E-042** | CSMS `ServerTlsPolicy::Mutual`, `verification` resolves to `CaFiles` with zero `ca_files` | fails at construction/`resolve()`, never at listener start (OC-R-039) |
| **OC-E-043** | CSMS `ServerTlsPolicy::Mutual` with `identity: CertSource::SelfSigned` and `verification: CaFiles` with ≥1 file | permitted; self-signed identity and client-cert CAs are independent (OC-R-040) |
| **OC-E-044** | CSMS `ServerTlsPolicy::Mutual` with self-signed certificate, `CaFiles` with zero files | fails at construction, as any zero-file `CaFiles` (OC-R-039) |
| **OC-E-045** | Dialog server-role Self-Signed toggle (shown at TLS/mTLS) toggled On after `cert_file`/`key_file` text entered | resolved identity `CertSource::SelfSigned`, both files excluded; stored text preserved for Off (OC-R-143, OC-R-144) |
| **OC-E-046** | Dialog TLS selector moved to Off after certificate paths and CA entries entered | resolved policy `None`, no payload; every hidden widget keeps its state; displayed protocol reverts to `ws://` (OC-R-127, OC-R-163) |
| **OC-E-047** | Dialog Basic Authentication On, TLS selector Off | accepted: Profile 1, credentials over plain `ws://` (OC-R-165) |
| **OC-E-048** | Hand-written `ws://` instance whose own-role policy is not `None`, reopened in the dialog | selector shows TLS or mTLS, Protocol display shows derived `wss://`; confirming writes `wss://` back, promoting the inert pairing into a live one (OC-R-161). The instance stays inert (OC-R-042/OC-R-097) only while unedited |
| **OC-E-049** | Configured PEM file cannot be opened, or contains no certificate or no private key | CS dial / CSMS bind fails with a TLS error, before socket work |
| **OC-E-050** | `username` without `password` (or vice versa) | Basic Auth **not** enabled; field inert |
| **OC-E-051** | `wss://` **server** endpoint, identity `CertSource::Ephemeral` | binds with an ephemeral self-signed certificate, logs the fallback; never silently plain TCP |
| **OC-E-052** | `ws://` **client** endpoint with TLS material | material inert; connection plain |
| **OC-E-053** | `ws://` CS endpoint whose `ClientTlsPolicy` would fail validation (empty `ca_files`, `Ephemeral` identity, unreadable PEM) | connects in plaintext; policy neither validated nor loaded (OC-R-097) |
| **OC-E-054** | `ws://` **server** endpoint with `ServerTlsPolicy` other than `None` | material inert; listener plain TCP, symmetric with the `ws://` client |
| **OC-E-055** | `ServerTlsPolicy::Mutual` client certificate signed by any one of several `ca_files` | accepted; `ca_files` is a trust-anchor set, not an ordered chain (OC-R-039) |
| **OC-E-056** | `ServerTlsPolicy::Mutual` with `CertVerification::Skip`, no client certificate presented | handshake still fails: `Skip` skips the CA/identity check on a *presented* cert, does not make presenting optional (OC-R-134) |
| **OC-E-057** | Dialog server-role Skip Verify toggled On while mTLS selected | shared CA list hidden, resolved verification `CertVerification::Skip` regardless of entries; list preserved for Off (OC-R-151) |
| **OC-E-058** | Dialog client-role (CS) Self Signed toggled On while mTLS selected | Client Cert/Key inputs hidden, resolved identity `CertSource::SelfSigned` regardless of text; text preserved for Off (OC-R-147, OC-R-148) |
| **OC-E-059** | Self-signed pair, once generated for a module instance | cached and reused across every bind/connect/reconnect and `:restart`/`:reload`, including a config edit leaving the source self-signed; regenerated only on a transition *into* self-signed; a fresh instance discards the cache; pair never on disk (OC-R-131/OC-R-132/OC-R-115) |
| **OC-E-060** | CS `CertVerification::CaFiles` with empty `ca_files` | refused at construction and dialog submit (OC-R-130/OC-R-154), same reasoning as the Modbus client role |
| **OC-E-061** | `CertVerification::RootStore` on a CSMS's client-certificate verification | rejected at construction, no toggle offered (OC-R-133); reasoning in MB-E-071 |
| **OC-E-062** | `extra_headers` entry whose `name` collides (case-insensitively) with a client-controlled header | construction fails, naming the header (OC-R-153) |
| **OC-E-063** | `extra_headers` entry whose `name` or `value` has a byte outside the allowed grammar (e.g. CR/LF) | construction fails, naming header and field (OC-R-118) |
| **OC-E-064** | `extra_headers` on a server-role device config | inert; client-only |

---

## Simulator boundaries

| ID | Condition | Behavior |
|---|---|---|
| **OC-E-065** | Inbound Call targets a connector/EVSE the station lacks | CallError `PropertyConstraintViolation` ("unknown connectorId"); connection up |
| **OC-E-066** | Inbound Call targets id `0`, or names none | always valid — the charge point itself |
| **OC-E-067** | Inbound Call the CS simulator does not model | **default-accepted** with the `Default`-derived response |
| **OC-E-068** | Charging profile stack level exceeds the configured maximum | rejected, nothing applied. Absent that key, no ceiling |
| **OC-E-069** | Charging profile with no limit in its schedule | accepted; no limit applied |
| **OC-E-070** | Clear-charging-profile with an unrecognized purpose | clears nothing, still succeeds |
| **OC-E-071** | Clear-charging-profile with no purpose criterion | clears **every** per-purpose limit |
| **OC-E-072** | Cancel-reservation whose id matches nothing | succeeds; nothing cleared |
| **OC-E-073** | Remote start with no connector target | falls back to the **first** connector |
| **OC-E-074** | Remote stop for a transaction id not live | succeeds; nothing stopped |
| **OC-E-075** | Configuration write to a read-only key | rejected |
| **OC-E-076** | Configuration write to an unknown key | **creates** it as writable |
| **OC-E-077** | Configuration read naming unknown keys | known ones returned; unknown listed as unknown |
| **OC-E-078** | BootNotification response with interval `0` or none | treated as unset: 30 s heartbeat |
| **OC-E-079** | Heartbeat interval below 1 s | clamped to 1 s |
| **OC-E-080** | RFID accept-lists all empty | every tag accepted (open mode) |
| **OC-E-081** | Tag listed only on connector A, presented at connector B | rejected at B, not inherited sideways; but it **does** authorize a connector-less authorization request, which unions every list |
| **OC-E-082** | Message buffer exceeds 200 messages | oldest evicted. Messages teed to the log file each refresh tick, so an evicted message is still logged if it survived until the next tick (OC-E-093) |
| **OC-E-083** | 1.6 ICCID/IMSI/meter serial/meter type left empty | omitted from `BootNotification` entirely, not sent as an empty string (wire field requires length ≥ 1 when present) |

---

## Known limitations — intentional constraints

### CS reconnect resets on handshake, not on message exchange

**OC-E-084** — A CS reconnects on a failed dial or dropped connection with the shared bounded-exponential-backoff driver (MB-R-051; OC-R-048, OC-R-105–107), governed by `reconnect` (default enabled). A non-terminate command sent while backing off is dropped, not queued (MB-R-054). Backoff resets to 1 s as soon as the WebSocket handshake completes, before any OCPP message (OC-R-105): a peer that accepts the socket and immediately drops it, every time, sees the backoff reset on every attempt — retrying near 1 s — rather than growing as it would if reset required a message exchange.

### CSMS bind retry

**OC-E-085** — A CSMS whose bind fails retries with the shared backoff driver (OC-R-139, OC-R-108–109). A server still binds **automatically on creation** (OC-R-138). Governed by the same `reconnect` field as the CS role (default enabled); disabled, a failed bind ends the module task (MB-R-130–134).

### Unbounded connections and no idle timeout

**OC-E-086** — A CSMS serves any number of concurrent connections, no cap, no idle timeout. A station that connects and goes silent holds its connection and registry entry indefinitely.

### No version-neutral semantic layer

**OC-E-087** — Deliberately no neutral abstraction over the three versions. The surface is the per-version action set, so every action can be listed and every raw payload inspected; sharing between 2.0.1 and 2.1 is plain shared functions. Consequence: adding a version means adding its action table, inbound handlers, and action-spec module — no single seam makes it free.

### `NotifyPeriodicEventStream` is not an action

**OC-E-088** — OCPP 2.1's `NotifyPeriodicEventStream` is a one-way streaming datagram with no request/response pair, so it cannot be an action-table entry. Absent from the 90-action set; can be neither sent nor received.

### The RFID accept-list is the only CSMS authorization model

**OC-E-089** — The simulated CSMS accepts or rejects a tag purely by list membership. No local auth list, auth cache, expiry, parent id tag, or group-id handling; those actions are default-accepted, changing no CSMS state.

### Server-side configuration is transient

**OC-E-090** — A CSMS's observed state (station entries, connectors, per-station configuration) is discarded on `:stop` and `:restart`, never written to the device config. Only RFID accept-lists persist. The client role persists its connector table and configuration-key store.

### Connector count is unbounded

**OC-E-091** — Nothing caps the connectors a client-role device config declares or that may be added at runtime.

### A stale reply is silently dropped

**OC-E-092** — A reply arriving after its Call timed out finds no entry and is discarded with no log line. Caller's side: the Call failed; wire's side: the peer answered.

### A message burst larger than the buffer can lose log lines

**OC-E-093** — Messages are teed from memory into the log file once per refresh tick (~100 ms), by sequence number. A message both created **and** evicted between two ticks (more than 200 messages in one tick) never reaches the file. Unreachable at any realistic OCPP rate.

### A cancelled inbound Call handler's side effects may have partially applied

**OC-E-094** — Teardown (OC-R-121) aborts an in-flight handler rather than waiting. A side effect already begun is not rolled back; only the reply is guaranteed never sent.

### The CS and CSMS connection drivers stay separate

**OC-E-095** — `cs::core::run` and `csms::core::run_connection` share a skeleton (build dispatch, start connection, `on_connected`, `select!` loop over commands, `shutdown`, `on_disconnected`) and are deliberately not unified: differences thread through the whole body, not the ends. CSMS carries a `ConnectionId` into every handler call (`on_connected(conn)`, `handle_call(conn, action)`, `on_disconnected(conn)`), so `CsActionHandler` and `CsmsActionHandler` have different signatures and no single generic bound covers both without an adapter trait. CS returns `RunEnd::{Terminated, Disconnected}`, which its retry loop classifies to stop or back off; CSMS returns `()` and deregisters from the registry. Command enums differ in name and variants. Consequence: a change to the duplex loop is made twice — unifying costs a handler-adapter trait plus a generic over the return type to save ~45 lines across two 90-line files, and would obscure the retry-classification contract `RunEnd` makes explicit. `wait_backoff` in `ferrowl-util` is already shared: a utility over a channel and a clock, not a lifecycle abstraction spanning the roles.
