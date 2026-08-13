#!/usr/bin/env bash
# Build Termux .deb package for gtm-rs.
# Requires: termux-create-package, cross-compiled binaries in target/aarch64-linux-android/release/
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:-$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | sed 's/^version = "\(.*\)"/\1/')}"
ARCH="aarch64"

BIN_DIR="$REPO_ROOT/target/aarch64-linux-android/release"
FILES_DIR="$REPO_ROOT/target/aarch64-linux-android/termux-pkg/files"

# Verify binaries exist
for bin in gtmd gtm; do
    if [ ! -f "$BIN_DIR/$bin" ]; then
        echo "Error: $BIN_DIR/$bin not found."
        echo "Run 'make termux' first."
        exit 1
    fi
done

# Assemble package tree
rm -rf "$FILES_DIR"
mkdir -p "$FILES_DIR/bin" "$FILES_DIR/share/man/man1"

cp "$BIN_DIR/gtmd" "$FILES_DIR/bin/"
cp "$BIN_DIR/gtm"  "$FILES_DIR/bin/"

# Copy man pages if they exist
if [ -d "$REPO_ROOT/artifacts/man" ]; then
    cp "$REPO_ROOT"/artifacts/man/gtm.1 "$REPO_ROOT"/artifacts/man/gtmd.1 "$REPO_ROOT"/artifacts/man/gtmd-ipc.1 \
        "$FILES_DIR/share/man/man1/" 2>/dev/null || true
fi

# Create the .deb
termux-create-package \
    --pkg-version "$VERSION" \
    --pkg-arch "$ARCH" \
    --files-dir "$FILES_DIR" \
    --deb-dir "$REPO_ROOT" \
    "$REPO_ROOT/termux/gtm.yml"

echo "Built: $(ls "$REPO_ROOT"/*.deb 2>/dev/null | head -1)"
