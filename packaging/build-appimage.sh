#!/usr/bin/env bash
#
# Build a Limonada AppImage.
#
# An AppImage is one executable file holding the app and its own files, runnable
# on any reasonably recent glibc distribution with no installation — the
# counterpart to the .deb, which targets Debian and Ubuntu specifically.
#
# It bundles no libraries at all. The binary links only the C runtime (`libc`,
# `libm`, `libgcc_s` — measured with `ldd` on the built package), and the
# graphics and windowing libraries it needs (`libGL`, `libEGL`, `libvulkan`,
# `libwayland-client`, `libX11`, `libxkbcommon`) it opens at run time with
# `dlopen`. So there is nothing to bundle: those belong to the machine's desktop
# and, bundled, would pin the wrong driver.
#
# Needs `appimagetool`: an explicit APPIMAGETOOL, or the copy kept in
# `target/tools/appimagetool` (where the packaging notes put it), or one on
# PATH, in that order:
#
#   https://github.com/AppImage/appimagetool
#
# Usage:
#   packaging/build-appimage.sh
#   APPIMAGETOOL=./appimagetool packaging/build-appimage.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

arch="${ARCH:-$(uname -m)}"
case "$arch" in
  x86_64 | aarch64) ;;
  *)
    echo "build-appimage: unsupported architecture '$arch' (set ARCH=x86_64 or aarch64)" >&2
    exit 1
    ;;
esac

# `target/tools/appimagetool` is checked before PATH so that `make appimage`
# works in a checkout that already has the tool, without exporting anything.
if [ -n "${APPIMAGETOOL:-}" ]; then
  tool="$APPIMAGETOOL"
elif [ -x "$root/target/tools/appimagetool" ]; then
  tool="$root/target/tools/appimagetool"
else
  tool=appimagetool
fi
if ! command -v "$tool" > /dev/null 2>&1 && [ ! -x "$tool" ]; then
  cat >&2 <<EOF
build-appimage: appimagetool was not found.

Install it and try again, for example:
  curl -L -o appimagetool \\
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
  chmod +x appimagetool
  APPIMAGETOOL=./appimagetool packaging/build-appimage.sh

It is also picked up from target/tools/appimagetool when that exists.
EOF
  exit 1
fi

echo "==> building the release binary"
cargo build --release

# No jq, no python: the version is the first `version = "..."` in Cargo.toml.
version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
if [ -z "$version" ]; then
  echo "build-appimage: could not read the version from Cargo.toml" >&2
  exit 1
fi
echo "==> packaging Limonada $version for $arch"

appdir="$root/target/appimage/Limonada.AppDir"
rm -rf "$appdir"
mkdir -p \
  "$appdir/usr/bin" \
  "$appdir/usr/share/applications" \
  "$appdir/usr/share/metainfo" \
  "$appdir/usr/share/icons/hicolor/256x256/apps" \
  "$appdir/usr/share/icons/hicolor/scalable/apps"

install -m 755 target/release/limonada "$appdir/usr/bin/limonada"
install -m 644 packaging/com.gatochalupa.limonada.desktop \
  "$appdir/usr/share/applications/com.gatochalupa.limonada.desktop"
install -m 644 packaging/com.gatochalupa.limonada.metainfo.xml \
  "$appdir/usr/share/metainfo/com.gatochalupa.limonada.metainfo.xml"
install -m 644 packaging/icons/com.gatochalupa.limonada-256.png \
  "$appdir/usr/share/icons/hicolor/256x256/apps/com.gatochalupa.limonada.png"
install -m 644 packaging/icons/com.gatochalupa.limonada.svg \
  "$appdir/usr/share/icons/hicolor/scalable/apps/com.gatochalupa.limonada.svg"

# The AppDir root also wants the entry and the icon beside each other, which is
# where AppImage tooling looks for them.
install -m 644 packaging/com.gatochalupa.limonada.desktop \
  "$appdir/com.gatochalupa.limonada.desktop"
install -m 644 packaging/icons/com.gatochalupa.limonada-256.png \
  "$appdir/com.gatochalupa.limonada.png"

cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
# Resolve this script's own directory so the payload is found wherever the
# AppImage happens to be mounted, then pass every argument straight through.
here="$(dirname "$(readlink -f "$0")")"
exec "$here/usr/bin/limonada" "$@"
RUN
chmod 755 "$appdir/AppRun"

out="$root/target/Limonada-$version-$arch.AppImage"
ARCH="$arch" "$tool" "$appdir" "$out"

echo "==> wrote $out"
