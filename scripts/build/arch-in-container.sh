#!/usr/bin/env bash
set -euo pipefail

platform="${1:?platform required}"
version="${2:?version required}"
arch="${3:?arch required}"

cd /work

export RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=mold"

pacman -Syu --noconfirm
pacman -S --noconfirm --needed \
  base-devel rust pandoc alsa-lib cmake clang lld mold git \
  libarchive zstd

cargo build --release

./scripts/build/manpages.sh artifacts
cargo run --release --bin release-gen completions artifacts

root="gtm-arch-${arch}"
mkdir -p release-assets \
  "$root/bin" "$root/man/man1" "$root/completions" \
  "$root/systemd" "$root/desktop" "$root/icons"
cp target/release/gtm "$root/bin/"
cp target/release/gtmd "$root/bin/"
cp artifacts/man/*.1 "$root/man/man1/"
cp artifacts/completions/* "$root/completions/"
cp dist/gtmd.service "$root/systemd/"
cp dist/gtm.desktop "$root/desktop/"
cp assets/gtm.svg "$root/icons/"
cp LICENSE "$root/"
cp install.sh "$root/"
tar czf "release-assets/gtm-arch-${arch}.tar.gz" "$root"

mkdir -p stage/usr/bin \
  stage/usr/lib/systemd/user \
  stage/usr/share/man/man1 \
  stage/usr/share/bash-completion/completions \
  stage/usr/share/zsh/site-functions \
  stage/usr/share/fish/vendor_completions.d \
  stage/usr/share/elvish/lib \
  stage/usr/share/powershell/Modules \
  stage/usr/share/applications \
  stage/usr/share/icons/hicolor/scalable/apps \
  stage/usr/share/licenses/gtm
cp target/release/gtm stage/usr/bin/
cp target/release/gtmd stage/usr/bin/
cp dist/gtmd.service stage/usr/lib/systemd/user/
cp artifacts/man/*.1 stage/usr/share/man/man1/
cp artifacts/completions/gtm.bash stage/usr/share/bash-completion/completions/gtm
cp artifacts/completions/gtmd.bash stage/usr/share/bash-completion/completions/gtmd
cp artifacts/completions/_gtm stage/usr/share/zsh/site-functions/_gtm
cp artifacts/completions/_gtmd stage/usr/share/zsh/site-functions/_gtmd
cp artifacts/completions/gtm.fish stage/usr/share/fish/vendor_completions.d/
cp artifacts/completions/gtmd.fish stage/usr/share/fish/vendor_completions.d/
cp artifacts/completions/gtm.elv stage/usr/share/elvish/lib/
cp artifacts/completions/gtmd.elv stage/usr/share/elvish/lib/
cp artifacts/completions/gtm.ps1 stage/usr/share/powershell/Modules/
cp artifacts/completions/gtmd.ps1 stage/usr/share/powershell/Modules/
cp dist/gtm.desktop stage/usr/share/applications/
cp assets/gtm.svg stage/usr/share/icons/hicolor/scalable/apps/
cp LICENSE stage/usr/share/licenses/gtm/

bash /work/scripts/build/arch-pkg.sh "$version" "$arch" stage \
  "release-assets/gtm-${version}-${arch}.pkg.tar.zst"

echo "arch build complete: $(ls -1 release-assets)"
