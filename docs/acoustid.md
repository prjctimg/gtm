# AcoustID / music recognition ("built-in Shazam") — feasibility writeup

**Status:** feasibility study (no code changed — item 11 of `PROMPT.md`).
**Scope:** identify the *currently playing* track with the decoded audio gtm
already has. True microphone capture ("point it at the speaker") is analysed
but deferred.

---

## 1. What the "Shazam" experience would look like

A key that fingerprints audio and, within a second or two, shows:

```
Now identifying…
  → "Lower Your Eyelids to Die With the Sun"
    M83 · Before the Dawn Heals Us
    score 0.94 · via AcoustID / MusicBrainz
```

Two distinct source modes:

1. **Identify the current track** — fingerprint the audio gtm is *already
   decoding* (local file, YouTube, Spotify). No mic involved. This is the
   practical, self-contained version.
2. **Microphone mode** — capture ambient audio (cpal), fingerprint ~2–5 s
   chunks, look up. This is the full "Shazam from any speaker" experience but
   needs mic permission handling, resampling, chunked fingerprinting and a UI
   affordance for when the mic is denied. Recommended as a phase 2.

## 2. How the pieces fit

Building blocks, all open source:

| Layer | Component | Notes |
|---|---|---|
| Fingerprint extraction | **Chromaprint** (C, v1.6.1, 2026-07-28) | The *de facto* audio fingerprint algorithm for MusicBrainz-world identification. Takes **raw uncompressed PCM**, no decoder of its own. |
| Fingerprint in Rust | `rusty-chromaprint` (bindings) or `chromaprint-next` (pure-Rust, bit-identical fingerprints) | Both on crates.io. `chromaprint-next` needs no system lib / FFI, ideal for a TUI project that already ships everywhere. |
| Lookup service | **AcoustID web service** (`/v2/lookup`) | POST/GET `fingerprint` + `duration` (+ `client` API key) → JSON with scored matches and nested MusicBrainz metadata (recording title, artists, release groups, cover-art MBID). |
| Metadata | **MusicBrainz** (the lookup returns recording/release MBIDs; cover art comes from Cover Art Archive by MBID — gtm already has cover workflows). | |
| Decoded audio | gtm's existing pipeline | `gtmd` decodes local files via symphonia/rodio and streams Spotify via librespot into the rodio mixer (`stream.rs` `PcmStreamSource`, `ChannelSink`). A raw-PCM tap point already exists in that chain (converter → f32 samples). |

### Why this fits gtm naturally

- **No decoder work needed.** Chromaprint wants decoded PCM; the daemon already
  produces exactly that for whatever is playing (file *or* stream).
- **No new native deps** if `chromaprint-next` is used. The existing decode
  pipeline converts to f32 already; Chromaprint wants s16 monophonic, so the
  tap is a resample step away.
- **Lookup is a plain HTTP call** — gtmd already talks to YouTube, Spotify,
  Deezer, Apple services; an AcoustID request is the same shape of work.

## 3. Feasibility assessment

### 3.1 Effort and confidence

| Part | Effort | Confidence |
|---|---|---|
| Tap PCM from the active decode path (daemon) | Small — hook into the converter/mixer input | High: the decoded samples already flow through one place |
| Encode to the Chromaprint input format (f32 → s16 mono, 11025 Hz is chromaprint's native rate) | Small (resampling) | High |
| Fingerprint via `chromaprint-next` | Small — ~50 lines (`Fingerprinter`, `feed_samples`, `finish` → base64) | High: crate is a documented drop-in; fallback `rusty-chromaprint` exists |
| AcoustID lookup + parse | Small — one `reqwest` call, already a dep pattern in gtmd | High: stable REST API |
| Key + registration | **Process work, not code** — register the app at acoustid.org, get a free app-wide API key | High |
| UI (notification with result, error states) | Small — gtm notification/footer infra exists | High |

Realistic estimate: **a working "identify current track" is a focused
weekend-to-week task** (a few hundred lines across gtmd + gtm, plus the API key
flow). It does not require touching the audio rust-crate stack.

### 3.2 The honest caveats

- **AcoustID is a *lookup*, not a magic Shazam.** It only returns matches that
  exist in the crowdsourced fingerprint database linked to MusicBrainz. Obscure
  or unreleased material → "no match", even though the track is right.
- **API key.** The service is free for non-commercial apps but requires
  registering an application and shipping its key (same class of work as the
  existing Spotify/Tidal/Deezer credentials, but *far less* auth machinery —
  no OAuth, no tokens, just a static key). Commercial use needs a paid plan via
  AcoustID OÜ.
- **Accuracy on streams.** Fingerprinting the decoded output works for any
  source (YouTube, Spotify, local). Re-encodes/re-broadcasts still match well
  (chromaprint tolerates compression), but degraded/noisy stream audio lowers
  match scores. The lookup returns scores; gtm can show the top match and its
  score rather than asserting certainty.
- **Privacy/network note.** The fingerprint is derived audio content sent to a
  third-party service. Local files *do not need* uploading if the goal is just
  metadata (see §5, offline match against gtm's own library first).

### 3.3 Mic mode (deferred phase 2)

- Needs `cpal` for capture (gtm deliberately avoids a hard cpal dep outside
  rodio; it's an optional dependency in the daemon's backend story), plus
  sample-rate conversion (mic ≈ 44.1–48 kHz → chromaprint's 11.025 kHz) and
  segmentation (identify on rolling 2–5 s windows).
- Chromaprint fingerprints are length-ish dependent — short windows reduce
  accuracy vs. the full-track case. Real-world Shazam-like UIs hide this with
  continuous re-lookup; fine as an enhancement, but it's the bulk of the extra
  work and risk.
- **Recommendation: door #1 only now.** The "built-in Shazam" label is
  achievable for *identification of what's playing in gtm*; true ambient mic
  recognition is a worthwhile but clearly separate follow-up.

## 4. Proposed architecture

```
gtmd (daemon)
 ├─ existing decode chain (symphonia / librespot → rodio mixer)
 │     tap decoded f32 → resample to 11025 Hz mono s16
 │     → chromaprint-next :: Fingerprinter (collect ~30–120 s)
 │     → base64 fingerprint + duration
 ├─ AcoustID client (new, small module; reqwest, like youtube/spotify clients)
 │     GET /v2/lookup?fingerprint=..&duration=..&client=KEY&meta=recordings+releasegroups
 │     → parse top N scored results (title, artists, release, MBIDs)
 ├─ optional offline fast path (see §5)
 └─ IpcResult::Recognition(...) → gtm TUI notification + optional cover fetch

gtm (TUI)
 ├─ key: identify current track → issue daemon request, "Identifying…" footer
 └─ render result (title/artist/score) via existing notification/footer infra
```

Config: `acoustid.api_key` alongside the existing provider credentials.

## 5. Bonus: offline "identify own library" cheap win

Before hitting the network, fingerprinting **local files is not even
necessary** — gtm already has full `TrackInfo` metadata. The interesting
offline variant is *duplicate/unknown-file identification* (fingerprint files
without tags and match against the fingerprinted library). That's per-file
background work (a "fingerprint library" job), independent of AcoustID, and a
nice later extension — worth mentioning because it could be built without any
API key at all.

## 6. Key risks & mitigations

| Risk | Mitigation |
|---|---|
| `chromaprint-next` unmaintained / fingerprint mismatch | Pin version; have `rusty-chromaprint` (FFI) as fallback; verify against known-good `fpcalc` output in tests |
| API key handling in a community app | Same pattern as existing provider keys; key failure → clear "AcoustID not configured" hint in the result UI |
| No-match / low-score UX | Always show score; distinguish "not found" from "service unavailable"; keep the notification non-blocking |
| Audio tap point captures the *already-playing* track only | Correct per scope (identify current track); mic mode is phase 2 |

## 7. Verdict

**Feasible, low-risk, and well-aligned with the codebase.** None of the
hard parts (decoding, network clients, notifications) are new — the only new
pieces are a fingerprinting crate and one REST lookup. The pragmatic first
iteration is: **identify the current track** with a registered AcoustID key,
`chromaprint-next`, and the decoded PCM already flowing through `gtmd`'s mixer.
Mic-based ambient recognition is deferred as phase 2. The single most important
expectation to set: AcoustID is a *crowdsourced lookup* — coverage is broad but
not universal, so results must be presented as scored matches, never as
certainty.