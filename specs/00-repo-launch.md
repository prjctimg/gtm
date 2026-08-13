# Spec 00 — Repository & Launch Readiness

Goal: make the gtm.rs repo presentable for a public launch (r/rust, Show HN,
r/unixporn). All items are repository-level (no application code).

Status: **Planned**.

---

## A1 — Remove `PROMPT.md` from git, ignore screenshots

- `git rm PROMPT.md` and add to `.gitignore`: `PROMPT.md` and `*.png` (or the
  two `Screenshot 2026-08-08 *.png` files by name).
- Do NOT commit `PROMPT.md` or the screenshots (worktree policy, see
  `specs/README.md`).

## A2 — Add a CI workflow

New file `.github/workflows/ci.yml`:

```yaml
name: CI
on:
  push:
  pull_request:
jobs:
  check:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: Swatinem/rust-cache@v2
      - name: Install ALSA headers (Linux)
        if: runner.os == 'Linux'
        run: sudo apt-get update && sudo apt-get install -y libasound2-dev
      - run: cargo fmt --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```

Note: on macOS the optional `mpris` feature (zbus) is disabled at build time by
the daemon's feature set only when needed; if the default build fails on
macOS, build with `--no-default-features` in gtmd — verify locally first.

## A3 — Add `CONTRIBUTING.md`

Cover: build-from-source (ALSA dev headers, Rust 1.81+), crate layout table
(gtm-core / gtm-audio / gtmd / gtm / gtm-mpris / release-gen), the green gates
(fmt / clippy / test), commit conventions (conventional commits), the
`tapes/` VHS reference for demo assets, and a pointer to the gtm.spec repo.

## A4 — Add issue templates

Under `.github/ISSUE_TEMPLATE/`:

- `bug_report.yml` — form: version, platform, terminal, repro steps, expected
  vs actual, logs (`gtmd.log`).
- `feature_request.yml` — form: problem, desired behavior, alternatives.
- `config.yml` — links to the wiki and gtm.spec.

## A5 — README overhaul

`README.md` currently has only a Version + Rust badge (lines 6-7), no feature
list, no keybinding cheat sheet, no screenshots.

- Badges row: CI status
  `https://img.shields.io/github/actions/workflow/status/prjctimg/gtm.rs/ci.yml`,
  license `GPL-3.0`, rustc 1.81+, GitHub release.
- Add a "Features" section (background daemon, YT/Spotify/Deezer integration,
  EQ presets + visualizer, cover art, themes, MPRIS, Termux/Android support).
- Add a keybinding cheat-sheet table mirroring `gtm-keybindings(1)`.
- Move the two worktree screenshots into `assets/screenshots/` and embed them.
- Add a demo-GIF `<img>` placeholder under the tagline (see
  `tapes/VHS.md` — the repo already documents VHS workflows) with a note on
  how to regenerate it.
- Replace the "unable to handle external contributions" paragraph with a short
  "Contributions welcome — see CONTRIBUTING.md".

## A6 — Cargo metadata

Root `Cargo.toml` `[workspace.package]` currently sets only
`version` / `edition` / `license` (lines 6-10). Add:

```toml
repository = "https://github.com/prjctimg/gtm.rs"
homepage = "https://github.com/prjctimg/gtm.rs"
keywords = ["music", "player", "tui", "audio", "youtube"]
categories = ["command-line-utilities", "multimedia::audio"]
readme = "../README.md"   # or per-crate READMEs
```

Inherit in all six crate manifests (`gtm-core`, `gtm-audio`, `gtmd`, `gtm`,
`gtm-mpris`, `release-gen`) via `repository.workspace = true`, etc.

## A7 — Install UX

Root is a virtual workspace manifest, so `cargo install --path .` fails with
`found a virtual manifest`. Document `cargo install --path gtm` in the README
"Build from Source" section (leave the `cargo build --release` + `make install`
path as the primary). Do not restructure the workspace.

## A8 — Mixer lock hardening

`gtm-audio/src/mixer.rs` (~35 sites) and `decode_thread.rs:151` use
`Mutex::lock().unwrap()`. Replace with
`lock().unwrap_or_else(|p| p.into_inner())` so a poisoned lock cannot panic the
daemon's 16 ms control loop. Hot-path sites to prioritize:

- `poll()` — mixer.rs:735-742 and 783-789 (position branch)
- `step_crossfade()` — mixer.rs:699-700
- `force_complete_crossfade()` — mixer.rs:628-629
- getters `current_position` (573-577), `duration` (582), `active_remaining`
  (586)
- control-plane loaders `load_active` (393-412), `load_active_decoded`
  (435-444), `play` (504), `stop` (522-525), `seek` (535-537)

Also replace `decode_thread.rs:105`
`.expect("failed to spawn decode thread")` with an error-propagating `Result`
(or keep if threading a return value is disproportionate — note the decision).

## A9 — Release profile

Root `Cargo.toml`:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
```

## A10 — Desktop file + icon bundling

- Add `assets/gtm.svg` (simple icon) and a `gtm.desktop` that references it.
- Include `gtm.desktop` + icon in the `gtm-full-{platform}.tar.gz` archives and
  the `gtm-full` `.deb` in `.github/workflows/release.yml` (the deb currently
  packages gtm/gtmd/man/completions/service — see the `assets` list in
  `gtm/Cargo.toml` `[package.metadata.deb]`).

## Acceptance

- `git status` clean of `PROMPT.md` / screenshots.
- CI badge shows a passing run.
- `cargo install --path gtm` works from a fresh clone.
- `cargo package` succeeds for the `gtm` crate (metadata complete).
