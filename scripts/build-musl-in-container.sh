#!/usr/bin/env bash
# Build the Alpine (musl) release artifacts inside an alpine container.
#
# Runs with the workspace mounted at /work (CWD=/work). Produces:
#   release-assets/gtm-full-{platform}.tar.gz
#   release-assets/gtm-{version}-r0-{arch}.apk
#
# Usage (from the runner host):
#   docker run --rm -v "$PWD":/work -w /work -e HOME=/root alpine:3.22 \
#     /bin/sh -c "apk add --no-cache bash && /bin/bash /work/scripts/build-musl-in-container.sh <platform> <version> <arch>"
set -euo pipefail

platform="${1:?platform required}"
version="${2:?version required}"
arch="${3:?arch required}"

cd /work

export RUSTFLAGS="-C target-feature=-crt-static"

apk add --no-cache \
  ca-certificates curl build-base cmake musl-dev \
  pkgconfig alsa-lib-dev pandoc

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
export PATH="$HOME/.cargo/bin:$PATH"

cargo build --release

./scripts/gen-manpages.sh artifacts
cargo run --release --bin release-gen completions artifacts

# ── Complete per-target archive ──
root="gtm-full-${platform}"
mkdir -p release-assets \
  "$root/bin" "$root/man/man1" "$root/completions" \
  "$root/systemd" "$root/desktop" "$root/icons"
cp target/release/gtm  "$root/bin/"
cp target/release/gtmd "$root/bin/"
cp artifacts/man/*.1        "$root/man/man1/"
cp artifacts/completions/*  "$root/completions/"
cp dist/gtmd.service  "$root/systemd/"
cp dist/gtm.desktop   "$root/desktop/"
cp assets/gtm.svg     "$root/icons/"
cp LICENSE            "$root/"
tar czf "release-assets/gtm-full-${platform}.tar.gz" "$root"

# ── Alpine .apk from a rootfs-style stage tree ──
mkdir -p stage/usr/bin \
  stage/usr/lib/systemd/user \
  stage/usr/share/man/man1 \
  stage/usr/share/bash-completion/completions \
  stage/usr/share/zsh/vendor-completions \
  stage/usr/share/fish/vendor_completions.d \
  stage/usr/share/elvish/completions \
  stage/usr/share/powershell/completions \
  stage/usr/share/applications \
  stage/usr/share/icons/hicolor/scalable/apps \
  stage/usr/share/licenses/gtm-full
cp target/release/gtm  stage/usr/bin/
cp target/release/gtmd stage/usr/bin/
cp dist/gtmd.service stage/usr/lib/systemd/user/
cp artifacts/man/*.1 stage/usr/share/man/man1/
cp artifacts/completions/gtm.bash  stage/usr/share/bash-completion/completions/gtm
cp artifacts/completions/gtmd.bash stage/usr/share/bash-completion/completions/gtmd
cp artifacts/completions/_gtm  stage/usr/share/zsh/vendor-completions/_gtm
cp artifacts/completions/_gtmd stage/usr/share/zsh/vendor-completions/_gtmd
cp artifacts/completions/gtm.fish  stage/usr/share/fish/vendor_completions.d/
cp artifacts/completions/gtmd.fish stage/usr/share/fish/vendor_completions.d/
cp artifacts/completions/gtm.elv  stage/usr/share/elvish/completions/
cp artifacts/completions/gtmd.elv stage/usr/share/elvish/completions/
cp artifacts/completions/gtm.ps1  stage/usr/share/powershell/completions/
cp artifacts/completions/gtmd.ps1 stage/usr/share/powershell/completions/
cp dist/gtm.desktop stage/usr/share/applications/
cp assets/gtm.svg stage/usr/share/icons/hicolor/scalable/apps/
cp LICENSE stage/usr/share/licenses/gtm-full/

bash /work/scripts/build-alpine-apk.sh "$version" "$arch" stage \
  "release-assets/gtm-${version}-r0-${arch}.apk"

echo "musl build complete: $(ls -1 release-assets)"
