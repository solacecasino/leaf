#!/bin/bash
# Builds an adapter from adapters/<name> and packages it for this system as
# dist/<name>-<version>-<platform>.zip, ready to drop on Leaf's window (on
# macOS one universal zip for both kinds of Mac).
# Usage: scripts/package-adapter.sh upscale-4k
set -e
cd "$(dirname "$0")/.."
name=${1:?which adapter}
src=adapters/$name
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$src/Cargo.toml" | head -1)
stage=target/adapter-stage/$name
rm -rf "$stage" && mkdir -p "$stage" dist
cp "$src/adapter.toml" "$src/README.md" "$src/LICENSE" "$stage/"
case "$(uname -s)" in
  Darwin)
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
      rustup target add "$t" >/dev/null
      cargo build --release --manifest-path "$src/Cargo.toml" --target "$t"
    done
    lipo -create -output "$stage/$name" "$src/target/aarch64-apple-darwin/release/$name" "$src/target/x86_64-apple-darwin/release/$name"
    codesign --force --sign - "$stage/$name"
    platform=macos-universal
    ;;
  MINGW*|MSYS*|CYGWIN*)
    cargo build --release --manifest-path "$src/Cargo.toml"
    cp "$src/target/release/$name.exe" "$stage/"
    # Windows programs end in .exe.
    sed -i "s|^run = \[\"./$name\"\]|run = [\"./$name.exe\"]|" "$stage/adapter.toml"
    case "$(rustc -vV | sed -n 's/^host: //p')" in aarch64*) platform=windows-arm64 ;; *) platform=windows-x64 ;; esac  # not the shell's: Git Bash is x64 on Arm
    ;;
  *)
    cargo build --release --manifest-path "$src/Cargo.toml"
    cp "$src/target/release/$name" "$stage/"
    case "$(rustc -vV | sed -n 's/^host: //p')" in aarch64*) platform=linux-arm64 ;; *) platform=linux-x64 ;; esac
    ;;
esac
out="dist/$name-$VERSION-$platform.zip"
rm -f "$out"
(cd target/adapter-stage && python3 -m zipfile -c "../../$out" "$name" 2>/dev/null || python -m zipfile -c "../../$out" "$name")
echo "Packaged $out"
