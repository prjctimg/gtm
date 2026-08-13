# Spec 17 — Actor Model IPC Refactor

## Requirement

> When you play tracks manually by pressing Enter on them in a list, the daemon/TUI communication remains functional but when you use the keybinding equivalents things go wrong with commands being buffered for 15s before being processed by the daemon when the connection is restored. This shows a fundamental flaw in our design and maybe we need to look at other approaches like the actor or the proactor models of dealing with the event loop.

## Root Cause

1. **IPC Worker** (`gtm-core/src/client.rs:554-613`): Processes ONE request at a time via `try_recv()` (line 580). After sending a request, it blocks on `read_response()` with a 15-second timeout (line 683). All other queued requests wait.

2. **Daemon dispatch** (`gtmd/src/daemon.rs:215-217`): Processes requests sequentially in `tokio::select!`. Slow commands (decode, library scan, sync) block all subsequent requests.

3. **Enter vs keybindings**: Enter key handlers in `app.rs` sometimes bypass the TUI command channels and call `DaemonClient` methods directly via `tokio::spawn`. Keyboard shortcuts go through `send_high()` → `handle_command()` → `tokio::spawn` → `client.play_pause().await`. Both end up at the same IPC worker bottleneck.

## Changes

### 17a. Daemon concurrent dispatch (`gtmd/src/daemon.rs`)

Replace sequential request processing with concurrent actor-style dispatch:

- Wrap mixer access in `Arc<tokio::sync::Mutex<dyn Mixer>>` (or use a dedicated mixer command channel)
- In the main `run()` loop, instead of `self.dispatch(client_id, req, reply_tx).await`:
  ```rust
  Some((client_id, req, reply_tx)) = self.req_rx.recv() => {
      let state = self.state.clone();
      let event_tx = self.event_tx.clone();
      let mixer = self.mixer.clone(); // needs Arc wrapping
      tokio::spawn(async move {
          Self::handle_request(state, mixer, event_tx, client_id, req, reply_tx).await;
      });
  }
  ```
- Each request handler runs concurrently. Only mixer-aquiring commands serialize via the mixer mutex.
- Library operations, status queries, and other read-only requests proceed in parallel.

### 17b. Mixer Arc wrapping (`gtm-audio/src/mixer.rs`, `gtmd/src/daemon.rs`)

- The `Mixer` trait is already `Send + Sync`. The `AudioMixer` uses `&mut self` methods.
- Wrap the mixer in `Arc<tokio::sync::Mutex<Box<dyn Mixer>>>` in the daemon
- Request handlers that need mixer access acquire the mutex lock
- This serializes only mixer operations while allowing all other requests to proceed concurrently

### 17c. IPC Worker concurrent requests (`gtm-core/src/client.rs`)

The IPC worker currently serializes all requests through a single socket. Two options:

**Option A (simpler)**: Keep the worker single-threaded but add request ID correlation:
- Each `PendingRequest` gets a unique `request_id`
- The worker writes requests with ID prefix: `{"id":123,...}\n`
- Responses include the same ID: `{"id":123,...}\n`
- Worker maintains a `HashMap<u64, oneshot::Sender>` for pending responses
- Reads responses in a loop, dispatching to the correct pending sender

**Option B (full)**: Use separate read/write tasks with multiplexed responses:
- Spawn a reader task that continuously reads from the socket
- Match responses to pending requests by ID
- Writer task sends requests from a queue

Implement **Option A** as it's less invasive.

### 17d. Priority routing (`gtm/src/app.rs`)

- The `high_pri_cmd_tx` channel already exists (unbounded)
- After the IPC worker supports concurrent requests, high-priority commands naturally proceed without waiting for slow commands
- No additional changes needed beyond 17c — the IPC worker will process requests as fast as the daemon can handle them

## Files Touched
- `gtmd/src/daemon.rs`: Concurrent dispatch, mixer Arc wrapping
- `gtm-core/src/client.rs`: Request ID correlation, concurrent request handling
- `gtm-core/src/ipc.rs`: Add `id` field to `DaemonReq` and `DaemonRes` (or use wire protocol framing)
- `gtm-core/src/wire.rs`: Add request ID to wire format
- `gtm-audio/src/mixer.rs`: No changes needed (Mixer trait already Send+Sync)
- `gtm/src/app.rs`: Minor — verify high_pri path works with new IPC

## Verification
- Multiple keyboard shortcuts in rapid succession all get processed (no 15s delay)
- Enter key and keyboard shortcuts have same responsiveness
- Daemon handles slow operations (library scan) without blocking other requests
- No data races or panics from concurrent mixer access
