# Releasing

## Trigger

Push a version tag (`v*.*.*`), or run the workflow manually
(`workflow_dispatch`) with `release_type: stable|nightly`.

- `stable` — tag `v<version>` (e.g. `v0.1.4`); release version must match
  `version` in the workspace `Cargo.toml`.
- `nightly` — tag `nightly`, prerelease, always points at latest `main`.

The workflow force-points the tag at the build commit, so re-running a release
overwrites the previous one for that tag.

## What gets published

Every build job stages its outputs into a single `release-assets/` directory
and uploads one artifact; the release job merges them, prints the asset
manifest, writes `checksums.txt`, and attaches everything to the GitHub
Release. No bare binaries or standalone docs archives are published —
completions and man pages ship inside each per-target archive.

Per stable release `v0.1.4` (illustrative):

| Asset | Target | Built from |
|---|---|---|
| `gtm-x86_64-linux.tar.gz` | Linux x86_64 (glibc) | Debian 12 |
| `gtm_0.1.4-1_amd64.deb` | Debian/Ubuntu x86_64 | Debian 12 |
| `gtm-0.1.4-1.x86_64.rpm` | Fedora/RHEL x86_64 | Debian 12 |
| `gtm-0.1.4-1-x86_64.pkg.tar.zst` | Arch x86_64 | Debian 12 |
| `gtm-aarch64-linux.tar.gz` | Linux ARM64 (glibc) | Debian 12 |
| `gtm_0.1.4-1_arm64.deb` | Debian/Ubuntu ARM64 | Debian 12 |
| `gtm-0.1.4-1.aarch64.rpm` | Fedora/RHEL ARM64 | Debian 12 |
| `gtm-0.1.4-1-aarch64.pkg.tar.zst` | Arch Linux ARM | Debian 12 |
| `gtm-aarch64-darwin.tar.gz` | macOS ARM64 | macOS runner |
| `gtm-aarch64-android.tar.gz` | Android / Termux ARM64 | Ubuntu + NDK |
| `gtm_0.1.4_aarch64.deb` | Termux ARM64 | Ubuntu + NDK |
| `gtm-x86_64-linux-musl.tar.gz` | Linux x86_64 (musl) | Alpine 3.22 |
| `gtm-0.1.4-r0-x86_64.apk` | Alpine x86_64 | Alpine 3.22 |
| `gtm-aarch64-linux-musl.tar.gz` | Linux ARM64 (musl) | Alpine 3.22 |
| `gtm-0.1.4-r0-aarch64.apk` | Alpine ARM64 | Alpine 3.22 |
| `checksums.txt` | all targets | release job (SHA-256) |

## Archive layout

`gtm-{platform}.tar.gz` is a flat, self-contained bundle:

```
gtm-{platform}/
├── bin/          gtm, gtmd
├── man/man1/     gtm.1, gtmd.1, gtmd-ipc.1
├── completions/  bash, zsh (_*), fish, elv, ps1 for gtm + gtmd
├── systemd/      gtmd.service (user unit)
├── desktop/      gtm.desktop
├── icons/        gtm.svg
└── LICENSE
```

> The shared [gtm.spec install.sh](https://github.com/prjctimg/gtm.spec)
> consumes these archives; keep its expected layout in sync when changing this
> structure.

## Package notes

- **.deb** — `cargo-deb`; deps resolved via `dpkg-shlibdeps` on the build host.
  Both Linux glibc targets build inside the Debian 12 container, so the amd64
  and arm64 debs target glibc 2.36 (Debian 12+).
- **.rpm** — binary packaging via `dist/gtm.spec` (no source rebuild);
  runtime dep is `alsa-lib`.
- **.pkg.tar.zst** — Arch package built from the staged tree via
  `scripts/build-arch-pkg.sh` (`.PKGINFO` + `.MTREE`).
- **.apk** — unsigned Alpine package via `scripts/build-alpine-apk.sh`;
  install with `apk add --allow-untrusted`.

## Checklist

1. Bump `version` in the workspace `Cargo.toml` and add a `CHANGELOG.md` entry.
2. Push and tag: `git tag v0.1.4 && git push origin v0.1.4`.
3. Watch `.github/workflows/release.yml`; confirm every build job and the
   release job go green.
4. Verify the release page lists only the expected assets (see table above) and
   `checksums.txt` passes `sha256sum -c`.
