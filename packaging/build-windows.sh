#!/usr/bin/env bash
#
# Cross-build the Windows executable and zip it.
#
# The app is plain Rust plus eframe/egui and a pure-Rust AVIF decoder, so the
# Windows build needs no Windows SDK, no C dependencies and no bundled runtime:
# it is the same source, compiled for `x86_64-pc-windows-gnu` against a mingw-w64
# toolchain. The result is a single `limonada.exe` that carries everything it
# needs and links only DLLs that ship with Windows (kernel32, user32, gdi32,
# opengl32, dxgi, dwmapi, ws2_32… — no mingw runtime beside it).
#
# Needs:
#   rustup target add x86_64-pc-windows-gnu
#   a mingw-w64 gcc and windres on PATH   (e.g. `apt install gcc-mingw-w64-x86-64`)
#
# Building on Windows itself does not need this script — there the ordinary
# `cargo build --release` with the MSVC toolchain produces the same program.
#
# Usage:
#   packaging/build-windows.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

target="x86_64-pc-windows-gnu"

missing=""
if ! rustup target list --installed 2> /dev/null | grep -qx "$target"; then
  missing="  rustup target add $target"
fi
if ! command -v x86_64-w64-mingw32-gcc > /dev/null 2>&1; then
  missing="${missing}
  apt install gcc-mingw-w64-x86-64    # or the equivalent for your distribution"
fi
if [ -n "$missing" ]; then
  cat >&2 <<EOF
build-windows: the Windows cross toolchain is incomplete.

Install what is missing and try again:
$(printf '%s\n' "$missing")
EOF
  exit 1
fi

echo "==> building the release binary for $target"
cargo build --release --target "$target"

exe="target/$target/release/limonada.exe"
if [ ! -f "$exe" ]; then
  echo "build-windows: $exe was not produced" >&2
  exit 1
fi

# No jq, no python: the version is the first `version = "..."` in Cargo.toml.
version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
if [ -z "$version" ]; then
  echo "build-windows: could not read the version from Cargo.toml" >&2
  exit 1
fi
echo "==> packaging Limonada $version for Windows x86_64"

# One folder inside the zip, so extracting it does not scatter files: the
# executable and the README that explains what it is.
stage="$root/target/windows/Limonada-$version"
rm -rf "$stage"
mkdir -p "$stage"
install -m 755 "$exe" "$stage/limonada.exe"
install -m 644 README.md "$stage/README.md"

zip_path="$root/target/Limonada-$version-x86_64-windows.zip"
rm -f "$zip_path"
(cd "$root/target/windows" && zip -q -9 -r "$zip_path" "Limonada-$version")

echo "==> $zip_path"
stat -c '    %s bytes  %n' "$zip_path" "$stage/limonada.exe"
