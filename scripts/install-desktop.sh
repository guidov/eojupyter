#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_PATH="${1:-$ROOT_DIR/target/release/eojupyter}"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

mkdir -p "$ICON_DIR/scalable/apps" "$APP_DIR"
cp "$ROOT_DIR/icons/eojupyter.svg" "$ICON_DIR/scalable/apps/eojupyter.svg"

for size in 32 48 64 128 256 512; do
  mkdir -p "$ICON_DIR/${size}x${size}/apps"
  cp "$ROOT_DIR/icons/eojupyter-${size}.png" "$ICON_DIR/${size}x${size}/apps/eojupyter.png"
done

rm -f "$APP_DIR/eojupyter.desktop"
sed "s|^Exec=.*|Exec=$BIN_PATH %F|" "$ROOT_DIR/eojupyter.desktop" > "$APP_DIR/io.github.guidov.eojupyter.desktop"
chmod +x "$APP_DIR/io.github.guidov.eojupyter.desktop"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICON_DIR" >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi

echo "Installed eojupyter desktop entry and icons to ${XDG_DATA_HOME:-$HOME/.local/share}"
