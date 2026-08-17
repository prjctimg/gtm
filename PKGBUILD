# Maintainer: prjctimg <prjctimg@outlook.com>
pkgname=gtm
pkgver=0.2.1
pkgrel=1
pkgdesc="Terminal-based music player daemon and client"
arch=('x86_64' 'aarch64')
url="https://github.com/prjctimg/gtm.rs"
license=('GPL-3.0-only')
depends=('alsa-lib' 'glibc')
makedepends=('cargo' 'pandoc')
source=("$url/archive/refs/tags/v${pkgver}.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "gtm.rs-${pkgver}"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "gtm.rs-${pkgver}"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  cargo build --release --frozen

  mkdir -p artifacts
  ./scripts/build/manpages.sh artifacts
  cargo run --release --bin release-gen completions artifacts
}

package() {
  cd "gtm.rs-${pkgver}"

  install -Dm 0755 target/release/gtm  "$pkgdir/usr/bin/gtm"
  install -Dm 0755 target/release/gtmd "$pkgdir/usr/bin/gtmd"

  install -Dm 0644 artifacts/man/gtm.1      "$pkgdir/usr/share/man/man1/gtm.1"
  install -Dm 0644 artifacts/man/gtmd.1     "$pkgdir/usr/share/man/man1/gtmd.1"
  install -Dm 0644 artifacts/man/gtmd-ipc.1 "$pkgdir/usr/share/man/man1/gtmd-ipc.1"

  install -Dm 0644 dist/gtmd.service "$pkgdir/usr/lib/systemd/user/gtmd.service"
  install -Dm 0644 dist/gtm.desktop "$pkgdir/usr/share/applications/gtm.desktop"
  install -Dm 0644 assets/gtm.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/gtm.svg"

  install -Dm 0644 artifacts/completions/gtm.bash  "$pkgdir/usr/share/bash-completion/completions/gtm"
  install -Dm 0644 artifacts/completions/_gtm      "$pkgdir/usr/share/zsh/site-functions/_gtm"
  install -Dm 0644 artifacts/completions/gtm.fish  "$pkgdir/usr/share/fish/vendor_completions.d/gtm.fish"
  install -Dm 0644 artifacts/completions/gtmd.bash "$pkgdir/usr/share/bash-completion/completions/gtmd"
  install -Dm 0644 artifacts/completions/_gtmd     "$pkgdir/usr/share/zsh/site-functions/_gtmd"
  install -Dm 0644 artifacts/completions/gtmd.fish "$pkgdir/usr/share/fish/vendor_completions.d/gtmd.fish"
}
