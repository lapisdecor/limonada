# Limonada — common tasks.
#
# `make deb`, `make appimage`, `make flatpak` and `make snap` are the four ways
# to ship a Linux build; `make windows` cross-builds the Windows one. See
# packaging/README.md for the tools each needs and what goes into them.

SHELL := /bin/bash
CARGO ?= cargo

# `cargo-deb` wants a Maintainer field. The default is a placeholder — override
# it for a real release:
#   make deb DEB_MAINTAINER='Your Name <you@example.org>'
DEB_MAINTAINER ?= Limonada contributors <limonada@example.invalid>

.PHONY: help build test lint run demo deb appimage flatpak snap windows dist clean

help:
	@echo 'Targets:'
	@echo '  build     Release build (cargo build --release)'
	@echo '  test      Offline test suite'
	@echo '  lint      clippy, warnings as errors'
	@echo '  run       Run the release build'
	@echo '  demo      Run against captured fixtures, no network'
	@echo '  deb       Debian package (requires cargo-deb)'
	@echo '  appimage  AppImage (requires appimagetool)'
	@echo '  flatpak   Flatpak bundle (requires flatpak-builder)'
	@echo '  snap      Snap package (requires snapcraft; builds in LXD)'
	@echo '  windows   Windows .exe, zipped (requires a mingw-w64 cross toolchain)'
	@echo '  dist      The two Linux packages that need no extra SDK'
	@echo '  clean     cargo clean'

build:
	$(CARGO) build --release

test:
	$(CARGO) test

lint:
	$(CARGO) clippy --all-targets -- -D warnings

run:
	$(CARGO) run --release

demo:
	LIMONADA_DEMO=1 $(CARGO) run --release

deb: build
	$(CARGO) deb --no-build --maintainer "$(DEB_MAINTAINER)"

appimage:
	packaging/build-appimage.sh

flatpak:
	packaging/build-flatpak.sh

snap:
	packaging/build-snap.sh

windows:
	packaging/build-windows.sh

dist: deb appimage

clean:
	$(CARGO) clean
