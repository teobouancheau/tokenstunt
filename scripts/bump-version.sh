#!/usr/bin/env bash
set -euo pipefail

# Bumps the workspace version in Cargo.toml from a git tag (e.g. v1.2.3).
# Usage: ./scripts/bump-version.sh v1.2.3

TAG="${1:?Usage: bump-version.sh <tag>}"
VERSION="${TAG#v}"

if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "error: invalid semver: $VERSION"
    exit 1
fi

sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
rm -f Cargo.toml.bak

cargo generate-lockfile 2>/dev/null || true

echo "bumped to $VERSION"
