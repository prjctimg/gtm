# Manual Playback with gtmd

Test the daemon end-to-end using `socat` (or `nc`) to send IPC commands over the Unix socket.

## 1. Build & Start the Daemon

```bash
# Build everything
cargo build

# Run the daemon in test mode (ephemeral socket, no log file)
RUST_LOG=debug cargo run --bin gtmd -- --test-mode --verbose
```

The daemon prints the socket path on startup:

```
INFO daemon started on /run/user/1000/gtmd.socket
```

## 2. Send Commands with socat

Open another terminal. Use `socat` to connect to the socket:

```bash
# Connect to the daemon socket
socat - UNIX-CONNECT:/run/user/1000/gtmd.socket
```

{"Queue":{"action":{"AddFolder":{"path":"/home/prjctimg/.local/share/gtm/audio/"}}}}
Type a JSON request line and press Enter:

### Ping (expect `{"Pong":null}` back)

```json
{"Ping":null}
```

### GetStatus (expect `{"Ok":{"version":1}}` back)

```json
{"GetStatus":null}
```

### Play a file

Audio files live in `~/.local/share/gtm/audio/`. Use the full path:

```json
{"Play":{"path":"/home/prjctimg/.local/share/gtm/audio/Future - Codeine Crazy (Official Audio).opus"}}
```

Expected response: `{"Ok":{"version":2}}`

### Queue commands

Manage the playback queue — list, add files, add folders, remove, reorder.

#### List queue

```json
{"Queue":{"action":{"List":null}}}
```

Response: `{"QueueState":{"version":2,"tracks":[...],"cursor":0}}`

#### Add a single file to queue

```json
{"Queue":{"action":{"Add":{"path":"/home/prjctimg/.local/share/gtm/audio/track1.opus","position":null}}}}
```

#### Add multiple files to queue

```json
{"Queue":{"action":{"AddMany":{"paths":["/path/to/track1.opus","/path/to/track2.mp3"]}}}}
```

#### Add an entire folder (recursive scan)

Scans recursively for audio files (`mp3`, `flac`, `ogg`, `wav`, `m4a`, `aac`, `opus`, `wma`):

```json
{"Queue":{"action":{"AddFolder":{"path":"/home/prjctimg/Music/Album"}}}}
```

#### Clear queue

```json
{"Queue":{"action":{"Clear":null}}}
```

#### Remove at index

```json
{"Queue":{"action":{"Remove":{"index":0}}}}
```

#### Move track

```json
{"Queue":{"action":{"Move":{"from":0,"to":2}}}}
```

### Set queue (replace entire queue with given tracks)

```json
{"Queue":{"action":{"Set":{"paths":["/path/to/a.opus","/path/to/b.flac"],"start_idx":0}}}}

### Pause

```json
{"Pause":null}
```

### Stop

```json
{"Stop":null}
```

### Seek (seconds)

```json
{"Seek":{"position_secs":30.0}}
```

### Set Volume (0-100)

```json
{"SetVolume":{"volume":75}}
```

### Toggle Shuffle

```json
{"ToggleShuffle":null}
```

### Cycle Repeat (0=Off, 1=One, 2=All)

```json
{"CycleRepeat":{"mode":"All"}}
```

### Next / Prev Track

```json
{"Next":null}
```

```json
{"Prev":null}
```

### Quit (stops daemon)

```json
{"Quit":null}
```

## 3. Receive Events

When connected with `socat`, the daemon also sends **binary event frames** to all
connected clients after each mutation or position update. These are `bincode`-encoded
`WireFrame` structs with a 4-byte big-endian length prefix.

To decode them, use the `gtm-cli` utility (once implemented) or inspect raw hex:

```bash
# Raw hex dump (frames arrive ~10Hz during playback)
socat - UNIX-CONNECT:/run/user/1000/gtmd.socket | xxd | head -20
```

A position-update frame looks like:

```
00000000: 00 00 00 3c  # length = 60 bytes
00000004: 01 00 00 00  # version = 1
          ...
```

## 4. Debug Logs

The daemon prints all requests and events to stderr at debug level:

```
DEBUG handle_request: Play { path: "/home/prjctimg/.local/share/gtm/audio/Future - Codeine Crazy (Official Audio).opus" }
DEBUG cmd_play: loading /home/prjctimg/.local/share/gtm/audio/Future - Codeine Crazy (Official Audio).opus
INFO  playback started: Future - Codeine Crazy
DEBUG handle_audio_event: Position(1.234)
```

## 5. socat One-Liners (no interactive shell)

```bash
# Ping
echo '{"Ping":null}' | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket

# Play (response appears on stdout, events discarded)
echo '{"Play":{"path":"'"$HOME"'/.local/share/gtm/audio/Future - Codeine Crazy (Official Audio).opus"}}' \
  | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket

# Play + read events for 2 seconds (head -c to limit binary output)
(echo '{"Play":{"path":"'"$HOME"'/.local/share/gtm/audio/Future - Codeine Crazy (Official Audio).opus"}}';
 sleep 2) \
  | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket | head -c 200 | xxd
```

## 6. Full Workflow Example

```bash
# Terminal 1: start daemon
cargo run --bin gtmd -- --test-mode --verbose

# Terminal 2: play a track, wait, seek, stop
echo '{"Play":{"path":"'"$HOME"'/.local/share/gtm/audio/Future - Codeine Crazy (Official Audio).opus"}}' \
  | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket
sleep 1
echo '{"Seek":{"position_secs":10.0}}' | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket
sleep 2
echo '{"Pause":null}' | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket
sleep 1
echo '{"Stop":null}' | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket
echo '{"Quit":null}' | socat - UNIX-CONNECT:/run/user/1000/gtmd.socket
```

## 7. Integration Test

The existing integration test does all of the above programmatically:

```bash
cargo test -p gtmd --test daemon_test -- --nocapture
```

Note: the test requires opus files in `~/.local/share/gtm/audio/`.
