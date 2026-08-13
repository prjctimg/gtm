# 01 — IPC Protocol Redesign

## Motivation

The current IPC has two issues:

1. **Mixed framing** — JSON responses and bincode events share the same socket without clear demultiplexing. The `parse()` heuristic (first byte `{` → JSON, else bincode) is fragile.

2. **Blocking request-response** — Every `DaemonClient::send()` writes then blocks on `read()` until the matching response arrives. This freezes the TUI render loop during IPC.

## Architecture

```
┌─────────────────────────────┐     ┌─────────────────────────────┐
│         gtm (client)        │     │        gtmd (daemon)        │
│                             │     │                             │
│  ┌───────────────────────┐  │     │  ┌───────────────────────┐  │
│  │   cmd_tx   cmd_rx     │  │     │  │   req_rx / dispatch   │  │
│  │ (mpsc channel)        │  │     │  │ (JSON lines)          │  │
│  │     ↕                 │  │     │  └───────────┬───────────┘  │
│  │  ┌─────────────┐      │  │     │              │              │
│  │  │ IPC Worker  │──────┼──┼─────┼── socket     │              │
│  │  │ (background │      │  │     │   (write)    │              │
│  │  │  task)      │◀─────┼──┼─────┼── socket     │              │
│  │  └─────────────┘      │  │     │   (read)     │              │
│  │        ↕              │  │     └───────────────┘              │
│  │  ┌─────────────┐      │  │                                    │
│  │  │ Pulse Rx    │──────┼──┼─────► Pulse Tx (binary events)    │
│  │  │ (dedicated  │      │  │       (dedicated socket pair)      │
│  │  │  reader)    │      │  │                                    │
│  │  └─────────────┘      │  │                                    │
│  └───────────────────────┘  │                                    │
└─────────────────────────────┘     └─────────────────────────────┘
```

## Changes

### 1. snake_case IPC commands

All enum variants in `DaemonReq`, `DaemonRes`, `DaemonEvent`, `QueueAction`, `LibraryAction` change from CamelCase to snake_case via `#[serde(rename_all = "snake_case")]`.

**Before:**
```json
{"Play": {"path": "/foo.mp3", "start_pos": 0.0}}
{"Queue": {"action": {"Clear": null}}}
```

**After:**
```json
{"play": {"path": "/foo.mp3", "start_pos": 0.0}}
{"queue": {"action": {"clear": null}}}
```

This is a breaking change — client and daemon must be updated atomically.

### 2. Dedicated pulse socket pair

A separate `socket_path_pulse` is created alongside the main socket (e.g., `gtmd.pulse`). The daemon writes binary-encoded `DaemonEvent` frames to all connected pulse sockets. Clients connect a dedicated reader thread/task that never sends data.

**Benefits:**
- Events never interleave with command responses
- The main socket stays clean JSON-only
- No heuristic first-byte sniffing

### 3. Background IPC worker

`DaemonClient` is replaced by a background task that owns the Unix socket connection:

```
cmd_tx ──► IPC Worker Task ──► daemon socket
              │
              ├──► response channel (oneshot per request)
              └──► event channel (broadcast)
```

**Flow:**
1. TUI sends `TuiCommand` to `cmd_tx` (non-blocking mpsc)
2. IPC Worker receives it, serializes to JSON, writes to socket
3. IPC Worker reads JSON lines from socket
4. Response lines are matched to pending requests via ordered sequence
5. Events are pushed to a shared `Vec<DaemonEvent>` (via `Arc<Mutex<>>`)
6. TUI reads events via `drain()` and applies to state mirror

**Commands no longer block the render loop.**

### 4. Response matching

Since the protocol is sequential (one request → one response, in order), the IPC Worker maintains a `VecDeque<oneshot::Sender<Result<DaemonRes>>>`. When it writes a request, it stores the sender. When it reads a response, it pops the sender and sends the response.

## Implementation Plan

| Step | Files | Change |
|------|-------|--------|
| 1 | `gtm-core/src/ipc.rs` | Add `#[serde(rename_all = "snake_case")]` to all IPC enums |
| 2 | `gtm-core/src/client.rs` | New `DaemonClient` using background task + channels |
| 3 | `gtmd/src/daemon.rs` | Add pulse socket listener + broadcast |
| 4 | `gtm/src/app.rs` | Update to use new non-blocking `DaemonClient` |
| 5 | `gtm/src/ui.rs` | Remove `ensure_daemon_running` ping hack |

## Sequence Diagram

```
TUI Render Loop          IPC Worker Task              Daemon
     │                        │                         │
     │── cmd_tx.send(Play)───►│                         │
     │                        │── JSON "play" ──────────►│
     │                        │                         │── load file
     │                        │◄──── JSON "ok" ──────────│
     │                        │── discard ok response    │
     │                        │◄──── bincode event ──────│ (via pulse)
     │◄── drain events ──────│                         │
     │── update state mirror  │                         │
     │── render               │                         │
```

## Migration

Phase 2 implements all of these changes atomically. Until then, Phase 1 bug fixes use the existing protocol with minimal changes.
