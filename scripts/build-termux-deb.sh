#!/usr/bin/env bash
# Build Termux .deb package for gtm-rs.
# Requires: termux-create-package, cross-compiled binaries in target/aarch64-linux-android/release/
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TERMUX_PKG_DIR="$REPO_ROOT/termux"
BUILD_DIR="$REPO_ROOT/target/aarch64-linux-android/termux-pkg"

VERSION="${1:-$(git -C "$REPO_ROOT" describe --tags --abbrev=0 2>/dev/null | sed 's/^v//' || echo "0.1.0")}"
ARCH="aarch64"

# Verify binaries exist
for bin in gtmd gtm; do
    if [ ! -f "$REPO_ROOT/target/aarch64-linux-android/release/$bin" ]; then
        echo "Error: target/aarch64-linux-android/release/$bin not found."
        echo "Run 'make termux' first."
        exit 1
    fi
done

# Assemble package tree
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/bin" "$BUILD_DIR/share/man/man1"

cp "$REPO_ROOT/target/aarch64-linux-android/release/gtmd" "$BUILD_DIR/bin/"
cp "$REPO_ROOT/target/aarch64-linux-android/release/gtm"  "$BUILD_DIR/bin/"

# Copy man pages if they exist
if [ -d "$REPO_ROOT/artifacts/man" ]; then
    cp "$REPO_ROOT/artifacts/man/"*.1 "$BUILD_DIR/share/man/man1/" 2>/dev/null || true
fi

# Create the .deb
cd "$BUILD_DIR"
termux-create-package \
    --architecture "$ARCH" \
    --version "$VERSION" \
    "$TERMUX_PKG_DIR"

# Move .deb to repo root
mv ./*.deb "$REPO_ROOT/"
echo "Built: $(ls "$REPO_ROOT"/*.deb 2>/dev/null | head -1)"
