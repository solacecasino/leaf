#!/bin/bash
# Makes this system's downloads in dist/:
#   macOS:   leaf9000-<v>-macos-universal.zip, -macos-arm64.zip, -macos-x64.zip
#   Windows: leaf9000-<v>-windows-<x64|arm64>.zip, with pdfium.dll
#   Linux:   leaf9000-<v>-linux-<x64|arm64>.tar.gz, with libpdfium.so
# PDFIUM_DIR is an unpacked pdfium-binaries release (Windows and Linux,
# where leaf9000 draws PDFs with it). Usage: scripts/package.sh
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
    rm -f dist/leaf9000-"$VERSION"-macos-*.zip
    # ditto keeps the app's signature, permissions and icon intact.
    ditto -c -k --sequesterRsrc --keepParent target/leaf9000.app "dist/leaf9000-$VERSION-macos-universal.zip"
    # Smaller downloads for one kind of Mac each.
    for a in arm64:arm64 x86_64:x64; do
      thin=target/thin-${a#*:}
      rm -rf "$thin" && mkdir -p "$thin"
      cp -R target/leaf9000.app "$thin/"
      lipo "$thin/leaf9000.app/Contents/MacOS/leaf9000" -thin "${a%%:*}" -output "$thin/leaf9000"
      mv "$thin/leaf9000" "$thin/leaf9000.app/Contents/MacOS/leaf9000"
      codesign --force --deep --sign - "$thin/leaf9000.app"
      ditto -c -k --sequesterRsrc --keepParent "$thin/leaf9000.app" "dist/leaf9000-$VERSION-macos-${a#*:}.zip"
    done
    ;;
  Linux)
    : "${PDFIUM_DIR:?set PDFIUM_DIR to an unpacked pdfium-binaries release}"
    cargo build --release
    d=leaf9000-$VERSION-linux-$(arch)
    rm -rf "target/stage" && mkdir -p "target/stage/$d"
    cp target/release/leaf9000 README.md LICENSE "target/stage/$d/"
    # The app menu entry and icon, added by ./install.sh.
    cp packaging/linux/install.sh packaging/linux/leaf9000.desktop "target/stage/$d/"
    cp assets/icon-256.png "target/stage/$d/leaf9000.png"
    cp "$PDFIUM_DIR/lib/libpdfium.so" "target/stage/$d/"
    cp "$PDFIUM_DIR/LICENSE" "target/stage/$d/LICENSE-pdfium.txt"
    tar -czf "dist/$d.tar.gz" -C target/stage "$d"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    : "${PDFIUM_DIR:?set PDFIUM_DIR to an unpacked pdfium-binaries release}"
    cargo build --release
    rm -rf target/stage && mkdir -p target/stage
    cp target/release/leaf9000.exe target/stage/leaf9000.exe
    cp README.md LICENSE target/stage/
    cp "$PDFIUM_DIR/bin/pdfium.dll" target/stage/
    cp "$PDFIUM_DIR/LICENSE" target/stage/LICENSE-pdfium.txt
    cp packaging/windows/install.cmd packaging/windows/install.ps1 packaging/windows/leaf9000.ico target/stage/
    out="dist/leaf9000-$VERSION-windows-$(arch).zip"
    rm -f "$out"
    (cd target/stage && python -m zipfile -c "../../$out" *)
    ;;
esac
ls -la dist
