#!/bin/sh
# Builds "GitHub PRs.app" and installs it in /Applications, so Spotlight,
# the Dock and app launchers can open it.
#
# Usage: scripts/bundle-macos.sh [install-dir]   (default: /Applications)
#
# For release builds: BIN=path/to/github-prs scripts/bundle-macos.sh --no-install
# wraps an already built binary (say, a universal one) and leaves the .app in
# target/release without installing it.
set -eu

cd "$(dirname "$0")/.."
CARGO="${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"
DEST="${1:-/Applications}"
NAME="GitHub PRs"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
APP="target/release/$NAME.app"

if [ -z "${BIN:-}" ]; then
  "$CARGO" build --release
  BIN="target/release/github-prs"
fi
"$CARGO" run --release --quiet --example make_icon -- target/release/icon.png

# App icon: macOS wants one file holding several sizes.
ICONSET="target/release/AppIcon.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s target/release/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) target/release/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/github-prs"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>dev.github-prs</string>
  <key>CFBundleExecutable</key><string>github-prs</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Sign it with no identity (no Apple account needed), so macOS runs it.
codesign --force --sign - "$APP"

if [ "$DEST" = "--no-install" ]; then
  echo "Built $APP"
  exit 0
fi

mkdir -p "$DEST"
rm -rf "$DEST/$NAME.app"
cp -R "$APP" "$DEST/"
# Tell Spotlight and launchers about it right away.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST/$NAME.app" 2>/dev/null || true
echo "Installed $DEST/$NAME.app"
