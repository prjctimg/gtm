# Phase 2 — Daemon Robustness

**Goal**: Enforce protocol-mandated connection lifecycle rules on the daemon side.

## Source of truth
- `prjctimg/gtm.spec` `protocol.md` § "Handshake", § "Error Handling"
- `prjctimg/gtm.spec` `fixes/gtm-rs.md` item 6

## Acceptance criteria

### 2.1 Per-client handshake timeout (10 s)
- File: `gtmd/src/daemon.rs` `accept_client` (~line 403)
- After spawning the reader task, spawn a watchdog tokio task that:
  - Sleeps 10 seconds.
  - Re-checks `inner.client_auth` for this `client_id`.
  - If still `false`, shuts down the client connection (drop the writer half or set a cancel token) and logs `"client {id}: handshake timeout"`.
- The watchdog must be cancelled when the reader task exits (use a `tokio::select!` or a `CancellationToken` per client).
- A non-handshaked client must not linger forever consuming the `client_auth` hashmap entry.

### 2.2 Malformed JSON closes the connection
- File: `gtmd/src/daemon.rs` reader loop (~line 435-441)
- Currently: `Err(e) => { warn!(...); continue; }`
- After change: `Err(e) => { warn!("client {id}: malformed JSON, closing: {e}"); break; }`
- Rationale: `protocol.md` "If the daemon receives malformed JSON, it MUST close the connection."

### 2.3 Oversized line closes the connection
- File: `gtmd/src/daemon.rs:431-434`
- Already correct (logs + `break;`). No behavioural change; add an explicit comment citing the spec for traceability.

### 2.4 Unknown `cmd` produces an error response (no close)
- File: `gtmd/src/daemon.rs` (~line 442-448) and `gtm-core/src/ipc.rs` `parse_cmd` (from Phase 1)
- If `parse_cmd` returns `Err(unknown_cmd_string)`, the daemon must send
  `WireRes::err(wire_req.id, format!("unknown command: {unknown_cmd_string}"))` and `continue` the reader loop.
- This must NOT close the connection (per spec only malformed JSON and oversized lines close; unknown commands are recoverable).

### 2.5 Unmatched command params produce an error response (no close)
- File: same as 2.4
- If `parse_cmd` parses the `cmd` string but `from_value` into the variant struct fails, send
  `WireRes::err(id, format!("invalid params for {cmd}: {e}"))` and `continue`.

## Verification
- New integration tests in `gtmd/tests/daemon_test.rs`:
  - `test_malformed_json_closes_connection`: send `"{not json}\n"`, assert read returns 0 (EOF) within 1 s.
  - `test_unknown_cmd_returns_error`: send `{"id":42,"cmd":"frobnicate"}\n`, assert response is `{"id":42,"ok":false,"error":"unknown command: frobnicate"}`.
  - `test_handshake_timeout`: connect, send nothing, assert EOF within ~11 s.
  - `test_pre_handshake_command_rejected`: send `{"id":1,"cmd":"ping"}` before handshake, assert `{"id":1,"ok":false,"error":"handshake required"}`.

## Commit message
`fix(spec): phase 2 — handshake timeout, malformed-JSON close, unknown-cmd error`