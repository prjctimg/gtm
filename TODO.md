# TODO — GTM-RS Comprehensive Fixes

## High Priority

- [ ] **7d — Termux socket path fix** — Ensure socket parent dir exists, add fallback path
- [ ] **7a — Q quit fix** — Spawn quit() task, push daemon_quitting event before exit
- [ ] **3 — Command palette fixes** — Fix labels, quit action, repeat cycle order
- [ ] **7c — Buffer overflow/underflow guards** — Max line length on daemon, max buffer on client
- [ ] **7b — Playback continuation** — Respect RepeatMode::Off in advance_queue, chain queue_set+play

## Medium Priority

- [ ] **2 — Footer EQ + SleepTimer modules** — Add to FooterModule enum, all presets, state_machine handler
- [ ] **1 — Cover image loading** — Add cover to track_info_popup, validate cover_path, trigger fetch
- [ ] **6c — Delete/playlist state sync** — Update tracks_cache after remove, refresh playlists after add/remove
- [ ] **6b — Favourite toggle state sync** — Optimistic local update, new ToggleFavourite IPC variant
- [ ] **6a — Edit metadata navigation** — hjkl/arrow keys, Enter→next field, confirm prompt, Ctrl+Enter save
- [ ] **5 — Separate binary/JSON** — Remove binary from primary socket, simplify client parse
- [ ] **4 — Daemon command handling** — Spawn dispatch tasks, add semaphore, refactor shared state
