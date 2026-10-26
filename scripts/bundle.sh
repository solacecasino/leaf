#!/bin/sh
# Builds a release Leaf.app in target/ with the icon, name, version and the
# file types Finder can open with it.
# Usage: scripts/bundle.sh             (this Mac's kind of processor)
#        UNIVERSAL=1 scripts/bundle.sh  one app for both Apple Silicon and Intel Macs
set -e
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/Leaf.app

if [ "$UNIVERSAL" = 1 ]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/release
  lipo -create -output target/release/leaf \
    target/aarch64-apple-darwin/release/leaf target/x86_64-apple-darwin/release/leaf
else
  cargo build --release
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/leaf "$APP/Contents/MacOS/leaf"

# Turn the 1024px PNG into an .icns with every size macOS asks for.
ICONSET=target/Leaf.iconset
rm -rf "$ICONSET" && mkdir "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Leaf.icns"
rm -rf "$ICONSET"

# Comic archives are Leaf's own; zips, EPUBs, images and folders only offer
# Leaf as another app to open them with, so nothing else's defaults change.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Leaf</string>
  <key>CFBundleDisplayName</key><string>Leaf</string>
  <key>CFBundleIdentifier</key><string>app.leaf.reader</string>
  <key>CFBundleExecutable</key><string>leaf</string>
  <key>CFBundleIconFile</key><string>Leaf</string>
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
      <key>CFBundleTypeIconFile</key><string>Leaf</string>
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
