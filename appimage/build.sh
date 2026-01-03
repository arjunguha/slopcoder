#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APPDIR="$ROOT_DIR/appimage/AppDir"
APPIMAGE_TOOL="$ROOT_DIR/appimage/appimagetool.AppImage"
APP_NAME="Slopcoder"
VERSION="${VERSION:-0.1.0}"

cd "$ROOT_DIR"

echo "Building frontend..."
cd frontend
npm install
npm run build
cd "$ROOT_DIR"

echo "Building server..."
cargo build --release -p slopcoder-server

echo "Preparing AppDir..."
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/slopcoder/frontend"

cp "$ROOT_DIR/target/release/slopcoder-server" "$APPDIR/usr/bin/"
cp -R "$ROOT_DIR/frontend/dist" "$APPDIR/usr/share/slopcoder/frontend/"
cp "$ROOT_DIR/appimage/Slopcoder.desktop" "$APPDIR/"
cp "$ROOT_DIR/appimage/slopcoder.svg" "$APPDIR/"
cp "$ROOT_DIR/appimage/AppRun" "$APPDIR/"

chmod +x "$APPDIR/AppRun"

if [[ ! -x "$APPIMAGE_TOOL" ]]; then
  echo "Downloading appimagetool..."
  curl -L -o "$APPIMAGE_TOOL" \
    "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
  chmod +x "$APPIMAGE_TOOL"
fi

OUTPUT="$ROOT_DIR/appimage/${APP_NAME}-${VERSION}-x86_64.AppImage"
echo "Building AppImage at $OUTPUT"
"$APPIMAGE_TOOL" "$APPDIR" "$OUTPUT"

echo "Done."
