# gtmd.dev — Documentation Site Architecture (Research + Recommendation)

**Status:** proposal — research only, no scaffold created yet.
**Scope:** a terminal-flavoured docs site for **gtm** served at `gtmd.dev`.
**Research input:** opencode's own docs platform (`opencode.ai/docs`), the
current content inventory in this repo, and the `ratzilla` framework
(Rust/WASM terminal-in-the-browser).

---

## 1. What we learned from opencode's docs architecture

opencode publishes its manual at `opencode.ai/docs` from a single repository
(`anomalyco/opencode`):

- **Static stack:** the site is built with **Astro + Starlight**, the docs
  theme on top of Astro. Content lives in
  `packages/web/src/content/docs/*.mdx` and is shipped as static HTML.
- **Content collections:** every page is an `mdx` file with frontmatter
  (`title`, `description`, etc.); Starlight derives the sidebar, table of
  contents, and previous/next links from that directory structure.
- **Built-in features used:** full-text search (Ctrl-K), multi-language
  (`Set language` menu with `lang` per page), light/dark/auto theme toggle,
  "Edit page" / "Found a bug?" deep links straight into the repo, per-page
  permalinks, and inline tab panels for code samples.
- **Everything lives next to the code:** docs are versioned with the code,
  contributions are ordinary PRs, and the "Edit page" link lowers the
  contribution barrier to a single commit.

**Takeaways for gtm:** a single-repo, content-collection docs site with
search, theme toggle, and "edit on GitHub" is the proven model. All of it is
achievable with Astro + Starlight and a static deploy.

---

## 2. Ratzilla feasibility check

**Ratzilla** (`ratatui/ratzilla`) is a Rust/WASM *application* framework: it
renders a real Ratatui TUI in the browser via `WebGl2Backend` /
`CanvasBackend` / `DomBackend`. It is **not** a docs content system — there is
no navigation, prose rendering, search, or SEO pipeline.

- It shines as an *interactive artifact*: a live, clickable gtm demo.
- It is the wrong tool for the actual manual (prose, indexable, accessible).

**Verdict:** build the site with Astro + Starlight; use Ratzilla only for one
optional "Try gtm in your browser" demo page (a wasm build of a headless gtm
TUI hitting a demo daemon). Gating it behind a feature toggle keeps the docs
site lightweight and evergreen.

---

## 3. Recommended stack

| Concern            | Choice                                  | Why |
|--------------------|-----------------------------------------|-----|
| Static site gen    | **Astro** (v5/v6)                       | Content collections, islands, tiny JS output |
| Docs theme         | **Starlight**                           | Sidebar, ToC, search, i18n, edit-links out of the box — same as opencode |
| CSS styling        | Custom theme layer for the terminal look | Blockquote "man page" callouts, fixed-width prose, ANSI-inspired accents |
| Markdown           | MDX (+ `` `md` for pure prose pages)     | Interactive examples (keybinding tables, TUI screenshots, terminal-style code) |
| Deployment         | GitHub Pages from `gh-pages` (or Pages Actions) | Free, static, no extra infra |
| Demo (optional)    | Ratzilla wasm page                      | Live TUI embed, feature-gated |

No runtime, no JS framework, no server: the output is pure HTML/CSS/JS that
any static host (and `pages.dev`/Cloudflare, if ever needed) can serve.

---

## 4. Content architecture (single source of truth)

Collapse the three current surfaces — GitHub **wiki**, **`gtm.1`** man page,
and partially `CONTRIBUTING.md` — into one tree in this repo and render it to
every static surface.

**Proposed layout in repo:** a top-level `docs/` Astro + Starlight site:

```
docs/
  astro.config.mjs          # Starlight config, sidebar, search, i18n
  src/content/config.ts     # content collection schema (frontmatter)
  src/content/docs/
    index.mdx               # ← migrated wiki/Home.md
    install.mdx
    quickstart.mdx
    tuiguide.mdx            # ← wiki/TUI-Guide.md (expanded)
    cli.mdx                 # ← wiki/CLI-Reference.md
    configuration.mdx       # ← wiki/Configuration.md
    ipc.mdx                 # ← wiki/IPC-Protocol.md (generated, see §6)
    architecture.mdx        # ← wiki/Architecture.md
    development.mdx         # ← wiki/Development.md
    manpage.mdx             # ← docs/man/gtm.1.md (rendered + copy)
    contributing.mdx        # ← CONTRIBUTING.md
```

**Ownership rules:**

- **Source of truth:** `docs/src/content/docs/`. Wiki and man page become
  *generated mirrors*, not hand-edited copies.
- `CONTRIBUTING.md` is kept at repo root (GitHub/editor surfacing) and mirrored
  as a docs page so the site is self-contained.
- A small sync script (`docs/sync-wiki.sh`, optional) regenerates the GH wiki
  from `docs/src/content/docs/` via `gh wiki` on release tags. This keeps the
  wiki alive for users who still land there *without* allowing it to drift.

---

## 5. Terminal "feel" — concrete design guidance

Starlight gives structure; the gtm identity comes from a theme layer:

- **Prose:** `ui-monospace`/`JetBrains Mono`-family stack for headings, code,
  and keybindings; generous line width inside each section; section dividers
  rendered as `───` rules.
- **Callouts / man-flavour:** Starlight "note"/tip components restyled as
  `▸ NOTE`, `⚠`, `✎` blocks with a `$`-prompt aesthetic; quote panels for
  prose blocks.
- **Colour:** use the default gtm theme ramp (green/amber accents on near-black;
  ANSI palette links) with both dark and light schemes — keep it readable.
- **Keybinding reference:** a reusable MDX component rendering the
  `footer`/keybind data as terminal-style tables with `[<key>]` columns.
- **Code:** shell sessions shown as real terminal transcripts
  (`$ gtm -c play ~/Music/song.flac`) with faint block cursors.
- **Screenshots / live demo:** half-block cover-art renders and an optional
  Ratzilla demo page give the "real TUI, in the browser" proof point.

---

## 6. IPC-Protocol page: generate, don't hand-author

`wiki/IPC-Protocol.md` describes the request/response wire format. The IPC
surface already has a canonical schema (`gtm-core::ipc`, `WireReq`/`WireRes`
serde models). Recommendation: a small doc-gen crate/bin (or a cargo alias)
emits the protocol section from `gtm-core` at build time so the manual never
drifts from the code. Ships in phase 2.

---

## 7. Navigation / information architecture

```
Install        > prerequisites, cargo install, release binaries, systemd user service
Quickstart     > first run, play a track, keybindings cheat sheet
TUI Guide      > Now Playing / Library / Settings, cover art backends, themes,
                 footer presets, lyrics, EQ
CLI Reference  > gtm flags + gtmd flags (mirrors gtm.1)
Configuration  > config.toml reference, per-key tables
Queue & MPRIS  > daemon features (replaces ad-hoc FAQ)
IPC Protocol   > generated wire spec
Architecture   > crate overview, data flow
Development    > build, test, bench, contributors
```

The sidebar mirrors `_Sidebar.md` today but gains search, pagination
(prev/next), and anchors for free.

---

## 8. CI / deployment

Add a `docs` workflow (draft):

1. **Trigger:** push to `main`/`dev`.
2. **Job:** `setup-node` + `npm ci` in `docs/` → `astro build`
   (`--base=/`), run `sync-wiki.sh` on release tags.
3. **Deploy:** `actions/deploy-pages`+ `upload-pages-artifact` from the
   `gh-pages` publish branch, or push the `dist/` folder to `gh-pages` — either
   works since output is static. Cache `npm` deps with `actions/cache`.
4. **Quality:** `astro check` (or `tsc`) fails the build on broken links/TS;
   a `--check` lint keeps the docs tree tidy like the Rust CI.

Only repo-owner-origin pushes deploy (the repo already has an *origin-only*
gate pattern: benchmark results are committed with the default
`GITHUB_TOKEN`, which never retriggers workflows — mirror that for docs).

---

## 9. i18n (later)

Starlight supports per-language content collections natively (`/zh-cn/...`).
Recommend deferring until the English tree is stable; the structure cost to add
it later is negligible.

---

## 10. Phased rollout

| Phase | Deliverable | Notes |
|-------|-------------|-------|
| **1** | Scaffold Astro + Starlight in `docs/`; migrate `Home`/`TUI Guide`/`Configuration`; CI deploy to GitHub Pages | Ships the primary surfaces on `gtmd.dev` |
| **2** | Generate `IPC-Protocol` from `gtm-core`; migrate `CLI Reference` + man page, `Architecture`, `Development`; `sync-wiki.sh` | Single source of truth complete |
| **3** | Terminal theme polish, keybinding component, optional Ratzilla demo page, i18n | Differentiation + "wow" factor |

Each phase is independently shippable; none blocks the others. Phase 1 is ~1-2
dev-days and immediately replaces the weakest current surface (the wiki has no
search, no versioning, and duplicates the man page).

---

## Appendix: source inventory (current)

| Current artifact          | Lines | Fate after migration |
|---------------------------|------|----------------------|
| `wiki/Home.md`            | 119   | → `docs/index.mdx` |
| `wiki/TUI-Guide.md`       | 99    | → `docs/tuiguide.mdx` |
| `wiki/CLI-Reference.md`   | 87    | → `docs/cli.mdx` (plus `gtm.1`) |
| `wiki/Configuration.md`   | 86    | → `docs/configuration.mdx` |
| `wiki/IPC-Protocol.md`    | 79    | → generated |
| `wiki/Architecture.md`    | 109   | → `docs/architecture.mdx` |
| `wiki/Development.md`     | 138   | → `docs/development.mdx` |
| `docs/man/gtm.1.md`       | 286   | curated in `docs/manpage.mdx` |
| `CONTRIBUTING.md`         | —     | mirrored as `docs/contributing.mdx` |
| `_Sidebar.md`/`_Footer.md`| 18    | → Starlight sidebar + site footer |