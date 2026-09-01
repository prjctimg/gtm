# Packaging for Omarchy

[Omarchy](https://omarchy.org/) is an Arch Linux-based distribution with its own
package repository: [`omacom/omarchy-pkgs`](https://github.com/omacom/omarchy-pkgs).
Packages are plain Arch `.pkg.tar.zst` produced from standard `PKGBUILD` files
with an `.omarchy/package.json` metadata file.

This page documents how gtm.rs would be listed there and what compliancy
requires. It reflects research against the `omarchy-pkgs` README and existing
Rust/TUI packages (e.g. `herdr`); re-verify against the current repo before
opening the submission PR, as the pipeline is forward-only (`edge → rc →
stable`) and review rules may change.

## Submission path

Fork `omacom/omarchy-pkgs`, add a package directory, and open a PR:

```
pkgbuilds/gtm/
├── PKGBUILD                  # standard Arch Rust PKGBUILD
└── .omarchy/
    └── package.json          # Omarchy metadata
```

## PKGBUILD (Rust, build from source)

Follow the [Arch Rust packaging guidelines](https://wiki.archlinux.org/title/Rust_package_guidelines):
`cargo fetch --locked` in `prepare()`, `cargo build --frozen --release` in
`build()`, list runtime shared-lib deps in `depends()`, install the license.

```bash
# Maintainer: prjctimg <prjctimg@outlook.com>
pkgname=gtm
pkgver=0.2.73
pkgrel=1
pkgdesc="Feature rich and cross platform terminal audio player with background playback and YouTube/Spotify integration"
arch=('x86_64' 'aarch64')
url="https://github.com/prjctimg/gtm.rs"
license=('GPL-3.0-only')
depends=('gcc-libs' 'glibc' 'alsa-lib')
makedepends=('cargo' 'pandoc' 'cmake')
options=('!lto')
source=("$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP') # set a real hash on first import; SKIP only for review

prepare() {
  cd "gtm.rs-$pkgver"
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "gtm.rs-$pkgver"
  export CARGO_TARGET_DIR=target
  cargo build --release --frozen --features pulseaudio

  mkdir -p artifacts
  ./scripts/build/manpages.sh artifacts
  cargo run --release --bin release-gen completions artifacts
}

package() {
  cd "gtm.rs-$pkgver"
  install -Dm 0755 target/release/gtm  "$pkgdir/usr/bin/gtm"
  install -Dm 0755 target/release/gtmd "$pkgdir/usr/bin/gtmd"
  install -Dm 0644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm 0644 artifacts/man/gtm.1 "$pkgdir/usr/share/man/man1/gtm.1"
  install -Dm 0644 artifacts/man/gtmd.1 "$pkgdir/usr/share/man/man1/gtmd.1"
  install -Dm 0644 artifacts/man/gtmd-ipc.1 "$pkgdir/usr/share/man/man1/gtmd-ipc.1"
  install -Dm 0644 dist/gtmd.service "$pkgdir/usr/lib/systemd/user/gtmd.service"
  install -Dm 0644 dist/gtm.desktop "$pkgdir/usr/share/applications/gtm.desktop"
  install -Dm 0644 assets/gtm.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/gtm.svg"
}
```

### Audio / streaming dependencies

gtm's daemon links ALSA/libasound for output and uses **InnerTube** directly for
YouTube (no `yt-dlp`) and **librespot** (via `rspotify`) for Spotify. Runtime
libs to declare in `depends=`:

- `alsa-lib` — audio output (PulseAudio backend is optional; on Arch the
  PulseAudio daemon is commonly present, but the lib is still a build-time link).
- `glibc`, `gcc-libs` — standard dynamic libs.

Verify actual linkage with `ldd target/release/gtmd` and carry only what is
truly referenced, to satisfy "no bundled C/C++ libs; use system packages".

## `.omarchy/package.json`

Minimal, maintain-the-PKGBUILD-ourselves form:

```json
{ "source": "local" }
```

With upstream version tracking from GitHub releases (recommended so Omarchy
builds track gtm releases):

```json
{
  "source": "local",
  "upstream": {
    "github": "prjctimg/gtm.rs",
    "checksums": "SHASUMS256.txt",
    "assets": {
      "x86_64": "gtm-debian-12-x86_64.tar.gz",
      "aarch64": "gtm-debian-12-aarch64.tar.gz"
    }
  },
  "release_ring": "fast"
}
```

Fields: `release_ring: "fast"` skips the edge→stable promotion delay;
`skip_build`/`rebuild_on`/`min_release_age` are optional extensions.
Match the asset names to whatever the release pipeline actually uploads.

## Compliancy checklist

- [ ] `arch=('x86_64' 'aarch64')` — both architectures required (aarch64 builds
      use QEMU emulation).
- [ ] Offline build — `cargo fetch --locked` in `prepare`, `--frozen` in `build`.
- [ ] `depends=` lists real runtime shared-libs only.
- [ ] License installed to `/usr/share/licenses/$pkgname/`.
- [ ] `.omarchy/package.json` present with correct `source`.
- [ ] No bundled C/C++ libs — prefer system packages over vendored `-sys` crates.
- [ ] `makepkg -s` builds clean locally on x86_64 before submitting.
- [ ] PR description explains the package and its runtime deps.

## Steps to submit

1. Confirm local `makepkg -s` succeeds on Arch/Omarchy (x86_64).
2. Fork `omacom/omarchy-pkgs`; add `pkgbuilds/gtm/PKGBUILD` +
   `pkgbuilds/gtm/.omarchy/package.json`.
3. Open a PR titled `add gtm` with the compliancy checklist filled in.
4. Address review; edge packages go through PR review, shared packages may
   auto-merge.

## "forward-only pipeline" note

Omarchy's repo is a forward-only pipeline (`edge → rc → stable`). Submissions
enter the `edge` ring first and are promoted; if immediate stable delivery is
wanted, set `"release_ring": "fast"`.