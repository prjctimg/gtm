#!/usr/bin/env bash
set -euo pipefail

# Build an Arch Linux .pkg.tar.zst from a rootfs-style stage tree.
version="${1:?version required}"
arch="${2:?arch required}"
stage="${3:?stage dir required}"
out="${4:?output file required}"

name="gtm"
pkgver="${version}-1"

[ -d "$stage" ] || {
  echo "error: stage dir not found: $stage" >&2
  exit 1
}
command -v bsdtar >/dev/null 2>&1 || {
  echo "error: bsdtar (libarchive) not found" >&2
  exit 1
}
command -v zstd >/dev/null 2>&1 || {
  echo "error: zstd not found" >&2
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
license = GPL3
depend = glibc
depend = alsa-lib
EOF

bsdtar -cf "$tmpdir/.MTREE" \
  --format=mtree \
  --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' \
  --uid 0 --gid 0 \
  -C "$root" .
cp "$tmpdir/.MTREE" "$root/.MTREE"

tar --zstd -cf "$out" --owner=root --group=root --numeric-owner -C "$root" .
echo "built $out ($(du -h "$out" | cut -f1))"
