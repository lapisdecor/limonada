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
# Needs `snapcraft` (sudo snap install snapcraft --classic), `rsync`,
# `unsquashfs` (squashfs-tools), and LXD with the invoking user in the `lxd`
# group. SNAP_PROVIDER=destructive builds on the host for a machine where the
# build snaps are already installed.
#
# ARCH=aarch64 on an amd64 machine is a cross-build: the manifest declares the
# cross toolchain and the script asks snapcraft for that platform.
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

for tool in snapcraft rsync unsquashfs; do
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

# The manifest declares a platform per architecture, so the build plan holds
# two entries and snapcraft needs to be told which one to run: without this it
# builds both (two snaps, and only one of them is wanted). ARCH=aarch64 on an
# amd64 machine is a cross-build — the manifest names the cross toolchain.
echo "==> packaging Limonada $version for $arch"
(cd "$stage" && snapcraft pack "${provider_args[@]}" --build-for "$snap_arch")

built="$stage/limonada_${version}_${snap_arch}.snap"
if [ ! -f "$built" ]; then
  echo "build-snap: snapcraft finished but wrote no $built" >&2
  exit 1
fi

# The file name comes from the build plan, not from what is inside: a
# cross-build whose target was never configured writes _arm64 around an x86-64
# binary and snapcraft still reports success (measured). Read the ELF header out
# of the package and refuse to hand over a snap for the wrong machine.
case "$snap_arch" in
  amd64) want_machine=3e00 ;;  # EM_X86_64
  arm64) want_machine=b700 ;;  # EM_AARCH64
  esac
# Through a file rather than a pipe into `head`: closing the pipe early is a
# SIGPIPE for unsquashfs, and `pipefail` turns that into a build failure (it
# did).
tmp_elf="$stage/.elf-check"
unsquashfs -cat "$built" bin/limonada > "$tmp_elf"
machine="$(od -An -j18 -N2 -tx1 "$tmp_elf" | tr -d ' \n')"
rm -f "$tmp_elf"
if [ "$machine" != "$want_machine" ]; then
  echo "build-snap: $built holds a binary with ELF e_machine 0x$machine," >&2
  echo "            not $snap_arch (0x$want_machine)" >&2
  exit 1
fi

out="$root/target/limonada_${version}_${snap_arch}.snap"
cp "$built" "$out"

echo "==> wrote $out ($snap_arch, ELF e_machine 0x$machine)"
echo "    install with: sudo snap install --dangerous $out"
echo "    run with:     limonada"
