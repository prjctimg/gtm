# Phase 1 — Wire Format Interoperability

**Goal**: Make the JSON on the wire match GTM Protocol v2 so a Rust client can talk to a non-Rust daemon and vice versa.

## Source of truth
- `prjctimg/gtm.spec` `protocol.md` § "Wire Format: JSON (Command Socket)"
- `prjctimg/gtm.spec` `commands.md` § "Command Envelope" + § "handshake"
- `prjctimg/gtm.spec` `fixes/gtm-rs.md` items 2, 3, 4, 6

## Acceptance criteria

### 1.1 Flat command params (no variant wrapper)
- File: `gtm-core/src/ipc.rs`
- Add `#[serde(untagged)]` to `pub enum DaemonReq` (line ~247).
- After the change, `serde_json::to_value(&DaemonReq::Play{path:"/a.mp3".into(), start_pos:0.0})` MUST emit `{"path":"/a.mp3","start_pos":0.0}` (NOT `{"play":{...}}`).
- `WireReq` flattening then yields `{"id":1,"cmd":"play","path":"/a.mp3","start_pos":0.0}` on the wire.
- Verify with: `cargo test -p gtm-core ipc` (add round-trip test) and `cargo build`.

### 1.2 Daemon dispatch on `cmd` string
- File: `gtmd/src/daemon.rs` reader task (~line 442)
- After `wire_req: WireReq` is parsed, the daemon must dispatch on `wire_req.cmd` (string), not by attempting `serde_json::from_value::<DaemonReq>(wire_req.params)` blindly. If the string maps to a known variant, deserialize params into that variant's struct. If the cmd is unknown, emit `WireRes::err(id, format!("unknown command: {cmd}"))` and continue (do not close — that's handled in Phase 2).
- Add a helper `fn parse_cmd(cmd: &str, params: Value) -> Result<DaemonReq, String>` in `gtm-core/src/ipc.rs` that matches against known `cmd` strings and reconstructs the variant.
- Update the client (`gtm-core/src/client.rs` `send_request_by_id` ~line 752) to keep emitting the explicit `cmd` string and the flattened `params` value. No behavioural change required once DaemonReq is untagged.

### 1.3 Handshake response carries daemon version
- File: `gtm-core/src/ipc.rs` `DaemonRes::Handshake` (line ~327) and `to_wire` (line ~368)
- Add a constant `pub const PROTOCOL_VERSION: u32 = 1;` in `ipc.rs`.
- `DaemonRes::Handshake { version, daemon, daemon_version }` `to_wire` must emit `{"version": <daemon's PROTOCOL_VERSION>, "daemon": <str>, "daemon_version": <str>}` alongside `id`/`ok`.
- The daemon (`gtmd/src/daemon.rs:587-595`) must send `version: PROTOCOL_VERSION`, NOT echo the client's requested version.

### 1.4 Version negotiation
- If client `version` > `PROTOCOL_VERSION`: respond with
  `WireRes::err(0, format!("protocol version {v} not supported, daemon supports {PROTOCOL_VERSION}"))` and DO NOT mark client authenticated. Client must disconnect (handle in `gtm-core/src/client.rs`).
- If client `version` <= `PROTOCOL_VERSION`: ok + daemon version (existing path).

### 1.5 Fix MetadataChanged event shape
- File: `gtm-core/src/ipc.rs:190-192`
- Change `WireEvent::new("metadata_changed", serde_json::json!({ "event": detail }))` to `serde_json::json!({ "detail": detail })` so the wire object is `{"event":"metadata_changed","detail":"..."}` (no field-name collision with the `event` tag).

## Verification
- `cargo build --workspace`
- `cargo test --workspace`
- Manual: write a test that serializes a `WireReq` and asserts the JSON matches the spec examples in `protocol.md`.
- Add integration test in `gtmd/tests/daemon_test.rs` that sends raw spec-shaped JSON (`{"id":0,"cmd":"handshake","version":1,"client":"test"}`) over a Unix socket without going through Rust enum serialization, and asserts the response contains `"version":1`, `"daemon":"gtmd-rs"`, `"ok":true`.

## Commit message
`fix(spec): phase 1 — flat wire params, daemon version in handshake, version negotiation`