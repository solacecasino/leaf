#!/bin/sh
# Makes the downloadable zip: dist/Leaf-<version>-macos-universal.zip,
# holding Leaf.app for both Apple Silicon and Intel Macs.
# Usage: scripts/package.sh
set -e
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
UNIVERSAL=1 scripts/bundle.sh

mkdir -p dist
ZIP="dist/Leaf-$VERSION-macos-universal.zip"
rm -f "$ZIP"
# ditto keeps the app's signature, permissions and icon intact.
ditto -c -k --sequesterRsrc --keepParent target/Leaf.app "$ZIP"
echo "Packaged $ZIP ($(du -h "$ZIP" | cut -f1 | tr -d ' '))"
