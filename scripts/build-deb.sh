#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
ARCH="$(dpkg --print-architecture)"
PKG_NAME="eojupyter"
OUT_DIR="$ROOT_DIR/dist"
DEB_PATH="$OUT_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"

if [[ "${1:-}" != "--no-build" ]] || [[ ! -x "$ROOT_DIR/target/release/$PKG_NAME" ]]; then
  cargo build --release
fi

STAGE_DIR="$(mktemp -d)"
trap 'rm -rf "$STAGE_DIR"' EXIT

PKG_ROOT="$STAGE_DIR/pkg"
mkdir -p \
  "$PKG_ROOT/DEBIAN" \
  "$PKG_ROOT/usr/bin" \
  "$PKG_ROOT/usr/share/applications" \
  "$PKG_ROOT/usr/share/doc/$PKG_NAME" \
  "$PKG_ROOT/usr/share/icons/hicolor/scalable/apps"

install -m 0755 "$ROOT_DIR/target/release/$PKG_NAME" "$PKG_ROOT/usr/bin/$PKG_NAME"
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "$PKG_ROOT/usr/bin/$PKG_NAME" 2>/dev/null || true
fi

sed 's|^Exec=.*|Exec=/usr/bin/eojupyter %F|' "$ROOT_DIR/eojupyter.desktop" \
  > "$PKG_ROOT/usr/share/applications/io.github.guidov.eojupyter.desktop"
chmod 0644 "$PKG_ROOT/usr/share/applications/io.github.guidov.eojupyter.desktop"

install -m 0644 "$ROOT_DIR/icons/eojupyter.svg" \
  "$PKG_ROOT/usr/share/icons/hicolor/scalable/apps/eojupyter.svg"

for size in 32 48 64 128 256 512; do
  mkdir -p "$PKG_ROOT/usr/share/icons/hicolor/${size}x${size}/apps"
  install -m 0644 "$ROOT_DIR/icons/eojupyter-${size}.png" \
    "$PKG_ROOT/usr/share/icons/hicolor/${size}x${size}/apps/eojupyter.png"
done

install -m 0644 "$ROOT_DIR/README.md" "$PKG_ROOT/usr/share/doc/$PKG_NAME/README.md"

# Compute runtime shared-library dependencies automatically
mkdir -p "$STAGE_DIR/debian"
cat > "$STAGE_DIR/debian/control" <<EOF
Source: $PKG_NAME

Package: $PKG_NAME
Architecture: $ARCH
EOF

DEPENDS="$(
  cd "$STAGE_DIR" && \
  dpkg-shlibdeps -O "$PKG_ROOT/usr/bin/$PKG_NAME" 2>/dev/null \
    | sed -n 's/^shlibs:Depends=//p'
)"
if [[ -z "$DEPENDS" ]]; then
  DEPENDS="libgtk-3-0, libwebkit2gtk-4.1-0"
fi

INSTALLED_SIZE="$(du -sk "$PKG_ROOT/usr" | awk '{print $1}')"

cat > "$PKG_ROOT/DEBIAN/control" <<EOF
Package: $PKG_NAME
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Maintainer: Guido Vettoretti <2guido@gmail.com>
Homepage: https://github.com/guidov/eojupyter
Installed-Size: $INSTALLED_SIZE
Depends: $DEPENDS
Description: Eye of Jupyter — fast, read-only IDE-style Jupyter Notebook viewer
 eojupyter is a fast, read-only Jupyter Notebook (.ipynb) viewer written in Rust
 with a VS Code-style file explorer on the left and a GitHub-style notebook
 viewer on the right. Supports Light and Dark themes, keyboard-first navigation,
 and native desktop or local browser modes.
EOF

cat > "$PKG_ROOT/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
  fi
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
  fi
fi
EOF
chmod 0755 "$PKG_ROOT/DEBIAN/postinst"

cat > "$PKG_ROOT/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "purge" ]; then
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
  fi
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
  fi
fi
EOF
chmod 0755 "$PKG_ROOT/DEBIAN/postrm"

mkdir -p "$OUT_DIR"
dpkg-deb --root-owner-group --build "$PKG_ROOT" "$DEB_PATH" >/dev/null
echo "Built $DEB_PATH ($(du -h "$DEB_PATH" | awk '{print $1}'))"
