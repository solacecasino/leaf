#!/bin/sh
# Builds a release leaf9000.app in target/ with the icon, name, version and the
# file types Finder can open with it.
# Usage: scripts/bundle.sh             (this Mac's kind of processor)
#        UNIVERSAL=1 scripts/bundle.sh  one app for both Apple Silicon and Intel Macs
set -e
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/leaf9000.app

if [ "$UNIVERSAL" = 1 ]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/release
  lipo -create -output target/release/leaf9000 \
    target/aarch64-apple-darwin/release/leaf9000 target/x86_64-apple-darwin/release/leaf9000
else
  cargo build --release
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Leaf.app from before the rename, so the two never get mixed up.
rm -rf target/Leaf.app
cp target/release/leaf9000 "$APP/Contents/MacOS/leaf9000"

# Turn the 1024px PNG into an .icns with every size macOS asks for.
ICONSET=target/leaf9000.iconset
rm -rf "$ICONSET" && mkdir "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/leaf9000.icns"
rm -rf "$ICONSET"

# Comic archives are leaf9000's own; zips, EPUBs, images and folders only
# offer it as another app to open them with, so nothing else's defaults change.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>leaf9000</string>
  <key>CFBundleDisplayName</key><string>leaf9000</string>
  <key>CFBundleIdentifier</key><string>app.leaf9000.reader</string>
  <key>CFBundleExecutable</key><string>leaf9000</string>
  <key>CFBundleIconFile</key><string>leaf9000</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.books</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>© 2026 Vishal Mohan · MIT License</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Comic Book Archive</string>
      <key>CFBundleTypeExtensions</key><array><string>cbz</string><string>cbr</string><string>cb7</string></array>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Default</string>
      <key>CFBundleTypeIconFile</key><string>leaf9000</string>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Book or Image</string>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>zip</string><string>epub</string><string>pdf</string>
        <string>jpg</string><string>jpeg</string><string>png</string><string>webp</string><string>gif</string><string>bmp</string>
      </array>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Folder of Pages</string>
      <key>LSItemContentTypes</key><array><string>public.folder</string></array>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Ad-hoc signature: not a developer certificate, but Apple Silicon needs it to run.
codesign --force --deep --sign - "$APP"

echo "Built $APP (version $VERSION)"
