# Phase 4 — Docs, Tests, Compliance Script

**Goal**: Eliminate doc drift, remove dead `bincode` references, and make the compliance check script actually verify what it claims.

## Source of truth
- `prjctimg/gtm.spec` is the canonical spec; local copies must not contradict it.

## Acceptance criteria

### 4.1 Delete stale local docs
- Remove:
  - `docs/ipc-protocol.md`
  - `docs/spec.md`
  - `docs/manual-playback.md`
- Keep `docs/man/gtmd-ipc.1.md` (already v2-aligned) and the other manpages.
- Add a short `docs/protocol.md` (≤ 30 lines) that points readers to `https://github.com/prjctimg/gtm.spec` as the canonical source and lists the local manpages.

### 4.2 Fix `gtm-core/README.md`
- Line 14: remove `bincode` from the dependency list (it is no longer a dependency; `rmp-serde` is).
- Re-read the README section by section and update any other stale claims (e.g., `gtmd.socket` paths, wire-format descriptions) if present.

### 4.3 Fix `docs/termux.md`
- Line 229: change `$PREFIX/tmp/gtmd.socket` → `$PREFIX/tmp/gtm/gtmd.sock` (Phase 1 socket path).
- Any other path references in the same doc updated similarly.

### 4.4 Audit `gtm-core/tests/core_tests.rs` bincode usage
- Lines 65, 66, 167, 168, 315, 658, 660 reference `bincode::serialize`/`deserialize`.
- `bincode` is NOT in `gtm-core/Cargo.toml` dependencies. If the tests currently fail to compile, this is a hidden bug.
- Action: convert those round-trip tests to use `serde_json` (the JSON envelope is now the canonical wire format) or `rmp_serde` where binary is conceptually relevant. Remove all `bincode::` references.
- If `bincode` is listed under `[dev-dependencies]` somewhere already, instead leave the tests alone but verify they pass; otherwise migrate them.
- Verify with `cargo test -p gtm-core`.

### 4.5 Rewrite `check_compliance.sh`
- The existing script uses broken `grep A | grep B` invocations that always evaluate false; the script's "✓ All Phase 2-4 requirements met!" is vacuously true and misleading.
- Rewrite to use `rg` or proper `grep` with separate conditions and explicit returns. Add new assertions for:
  - `DaemonReq` carries `#[serde(untagged)]`.
  - `WireRes::ok`/`WireRes::err` exist.
  - `WireEvent.event` field is `String`.
  - `wire::encode` uses `rmp_serde`.
  - `PROTOCOL_VERSION` constant exists.
  - `parse_cmd` function exists in `ipc.rs`.
  - All v2 commands present (set_loudness_mode, scan_loudness, set_pre_gain, set_gapless, set_dynamic_mode, set_scrobble, organize_library).
  - All v2 events present (loudness_mode_changed, ..., library_organized).
  - Handshake timeout logic in `daemon.rs`.
  - Malformed-JSON `break` in `daemon.rs`.
- Exit non-zero with a clear message on any failure.

### 4.6 Update `AGENTS.md` (root) protocol reference
- Append a one-line pointer to `https://github.com/prjctimg/gtm.spec` under the "Architecture" section so future agents know the canonical spec.

## Verification
- `cargo build --workspace && cargo test --workspace && cargo clippy --workspace && cargo fmt --check`
- `./check_compliance.sh` exits 0.
- `rg -n 'bincode' docs/ gtm-core/ gtm/ gtmd/` returns no hits (or only historical CHANGELOG mentions, which are fine).

## Commit message
`chore(spec): phase 4 — drop stale docs, fix bincode tests, rewrite compliance script`