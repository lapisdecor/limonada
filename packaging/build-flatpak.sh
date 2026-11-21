#!/usr/bin/env bash
#
# Build a Limonada Flatpak bundle.
#
# A Flatpak is one installable file holding the app, its own files and the
# metadata a desktop reads to list it — the third Linux artifact, after the .deb
# and the AppImage, and the only one that runs sandboxed.
#
# Flatpak's build sandbox has no network at all, so crates cannot be fetched
# while building. Every crate in Cargo.lock is therefore declared in
# `packaging/flatpak/cargo-sources.json` — generated, committed, and regenerated
# only when Cargo.lock changes (the command is in packaging/README.md) — and
# cargo builds against that vendored tree with `--offline`.
#
# Needs `flatpak` and `flatpak-builder`. The first run installs the freedesktop
# 25.08 SDK, its Platform and the rust-stable SDK extension from flathub into
# the *per-user* installation, because that needs no root; a machine that already
# has them system-wide still gets a per-user copy. Pass
# FLATPAK_INSTALLATION_ARGS=--system to use the system installation where sudo
# is available.
#
# Usage:
#   packaging/build-flatpak.sh
#   FLATPAK_INSTALLATION_ARGS=--system packaging/build-flatpak.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

app_id="com.gatochalupa.limonada"
manifest="packaging/flatpak/$app_id.yaml"

arch="${ARCH:-$(uname -m)}"
case "$arch" in
  x86_64 | aarch64) ;;
  *)
    echo "build-flatpak: unsupported architecture '$arch' (set ARCH=x86_64 or aarch64)" >&2
    exit 1
    ;;
esac

missing=()
for tool in flatpak flatpak-builder; do
  command -v "$tool" > /dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -gt 0 ]; then
  cat >&2 <<EOF
build-flatpak: ${missing[*]} was not found.

Install it and try again, for example:
  sudo apt install flatpak flatpak-builder
EOF
  exit 1
fi

if [ ! -f packaging/flatpak/cargo-sources.json ]; then
  cat >&2 <<EOF
build-flatpak: packaging/flatpak/cargo-sources.json is missing.

It is generated from Cargo.lock, because the flatpak build has no network to
fetch crates with. See "Regenerating cargo-sources.json" in packaging/README.md.
EOF
  exit 1
fi

# No jq, no python: the version is the first `version = "..."` in Cargo.toml.
version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
if [ -z "$version" ]; then
  echo "build-flatpak: could not read the version from Cargo.toml" >&2
  exit 1
fi

installation_args=(--user)
if [ -n "${FLATPAK_INSTALLATION_ARGS:-}" ]; then
  read -r -a installation_args <<< "$FLATPAK_INSTALLATION_ARGS"
fi

echo "==> packaging Limonada $version for $arch"
flatpak-builder \
  "${installation_args[@]}" \
  --install-deps-from=flathub \
  --arch="$arch" \
  --state-dir=target/flatpak/state \
  --repo=target/flatpak/repo \
  --force-clean \
  target/flatpak/build \
  "$manifest"

out="target/Limonada-$version-$arch.flatpak"
flatpak build-bundle target/flatpak/repo "$out" "$app_id"

echo "==> wrote $out"
echo "    install with: flatpak install --user $out"
echo "    run with:     flatpak run $app_id"
