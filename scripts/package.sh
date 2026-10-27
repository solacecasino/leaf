#!/bin/bash
# Makes this system's downloads in dist/:
#   macOS:   Leaf-<v>-macos-universal.zip, -macos-arm64.zip, -macos-x64.zip
#   Windows: Leaf-<v>-windows-<x64|arm64>.zip, with pdfium.dll
#   Linux:   leaf-<v>-linux-<x64|arm64>.tar.gz, with libpdfium.so
# PDFIUM_DIR is an unpacked pdfium-binaries release (Windows and Linux,
# where Leaf draws PDFs with it). Usage: scripts/package.sh
set -e
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mkdir -p dist

# What cargo builds for, not what the shell says: on Arm Windows, Git Bash
# is an x64 program and reports x64.
arch() {
  case "$(rustc -vV | sed -n 's/^host: //p')" in
    aarch64*) echo arm64 ;;
    *) echo x64 ;;
  esac
}

case "$(uname -s)" in
  Darwin)
    UNIVERSAL=1 scripts/bundle.sh
    rm -f dist/Leaf-"$VERSION"-macos-*.zip
    # ditto keeps the app's signature, permissions and icon intact.
    ditto -c -k --sequesterRsrc --keepParent target/Leaf.app "dist/Leaf-$VERSION-macos-universal.zip"
    # Smaller downloads for one kind of Mac each.
    for a in arm64:arm64 x86_64:x64; do
      thin=target/thin-${a#*:}
      rm -rf "$thin" && mkdir -p "$thin"
      cp -R target/Leaf.app "$thin/"
      lipo "$thin/Leaf.app/Contents/MacOS/leaf" -thin "${a%%:*}" -output "$thin/leaf"
      mv "$thin/leaf" "$thin/Leaf.app/Contents/MacOS/leaf"
      codesign --force --deep --sign - "$thin/Leaf.app"
      ditto -c -k --sequesterRsrc --keepParent "$thin/Leaf.app" "dist/Leaf-$VERSION-macos-${a#*:}.zip"
    done
    ;;
  Linux)
    : "${PDFIUM_DIR:?set PDFIUM_DIR to an unpacked pdfium-binaries release}"
    cargo build --release
    d=leaf-$VERSION-linux-$(arch)
    rm -rf "target/stage" && mkdir -p "target/stage/$d"
    cp target/release/leaf README.md LICENSE "target/stage/$d/"
    # The app menu entry and icon, added by ./install.sh.
    cp packaging/linux/install.sh packaging/linux/leaf.desktop "target/stage/$d/"
    cp assets/icon-256.png "target/stage/$d/leaf.png"
    cp "$PDFIUM_DIR/lib/libpdfium.so" "target/stage/$d/"
    cp "$PDFIUM_DIR/LICENSE" "target/stage/$d/LICENSE-pdfium.txt"
    tar -czf "dist/$d.tar.gz" -C target/stage "$d"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    : "${PDFIUM_DIR:?set PDFIUM_DIR to an unpacked pdfium-binaries release}"
    cargo build --release
    rm -rf target/stage && mkdir -p target/stage
    cp target/release/leaf.exe target/stage/Leaf.exe
    cp README.md LICENSE target/stage/
    cp "$PDFIUM_DIR/bin/pdfium.dll" target/stage/
    cp "$PDFIUM_DIR/LICENSE" target/stage/LICENSE-pdfium.txt
    out="dist/Leaf-$VERSION-windows-$(arch).zip"
    rm -f "$out"
    (cd target/stage && python -m zipfile -c "../../$out" *)
    ;;
esac
ls -la dist
