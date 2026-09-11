# OCPP — Requirements

Version-generic engine, Charging Station (CS) and CSMS roles, OCPP-J framing over WebSocket, call correlation and timeouts, security profiles, simulated CS/CSMS state machines, OCPP module configuration.

IDs stable, append-only (`OC-R-nnn`). See [`../README.md`](../README.md). Companions: [`api-contract.md`](./api-contract.md) (action tables, config fields), [`data-contract.md`](./data-contract.md) (wire frames, payload shapes, state model), [`edge-cases.md`](./edge-cases.md).

---

## Versions

**OC-R-001** — Exactly three versions: 1.6, 2.0.1, 2.1. Every version reachable in both roles.

**OC-R-002** — Each version declares its complete action table: exactly 28 actions for 1.6, 64 for 2.0.1, 90 for 2.1.

**OC-R-003** — Each version's table partitions into a CS-originated set and a CSMS-originated set: disjoint, together covering every action.

**OC-R-004** — Each version declares exactly one WebSocket subprotocol token: `ocpp1.6`, `ocpp2.0.1`, `ocpp2.1`. Fixed by version, never configurable.

**OC-R-005** — Every CSMS-originated action carries a connector scope `None` (no connector/EVSE field), `Optional` (field may be omitted), or `Required`, derived from the request's top-level connector/EVSE target.

**OC-R-006** — Each action exposes a `Default`-derived request template and response template, retrievable by wire action name. Unknown name → no template, not an error.

**OC-R-007** — 2.1 is a strict superset of 2.0.1: every 2.0.1 action name exists in 2.1; the 26 additional actions are additive.

**OC-R-008** — A version's request-validation rules apply to every inbound Call before it reaches a handler, not to outbound Calls.

---

## OCPP-J framing

**OC-R-009** — Wire format OCPP-J: JSON arrays in WebSocket **text** frames. Exactly three envelopes: Call `[2, uniqueId, action, payload]`, CallResult `[3, uniqueId, payload]`, CallError `[4, uniqueId, errorCode, errorDescription, errorDetails]`.

**OC-R-010** — Decoding rejects a frame that is not valid JSON, not an array, carries an unknown message-type id, has the wrong element count for its type (4 / 3 / 5), or whose `uniqueId`, `action`, or `errorCode` is not a JSON string.

**OC-R-011** — A unique id is an arbitrary string. Every outbound Call generates a fresh UUID v4; an inbound Call's id is echoed back verbatim on the reply.

**OC-R-012** — `errorCode` is one of exactly ten spec-fixed codes ([`api-contract.md`](./api-contract.md)). An unrecognized code on the wire maps to `GenericError`, not a frame failure.

**OC-R-013** — Non-text WebSocket frames (binary, ping, pong) are ignored without error, never treated as OCPP-J.

---

## Connection engine

**OC-R-014** — A connection is full-duplex: both peers may originate Calls concurrently on one socket. The engine is role-agnostic; CS and CSMS share one connection implementation.

**OC-R-015** — Outbound frames serialize through a single writer; frames never interleave on the wire.

**OC-R-016** — Each inbound Call dispatches in its own task; a slow or re-entrant handler never blocks reading further frames.

**OC-R-017** — An outbound Call awaiting a reply is registered in a correlation table keyed by unique id; an inbound CallResult or CallError completes the matching entry.

**OC-R-018** — A CallResult carries no action name; the reply is decoded using the originating action to select the response type.

**OC-R-019** — A CallResult or CallError matching no pending entry is discarded silently, connection undisturbed.

**OC-R-020** — Every awaited outbound Call is bounded by the configured reply timeout. On expiry the entry is discarded and the caller receives a `GenericError` rejection.

**OC-R-021** — A Call may be sent fire-and-forget: no correlation entry, no reply delivered.

**OC-R-022** — On teardown every still-pending outbound Call fails with a `GenericError` rejection; no caller left waiting.

**OC-R-023** — A peer's WebSocket close, or a transport error while reading, ends the connection: the role's command loop is signalled, the connection torn down, the role's disconnect hook fires.

**OC-R-024** — A malformed inbound frame is logged and does not tear down the connection; it is skipped except where OC-R-098 requires a CallError.

**OC-R-121** — Teardown does not block on an in-flight inbound Call handler: `shutdown()` aborts every running handler. An aborted handler never sends its reply.

---

## Errors

**OC-R-025** — A handler can reject an inbound Call at protocol level by returning a call error (code, description, details), sent as a CallError frame, connection intact.

**OC-R-026** — An inbound Call naming an action the negotiated version lacks → CallError `NotImplemented`.

**OC-R-027** — An inbound Call whose payload fails to deserialize into the action's request type, or fails the version's validation rules → CallError `FormationViolation`.

**OC-R-028** — A CallError received in reply to an outbound Call surfaces to the caller as a rejection carrying the peer's code, description, details verbatim, never a connection failure.

**OC-R-098** — A frame that fails to decode but is identifiable as a Call (`messageTypeId` 2) with a string `uniqueId` is answered with CallError `FormationViolation` carrying that id.

**OC-R-099** — A frame whose id cannot be recovered (not JSON, not an array, no `messageTypeId` of 2, or non-string `uniqueId`) is logged and skipped, no reply.

**OC-R-100** — A malformed CallResult or CallError is never answered, id recoverable or not.

---

## Security — transport

**OC-R-029** — Three security profiles: Profile 1 (HTTP Basic Auth over plain `ws://`), Profile 2 (TLS, server certificate only), Profile 3 (mutual TLS).

*Coverage note: OC-R-029 is an umbrella; substance covered by per-profile requirements and tests — OC-R-030/OC-R-031 (Profile 1, `ferrowl-ocpp/tests/ws_loopback_security.rs`), OC-R-096 (Profile 2, `ferrowl/src/module/ocpp/config/{session,device}.rs`), OC-R-039/OC-R-040 (Profile 3, `ws_loopback_security.rs`, `ferrowl-ocpp/src/security.rs`). No test cites OC-R-029 itself.*

**OC-R-030** — A CS with Basic Auth configured sends `Authorization: Basic <base64(user:pass)>` on the WebSocket upgrade request.

**OC-R-031** — A CSMS with Basic Auth configured rejects any upgrade whose `Authorization` header is absent or mismatched, answering HTTP 401, never disclosing the expected credential.

**OC-R-032** — A CSMS rejects an upgrade not advertising the version's subprotocol token, answering HTTP 400. On acceptance it echoes the token in `Sec-WebSocket-Protocol`.

**OC-R-033** — A Basic Auth password never appears in a log line, including via debug formatting.

**OC-R-034** — A CS's trust anchors are exactly those its `verification` names: `CertVerification::RootStore` = webpki root store plus every `extra_ca_files` entry; `CertVerification::CaFiles` = exactly the named `ca_files`; `CertVerification::Skip` = no anchor (OC-R-036).

**OC-R-035** — A CS presents a client certificate when, and only when, its policy is `ClientTlsPolicy::Mutual`; `Tls` and `None` present none.

**OC-R-166** — A CS presenting a client certificate (OC-R-035) with `CertSource::Files` presents the PEM pair at `cert_file`/`key_file`.

**OC-R-167** — A CS presenting a client certificate (OC-R-035) with `CertSource::SelfSigned` presents OC-R-115's cached ephemeral pair.

**OC-R-168** — `CertSource::Ephemeral` as a CS client identity (OC-R-035) is rejected at construction (MB-R-176).

**OC-R-036** — A CS with `CertVerification::Skip` accepts any server certificate without authenticating it (signature check performed, chain/identity check skipped).

**OC-R-129** — A CS with `CertVerification::RootStore` verifies the server certificate against the native root store plus every `extra_ca_files` entry.

**OC-R-130** — A CS with `CertVerification::CaFiles` verifies the server certificate against exactly the named `ca_files`, not the native store; `ca_files` non-empty.

**OC-R-037** — A CSMS TLS server certificate comes from the PEM files of `CertSource::Files` or an ephemeral self-signed certificate (`SelfSigned` or `Ephemeral`).

**OC-R-131** — A CSMS's self-signed pair (OC-R-037) is generated once and cached for the module instance's life, reused across every bind, reconnect-driven rebind, `:restart`/`:reload`, and config edit leaving the identity self-signed.

**OC-R-169** — A torn-down and freshly constructed CSMS instance discards the cached self-signed pair (OC-R-131) and generates a new one.

**OC-R-170** — A CSMS's cached self-signed pair (OC-R-131) is never written to disk.

**OC-R-132** — A CSMS's cached self-signed pair (OC-R-131) is regenerated only when the identity changes to `SelfSigned`/`Ephemeral` from `Files`, or the TLS mode transitions from `None` to `Tls`/`Mutual` while the identity is already self-signed.

**OC-R-038** — A generated self-signed CSMS certificate carries the listener's configured host as a subject-alternative name, plus `localhost` when the host differs.

**OC-R-039** — With a CSMS's `ServerTlsPolicy::Mutual`, client-certificate verification is governed by `verification`. `CaFiles`: a client presenting no certificate, or one not signed by any configured `ca_files`, is rejected; any one CA suffices; `ca_files` non-empty, construction failing otherwise.

**OC-R-133** — With a CSMS's `ServerTlsPolicy::Mutual` (OC-R-039), `CertVerification::RootStore` is rejected at construction as client-only.

**OC-R-134** — With a CSMS's `ServerTlsPolicy::Mutual` (OC-R-039), `CertVerification::Skip` still rejects a client presenting no certificate, but a presented one is accepted without CA validation (signature check performed, chain/identity skipped).

**OC-R-040** — `ServerTlsPolicy::Mutual` with a self-signed CSMS certificate (`identity: CertSource::SelfSigned`) is permitted with either `verification` mode, `CaFiles` (≥1 file) or `Skip`. `CaFiles` with zero files remains rejected at construction (OC-R-039).

**OC-R-041** — Failing to open, parse, or find a certificate or private key in a configured PEM file fails the CS connection start or CSMS listener start with a TLS error, before any socket work.

**OC-R-042** — The endpoint scheme is authoritative for a server's transport: `wss://` binds a TLS-terminated listener; `ws://` binds plain TCP even when a `ServerTlsPolicy` other than `None` is configured, the whole policy inert.

**OC-R-097** — The scheme is authoritative for a client's transport the same way: a `ws://` CS endpoint connects in plaintext and ignores any TLS material.

**OC-R-095** — A `wss://` **server** endpoint whose `identity` is `CertSource::Ephemeral` binds with an ephemeral self-signed certificate and reports the fallback in the module log.

**OC-R-171** — A `wss://` server endpoint whose `identity` is `CertSource::SelfSigned` binds with a self-signed certificate as OC-R-095 does, without logging a fallback.

**OC-R-172** — A `wss://` server endpoint (OC-R-095) never silently binds plain TCP.

**OC-R-096** — A `wss://` server's TLS material follows its `identity`: `SelfSigned` presents an ephemeral self-signed certificate; `Files` presents the named certificate + key; `Ephemeral` presents OC-R-095's logged fallback. Self-signed pair cadence per OC-R-037.

**OC-R-112** — A half-configured certificate pair is not representable in a config file: `CertSource::Files` carries `cert_file` and `key_file` as required fields, so naming one without the other fails to deserialize. The rule survives only in the dialog, whose inputs are independently editable: error border on a blank cert/key input while shown; refuses to close on submit while either of a role's pair is blank, CSMS identity and CS mTLS identity alike (MB-R-171).

**OC-R-115** — Under `ClientTlsPolicy::Mutual` with `identity: CertSource::SelfSigned`, a CS presents an ephemeral self-signed certificate/key pair as its mTLS identity, generated and cached per OC-R-037, never written to disk.

**OC-R-126** — The OCPP device security config carries its TLS material as a `tls` sub-block holding one `ServerTlsPolicy` under `server` and one `ClientTlsPolicy` under `client`, serialized `[security.tls.server]`/`[security.tls.client]`.

**OC-R-156** — `username` and `password` remain flat members of `security` (OC-R-126), shared by both roles.

**OC-R-157** — Each policy in the `tls` sub-block (OC-R-126) independently defaults to `None`, so an absent `[security.tls]` block is exactly both policies `None`.

**OC-R-158** — A CS or CSMS decides whether TLS is configured by matching its own role's policy variant in the `tls` sub-block (OC-R-126); the other role's policy is inert, never validated against this role's rules.

**OC-R-159** — The endpoint scheme remains authoritative over the `tls` sub-block's policy (OC-R-126, OC-R-042).

**OC-R-117** — A CS device config may declare `extra_headers`, an ordered list of `HeaderDef { name, value }`.

**OC-R-152** — Every configured `extra_headers` header (OC-R-117) is sent on the WebSocket upgrade in addition to any header the client sets itself (OC-R-030's `Authorization`, OC-R-004's subprotocol token).

**OC-R-153** — Construction rejects an `extra_headers` `HeaderDef` (OC-R-117) whose `name` case-insensitively matches a client-controlled header (`Authorization`, `Host`, `Upgrade`, `Connection`, `Sec-WebSocket-Key`, `Sec-WebSocket-Version`, `Sec-WebSocket-Protocol`, `Sec-WebSocket-Extensions`), naming it in the error.

**OC-R-118** — `HeaderDef.name` matches the HTTP token grammar (visible ASCII, no separators or whitespace); `HeaderDef.value` contains only printable ASCII (0x20–0x7E), excluding CR/LF and other controls. Construction rejects any violation, naming the offending header and field.

**OC-R-119** — `extra_headers` is a client-only device config field, not exposed through the `--ocpp` key=value CLI form, consistent with the other list-shaped client-only fields (`connectors`, `config`).

**OC-R-120** — A CS's connection status lines (e.g. "Client disconnected") go to the module log, not the message log, which records only request/response pairs (data-contract.md `## Message log`).

---

## Security — setup dialog

**OC-R-110** — The OCPP setup dialog offers a server-role Self-Signed toggle, shown whenever the TLS selector (OC-R-127) is TLS or mTLS.

**OC-R-143** — With the server-role Self-Signed toggle (OC-R-110) On, the server `cert_file`/`key_file` inputs are hidden and the resolved identity is `CertSource::SelfSigned` regardless of text; validation does not require those files. The toggle has no effect on the client-CA list or the selector's mTLS position.

**OC-R-144** — The server-role Self-Signed toggle (OC-R-110) leaves the hidden `cert_file`/`key_file` inputs' stored text unmodified, so Off restores the paths and re-requires them.

**OC-R-111** — The dialog's client-side Root Store toggle and server-CA list (OC-R-125) are both hidden whenever Skip-Verify is On. The client-role Skip-Verify toggle itself is shown only when the TLS selector (OC-R-127) is TLS or mTLS, since a connection with TLS off has no certificate to verify.

**OC-R-145** — With the client-role Skip-Verify toggle (OC-R-111) On, the resolved verification is `CertVerification::Skip`.

**OC-R-146** — The client-role Skip-Verify toggle (OC-R-111) leaves the hidden Root Store toggle's and server-CA list's stored state unmodified, so Off restores what was entered.

**OC-R-113** — The dialog presents CA file paths through one shared list widget (server (CSMS) role: client-CA list, shown whenever mTLS is selected; client (CS) role: server-CA list, OC-R-125), mirroring MB-R-136: zero or more paths added, edited, or removed individually.

**OC-R-149** — An add-entry confirm on the shared CA list widget (OC-R-113) is rejected (sub-dialog stays open with an inline error, nothing appended) unless the path is non-empty, exists on disk, is not a directory, and has extension `pem`/`crt`/`key` (case-insensitive).

**OC-R-150** — Server role: mTLS with a non-empty shared CA list (OC-R-113) and Skip Verify Off → `ServerTlsPolicy::Mutual` with `CertVerification::CaFiles` holding exactly those files; an empty list there is a validation error.

**OC-R-151** — The dialog also offers a server-role Skip Verify toggle, shown whenever mTLS is selected: On hides the client-CA list (OC-R-113) and resolves verification to `CertVerification::Skip` regardless of entries, list preserved for Off.

**OC-R-116** — The dialog offers a CS (client) role Self Signed toggle, shown whenever mTLS is selected.

**OC-R-147** — With the CS (client) role Self Signed toggle (OC-R-116) On, the Client Cert/Key inputs are hidden and excluded from the resolved config regardless of text, resolving to `ClientTlsPolicy::Mutual` with `identity: CertSource::SelfSigned`; validation does not require those files.

**OC-R-148** — The CS (client) role Self Signed toggle (OC-R-116) leaves the hidden Client Cert/Key inputs' stored text unmodified, so Off restores and re-requires the paths.

**OC-R-125** — The dialog's client (CS) role offers, whenever Skip-Verify is Off and the TLS selector (OC-R-127) is TLS or mTLS, a Root Store toggle (default On) plus the shared CA list widget (OC-R-113) holding zero or more server-CA paths, replacing the single `ca_file` input.

**OC-R-173** — With the client-role Root Store toggle On (OC-R-125), the config resolves to `CertVerification::RootStore` with the CA list as `extra_ca_files`, empty or not.

**OC-R-174** — With the client-role Root Store toggle Off (OC-R-125), the config resolves to `CertVerification::CaFiles` with the CA list as `ca_files`.

**OC-R-154** — With the client-role Root Store toggle (OC-R-125) Off, an empty shared CA list is a validation error refusing to close the dialog (OC-R-150).

**OC-R-155** — Skip-Verify On hides both the client-role Root Store toggle and the shared CA list (OC-R-125) per OC-R-111.

**OC-R-127** — The dialog presents TLS through a single three-way selector, Off / TLS / mTLS, shown for both roles and applying to the instance's own role, with no "security level" or profile selection. The selector maps one-to-one onto the role's policy variant (Off → `None`, TLS → `Tls`, mTLS → `Mutual`), each variant contributing exactly its payload (OC-R-110, OC-R-111, OC-R-113, OC-R-116, OC-R-125).

**OC-R-160** — The TLS selector's (OC-R-127) Off position never resolves to a `Tls` variant with placeholder identity or verification, so a dialog-created station with TLS off carries the same `None` policy a file-created one does.

**OC-R-161** — The dialog's Protocol field is a read-only display derived from the TLS selector (OC-R-127) alone (`wss://` whenever the selector is other than Off, `ws://` at Off), and the resolved endpoint carries the scheme displayed, so the dialog can never produce the scheme/policy mismatch OC-R-042 and OC-R-097 render inert.

**OC-R-162** — The dialog carries no explanatory row beneath the Protocol display (OC-R-161).

**OC-R-163** — Changing the TLS selector (OC-R-127) leaves every hidden widget's stored state unmodified, so returning to a previous selection restores what was entered (MB-R-186).

**OC-R-128** — The dialog offers Basic Authentication as an On/Off selection independent of the TLS selector, shown for both roles, with username and password inputs on one line shown only while On.

**OC-R-164** — With Basic Authentication (OC-R-128) On and both inputs non-empty, `username`/`password` are set in `security`; Off → both unset regardless of text, stored text left unmodified.

**OC-R-165** — Basic Authentication (OC-R-128) On with TLS Off is valid (Profile 1, OC-R-029), as is On with TLS or mTLS.

---

## Role — Charging Station (CS, client)

**OC-R-043** — A CS dials a full WebSocket URL (scheme, host, port, path), advertising exactly its version's subprotocol token.

**OC-R-044** — The charge-point identity is the last non-empty path segment of the URL.

**OC-R-045** — A CS accepts commands on a command channel while connected: send a Call and await its typed reply, send without awaiting, terminate.

**OC-R-046** — A CS answers CSMS-originated Calls through a handler and exposes connect and disconnect lifecycle hooks.

**OC-R-047** — Terminating a CS, or closing its command channel, tears the connection down and ends the client task successfully.

**OC-R-048** — With `reconnect` enabled (default), a CS never ends its task on a failed dial or dropped connection; it waits a backoff and retries per MB-R-051.

**OC-R-135** — With `reconnect` disabled, a CS's failed dial or dropped connection ends the task with that error, after emitting a disconnected status.

**OC-R-105** — A CS's backoff resets to 1 s after any connection whose WebSocket handshake completed, regardless of whether any OCPP message was exchanged.

**OC-R-106** — Terminating a CS, or closing its command channel, while backing off aborts the wait immediately and ends the task with success (extends OC-R-047).

**OC-R-107** — `reconnect`, the endpoint, and the security (TLS/auth) configuration are re-read from the shared device config on every dial attempt, so an edit takes effect on the next reconnect without restart (MB-R-056).

**OC-R-114** — With `reconnect` enabled, a CS logs the failure reason for each failed dial or dropped connection, and the wait duration before each backoff wait (MB-R-051).

**OC-R-123** — A CS module's displayed status follows MB-R-137's three-state rule: `CONNECTED` while the WebSocket is open; `RECONNECTING` while the task runs but is not connected (OC-R-048); `DISCONNECTED` while the task is not running.

---

## Role — CSMS (server)

**OC-R-049** — A CSMS binds a TCP listener on a configured host and port and accepts CS connections in a loop, serving each concurrently. Port `0` binds an OS-assigned port; the bound address is retrievable.

**OC-R-050** — When TLS is configured, every accepted socket is TLS-terminated before the WebSocket handshake.

**OC-R-051** — Each accepted connection gets an opaque, monotonically increasing connection id from 1. The charge-point identity from the URL path is kept as metadata against that id, **not** used as the connection key, so reconnects and duplicate identities never collide.

**OC-R-052** — A CSMS accepts commands: send a Call to one connection with or without awaiting, broadcast a fire-and-forget Call to every live connection, disconnect one connection, terminate.

**OC-R-053** — Terminating a CSMS terminates every live connection and ends the accept loop.

**OC-R-108** — A CSMS's listener-bind backoff resets to 1 s once the listener has bound and accepted at least one connection.

**OC-R-109** — Terminating a CSMS while backing off from a failed bind aborts the wait immediately and ends the module task successfully (extends OC-R-053).

**OC-R-054** — A connection is deregistered when its loop ends, for any reason.

**OC-R-055** — A command addressing an unknown connection id fails that command alone: awaited Call → `InternalError` rejection; fire-and-forget → logged and dropped. Server keeps running.

**OC-R-056** — A CSMS answers CS-originated Calls through a handler told which connection the Call arrived on.

---

## Simulated Charging Station behavior

**OC-R-057** — A CS module maintains charge-point-wide state (model, vendor, firmware version, serial number, configuration/variable key store, CSMS-supplied heartbeat cadence, charge-point-level reservation) and a list of connector states, multiplexed over the single WebSocket.

**OC-R-058** — Each connector carries its own metering, status, transaction, per-purpose charging limits, RFID tag, reservation.

**OC-R-059** — A defined subset of CS-originated actions is *state-driven*: request built entirely from observed state, sent without a dialog. All others send through a dialog.

**OC-R-060** — While connected, the CS sends Heartbeat automatically at the cadence the CSMS returned in its BootNotification response, falling back to 30 s when absent or zero, never faster than 1 s.

**OC-R-061** — While connected, the CS sends MeterValues automatically about every 5 s per connector with a live transaction; none when no transaction is live.

**OC-R-062** — Losing the connection halts all automatic transmission and resets the heartbeat cadence counter.

**OC-R-063** — An inbound Call carrying a top-level connector/EVSE id the station lacks → CallError `PropertyConstraintViolation`. Id `0` and an absent id are always valid (the charge point itself).

**OC-R-064** — An inbound Call the CS simulator does not model is default-accepted with the action's `Default`-derived response.

**OC-R-065** — The CS answers configuration reads from its key store: a request naming keys returns the known ones and lists the unknown; a request naming none returns every key.

**OC-R-066** — A configuration write updates an existing writable key, is rejected for a read-only key, and creates a missing key.

**OC-R-067** — An inbound charging-profile installation is rejected when its stack level exceeds the configured maximum stack level; otherwise its limit applies to the targeted connector under the field matching the profile's purpose. Absent that configuration key, no ceiling.

**OC-R-068** — Clearing charging profiles erases only the per-purpose limit matching the request's purpose criterion, or every per-purpose limit when none is given. Unrecognized purpose clears nothing.

**OC-R-069** — A reservation is recorded at the level the request targets (charge point or connector) and cleared by a cancellation carrying the same reservation id, at whichever level holds it.

**OC-R-070** — A remotely started transaction mints a local transaction id, puts the targeted connector (absent an explicit target, the first) into a charging state, and transmits the transaction-start message (`StartTransaction` for 1.6, `TransactionEvent` with `eventType=Started` for 2.0.1/2.1) through the same send path the RFID/operator flow uses, so the CSMS learns the id via the normal wire message.

**OC-R-136** — A remote stop clears the transaction (OC-R-070), clears the transaction-scoped charging limit, returns the connector to available.

**OC-R-071** — A reset returns every connector to available, clears its transaction, zeroes its session energy.

**OC-R-122** — Whenever a transaction-start message (`StartTransaction` or `TransactionEvent` `eventType=Started`) is transmitted, by RFID/operator action or accepted remote-start (OC-R-070), the same send also transmits a `StatusNotification` for that connector with its updated status: `ChargePointStatus::Charging` for 1.6, `ConnectorStatusEnumType::Occupied` for 2.0.1/2.1.

**OC-R-072** — Ending a transaction clears only the transaction-scoped charging limit; default and maximum limits persist.

---

## Simulated CSMS behavior

**OC-R-073** — A CSMS module answers every CS-originated Call. Four are crafted: boot notification (accepted, current time, heartbeat interval), heartbeat (current time), authorization (accept/reject status), transaction start (freshly minted unique transaction id plus accept/reject status).

**OC-R-137** — A CSMS module answers every CS-originated Call other than the four crafted in OC-R-073 with the action's `Default`-derived response.

**OC-R-074** — A CSMS maintains RFID accept-lists at two levels: one charge-point-wide, one per connector/EVSE. A connector's effective set = its own list ∪ the charge-point-wide list.

**OC-R-075** — An empty effective set accepts every tag. A non-empty set accepts only listed tags.

**OC-R-076** — An authorization request (names no connector) is checked against the charge-point-wide list ∪ **every** connector list. A transaction start (names a connector) is checked against that connector's effective set only.

**OC-R-077** — A CSMS observes every connected station's connectors from inbound traffic and tracks them per connection; connectors are not pre-configured for the server role.

**OC-R-078** — Every inbound Call and its reply, and every outbound Call and its reply, is recorded for display and logging, tagged with its charge-point/connector scope.

---

## Module lifecycle and configuration

**OC-R-079** — An OCPP module instance is a charging station (client) or a management system (server), never both, speaking exactly one version.

**OC-R-080** — The OCPP version is a property of the **device config**, not the session entry: a device's scripts call version-specific actions and are version-locked.

**OC-R-081** — The session entry carries only instance name, device config path, and endpoint (scheme, ip, port, path). Version, role, timeout, security, scripts, connectors, configuration keys, and (client role) CS boot identity live in the device config.

**OC-R-103** — A charging-station (client) device config persists the CS boot identity (model, vendor, firmware version, serial number) as it persists connectors and configuration keys: `:wd`/`:write-device` writes current values.

**OC-R-140** — Loading a charging-station device config seeds the persisted CS boot identity fields (OC-R-103) into CS state, overriding built-in defaults only when the field is present.

**OC-R-104** — In OCPP 1.6, charge-point-wide state also carries four optional identity fields: SIM ICCID, SIM IMSI, meter serial number, meter type. Each is persisted by `:wd`/`:write-device` as OC-R-103. Not applicable to 2.0.1/2.1.

**OC-R-141** — Each of the four 1.6 optional identity fields (OC-R-104) is seeded on load as OC-R-140, overriding built-in defaults only when the field is present.

**OC-R-142** — Each of the four 1.6 optional identity fields (OC-R-104) that is empty is omitted from `BootNotification` entirely; valued, it is included under its wire name (`iccid`, `imsi`, `meterSerialNumber`, `meterType`).

**OC-R-082** — Connection or listener configuration is rebuilt from the current module spec on every start, so an edited endpoint or security section takes effect on the next start.

**OC-R-083** — A client module does **not** connect automatically; only on explicit start.

**OC-R-138** — A server module binds its listener automatically on creation.

**OC-R-139** — With `reconnect` enabled (default), a server module's failed bind (OC-R-138) does not end the module task; it retries per MB-R-051. Disabled: a failed bind ends the task with that error, surfaced to the caller.

**OC-R-124** — A CSMS server module's displayed status follows MB-R-153's three-state rule: `CONNECTED` while the listener is bound; `RECONNECTING` while the task runs but is not bound (OC-R-083); `DISCONNECTED` while the task is not running.

**OC-R-084** — Restarting a module stops the current instance and starts a new one from the current spec. Restarting a server additionally discards every observed station entry.

**OC-R-085** — Changing a module's role or version replaces the view with one built for the new role/version. Changing anything else reconfigures the running instance in place, reconnecting only if it was connected.

**OC-R-086** — Switching a client's version keeps its Lua scripts and warns they may call actions the new version lacks.

**OC-R-087** — Each module keeps a bounded in-memory message log of the most recent 200 messages, evicting oldest first; complete history only in the configured log file.

**OC-R-088** — With file logging enabled, each message is written to the log file at most once, tracked by sequence number, so eviction from memory neither duplicates nor skips a message (OC-E-093 for the burst bound).

**OC-R-101** — Encoding an action or response to JSON for the message log never discards an encode failure silently: the failure is logged to the module's error channel before the payload degrades to JSON `null`.

**OC-R-102** — When a module view stops or (re)starts its backend for a settings change, version switch, or `stop`/`restart`, a stop or start failure is reported in the module message log at Error level, not discarded.

---

## Send dialogs

**OC-R-089** — Every action reachable through a send dialog is classified as exactly one of *typed* (flat property table with per-property kind, prefill source, optionality) or *raw JSON*.

**OC-R-090** — An action is raw-JSON when, and only when, its request's required fields include a nested object, or a repeated list with no optional escape hatch.

**OC-R-091** — Every raw-JSON action ships a template payload that decodes and validates against its own version's request type.

**OC-R-092** — A typed dialog always also offers a raw-JSON mode, prefilled from the current rows.

**OC-R-093** — An action whose required fields are a nested shape drivable by a few flat fields may have a typed dialog with a custom assembler folding those fields into the full nested request.

**OC-R-094** — A dialog-assembled payload is validated by decoding against the version's request type before sending; a payload failing to decode is reported and not sent.
