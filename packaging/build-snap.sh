#!/usr/bin/env bash
#
# Build a Limonada snap.
#
# A snap is one installable file holding the app and the metadata a desktop reads
# to list it, confined to the interfaces its manifest declares — the fourth Linux
# artifact, after the .deb, the AppImage and the Flatpak.
#
# Two things shape this script. snapcraft copies every source it is given into
# parts/ wholesale, and this repository's target/ is tens of gigabytes, so the
# build runs in target/snap/project, an rsync'd copy of the sources (under
# target/, so `make clean` removes it too). And the build needs snaps of its own
# — the GNOME extension builds against gnome-core26-sdk, the rust plugin against
# rustup — which installing on the host would cost root; snapcraft's LXD provider
# installs them inside its container instead.
#
# Needs `snapcraft` (sudo snap install snapcraft --classic), `rsync`, and LXD with
# the invoking user in the `lxd` group. SNAP_PROVIDER=destructive builds on the
# host for a machine where the build snaps are already installed.
#
# Usage:
#   packaging/build-snap.sh
#   SNAP_PROVIDER=destructive packaging/build-snap.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# snapcraft names its file after the snap architecture, which is not what
# `uname -m` prints (x86_64 → amd64).
arch="${ARCH:-$(uname -m)}"
case "$arch" in
  x86_64) snap_arch=amd64 ;;
  aarch64) snap_arch=arm64 ;;
  *)
    echo "build-snap: unsupported architecture '$arch' (set ARCH=x86_64 or aarch64)" >&2
    exit 1
    ;;
esac

for tool in snapcraft rsync; do
  if ! command -v "$tool" > /dev/null 2>&1; then
    cat >&2 <<EOF
build-snap: $tool was not found.

Install it and try again, for example:
  sudo snap install snapcraft --classic
  sudo apt install rsync
EOF
    exit 1
  fi
done

# No jq, no python: the version is the first `version = "..."` in Cargo.toml.
version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
if [ -z "$version" ]; then
  echo "build-snap: could not read the version from Cargo.toml" >&2
  exit 1
fi

# A snap's metadata is written before any part is built, so the version lives in
# the manifest too. A release that bumps one and not the other would install
# under a version the app does not report, so it stops here instead.
manifest_version="$(sed -n 's/^version: *"\([^"]*\)"$/\1/p' snap/snapcraft.yaml)"
if [ "$manifest_version" != "$version" ]; then
  echo "build-snap: snap/snapcraft.yaml says version '$manifest_version' but Cargo.toml says '$version'" >&2
  exit 1
fi

stage="$root/target/snap/project"
echo "==> staging the sources in ${stage#"$root/"}"
mkdir -p "$stage"
rsync -a --delete --exclude '/target/' --exclude '/.git/' "$root"/ "$stage"/

provider_args=(--use-lxd)
case "${SNAP_PROVIDER:-lxd}" in
  lxd) ;;
  destructive) provider_args=(--destructive-mode) ;;
  *)
    echo "build-snap: SNAP_PROVIDER must be 'lxd' or 'destructive'" >&2
    exit 1
    ;;
esac

echo "==> packaging Limonada $version for $arch"
(cd "$stage" && snapcraft pack "${provider_args[@]}")

built="$stage/limonada_${version}_${snap_arch}.snap"
if [ ! -f "$built" ]; then
  echo "build-snap: snapcraft finished but wrote no $built" >&2
  exit 1
fi

out="$root/target/limonada_${version}_${snap_arch}.snap"
cp "$built" "$out"

echo "==> wrote $out"
echo "    install with: sudo snap install --dangerous $out"
echo "    run with:     limonada"
