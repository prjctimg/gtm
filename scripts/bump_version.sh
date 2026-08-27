#!/usr/bin/env bash
# Version bump script for gtm.rs
# Usage: ./scripts/bump_version.sh <new_version>
# Example: ./scripts/bump_version.sh 0.2.6

set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <new_version>"
    echo "Example: $0 0.2.6"
    exit 1
fi

NEW_VERSION="$1"

# Validate version format (semver)
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?(\+[a-zA-Z0-9.-]+)?$ ]]; then
    echo "Error: Invalid version format. Use semantic versioning (e.g., 0.2.6, 1.0.0-beta.1)"
    exit 1
fi

WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TOML="$WORKSPACE_ROOT/Cargo.toml"
CHANGELOG="$WORKSPACE_ROOT/CHANGELOG.md"

if [ ! -f "$CARGO_TOML" ]; then
    echo "Error: Cargo.toml not found at $CARGO_TOML"
    exit 1
fi

# Get current version from Cargo.toml
CURRENT_VERSION=$(grep '^version = ' "$CARGO_TOML" | head -1 | sed 's/version = "\(.*\)"/\1/')
echo "Current version: $CURRENT_VERSION"
echo "New version: $NEW_VERSION"

# Update Cargo.toml workspace version
sed -i "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
echo "Updated $CARGO_TOML"

# Update Makefile rpm target version
sed -i "s/gtmd-$CURRENT_VERSION/gtmd-$NEW_VERSION/g" "$WORKSPACE_ROOT/Makefile"
echo "Updated Makefile"

# Update CHANGELOG.md - add new version entry at the top (after header)
if [ -f "$CHANGELOG" ]; then
    # Create temporary file with new version entry
    TEMP_CHANGELOG=$(mktemp)
    {
        # Print header lines until first version entry
        awk '/^## \[/ {exit} {print}' "$CHANGELOG"
        # Add new version entry
        echo "## [$NEW_VERSION] - $(date +%Y-%m-%d)"
        echo ""
        echo "### Added"
        echo ""
        echo "### Changed"
        echo ""
        echo "### Fixed"
        echo ""
        echo "### Removed"
        echo ""
        # Print rest of changelog
        awk 'NR>1 && /^## \[/ {found=1} found {print}' "$CHANGELOG"
    } > "$TEMP_CHANGELOG"
    mv "$TEMP_CHANGELOG" "$CHANGELOG"
    echo "Updated $CHANGELOG"
fi

# Show git diff
cd "$WORKSPACE_ROOT"
git diff --stat

echo ""
echo "Version bumped to $NEW_VERSION"
echo "Next steps:"
echo "  1. Review changes: git diff"
echo "  2. Commit: git commit -am \"chore: bump version to $NEW_VERSION\""
echo "  3. Tag: git tag v$NEW_VERSION"
echo "  4. Push: git push && git push --tags"