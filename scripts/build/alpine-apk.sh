#!/usr/bin/env bash
set -euo pipefail

# Build an unsigned Alpine .apk from a rootfs-style stage tree.
version="${1:?version required}"
arch="${2:?arch required}"
stage="${3:?stage dir required}"
out="${4:?output file required}"

name="gtm"
pkgver="${version}-r0"

[ -d "$stage" ] || {
  echo "error: stage dir not found: $stage" >&2
  exit 1
}

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
root="$tmpdir/root"
mkdir -p "$root"
cp -a "$stage"/. "$root"/

size="$(du -sk "$root" | cut -f1)"

cat >"$root/.PKGINFO" <<EOF
pkgname = $name
pkgver = $pkgver
pkgdesc = Terminal music player (TUI + CLI) and background daemon.
url = https://github.com/prjctimg/gtm.rs
builddate = $(date +%s)
packager = prjctimg <prjctimg@outlook.com>
size = $size
arch = $arch
origin = $name
license = GPL-3.0-only
depend = alsa-lib
EOF

tar -czf "$out" --numeric-owner -C "$root" .
echo "built $out ($(du -h "$out" | cut -f1))"
