# Packaging

Four Linux artifacts from the same release build, plus a Windows one built from
the same source:

| Artifact | Targets | Needs |
| --- | --- | --- |
| `.deb` | Debian, Ubuntu, Mint, Pop!_OS… | [`cargo-deb`](https://crates.io/crates/cargo-deb) |
| `.AppImage` | any glibc distribution | [`appimagetool`](https://github.com/AppImage/appimagetool) |
| `.flatpak` | any distribution with flatpak installed | [`flatpak-builder`](https://docs.flatpak.org) and the freedesktop 25.08 SDK |
| `.snap` | any distribution with snapd | [`snapcraft`](https://snapcraft.io/snapcraft), LXD for the default build provider, and `squashfs-tools` for the script's architecture check |
| `.zip` (Windows) | Windows 10/11 on x86-64 | mingw-w64, `rustup target add x86_64-pc-windows-gnu` |

All are driven by `make` from the repository root:

```bash
make deb        # → target/debian/limonada_<version>_amd64.deb
make appimage   # → target/Limonada-<version>-<arch>.AppImage
make flatpak    # → target/Limonada-<version>-<arch>.flatpak
make snap       # → target/limonada_<version>_<arch>.snap
make windows    # → target/Limonada-<version>-x86_64-windows.zip
make dist       # the two Linux packages that need no extra SDK
```

None of these tools are required to build or run the app; they are only needed
to produce the packages. `make dist` stays Linux-only on purpose: cross-building
for Windows needs a toolchain that a Linux packaging machine may not have, so
`make windows` is its own step. `make flatpak` is a step of its own too, because
it needs flatpak-builder plus the freedesktop SDK and its rust-stable extension —
hundreds of megabytes that neither `.deb` nor AppImage build touches. `make snap`
is a step of its own for the same kind of reason: snapcraft builds it in a
container, because the build itself installs snaps (the GNOME SDK and rustup).

## What goes in

| File | Installed as |
| --- | --- |
| `target/release/limonada` | `/usr/bin/limonada` |
| `com.gatochalupa.limonada.desktop` | `/usr/share/applications/com.gatochalupa.limonada.desktop` |
| `com.gatochalupa.limonada.metainfo.xml` | `/usr/share/metainfo/com.gatochalupa.limonada.metainfo.xml` |
| `icons/com.gatochalupa.limonada-256.png` | `/usr/share/icons/hicolor/256x256/apps/com.gatochalupa.limonada.png` |
| `icons/com.gatochalupa.limonada.svg` | `/usr/share/icons/hicolor/scalable/apps/com.gatochalupa.limonada.svg` |
| `README.md` | `/usr/share/doc/limonada/README.md` |

Everything that carries an identity is named after the app id,
`com.gatochalupa.limonada`: the desktop entry, the AppStream file and both icons.
That is what a Flatpak requires and what a desktop uses to pair a running window
with its launcher entry, so the same names serve all four packages. The Debian
package itself, and the binary it installs, stay `limonada`.

The icon is a single SVG, exported once to a 256×256 PNG with Inkscape so
desktops that prefer a raster icon have one. It is committed rather than
generated at build time, so packaging needs neither Inkscape nor any other
image tool.

`StartupWMClass=com.gatochalupa.limonada` in the desktop entry matches the app id
the binary sets (`ViewportBuilder::with_app_id("com.gatochalupa.limonada")`), which
is what pairs a running window with its launcher entry.

## Flatpak

`make flatpak` builds `packaging/flatpak/com.gatochalupa.limonada.yaml`:

```bash
make flatpak    # → target/Limonada-<version>-<arch>.flatpak
```

It is the only artifact that runs sandboxed. Its inputs need no renaming for
Flatpak's sake: the entry, the metadata and both icons are already named after the
app id, and their `Icon=`, `<id>` and `<launchable>` already point at themselves,
so the manifest installs what the `.deb` installs. One source per file, three
packages.

Inside `/app` (which is what the app sees as its own tree):

| File | Installed as |
| --- | --- |
| `target/release/limonada`, built in the sandbox | `/app/bin/limonada` |
| `com.gatochalupa.limonada.desktop` | `/app/share/applications/com.gatochalupa.limonada.desktop` |
| `com.gatochalupa.limonada.metainfo.xml` | `/app/share/metainfo/com.gatochalupa.limonada.metainfo.xml` |
| `icons/com.gatochalupa.limonada-256.png` | `/app/share/icons/hicolor/256x256/apps/com.gatochalupa.limonada.png` |
| `icons/com.gatochalupa.limonada.svg` | `/app/share/icons/hicolor/scalable/apps/com.gatochalupa.limonada.svg` |
| `README.md` | `/app/share/doc/com.gatochalupa.limonada/README.md` |

`finish-args` is the whole list of permissions the app needs: `--share=network`
for the Lemmy API, `--device=dri` for the machine's own GL/Vulkan driver (which
is what wgpu and glow open), `--socket=wayland`, `--socket=fallback-x11` and
`--share=ipc`. Nothing else, and in particular no `--filesystem`: settings live in
`$XDG_DATA_HOME/limonada`, which inside the sandbox is
`~/.var/app/com.gatochalupa.limonada/data/limonada` and is already persistent.

### The icon's one-line header comment

The comment above the root element of `icons/com.gatochalupa.limonada.svg` is a single line for a
measured reason. `flatpak-builder` runs `appstreamcli compose` as its cleanup
step, and on the icon's original multi-line header comment that step reproducibly
failed with `E: file-read-error` — killing the build after everything had already
compiled. The same file composed cleanly, three runs out of three, with the
header comment reduced to one short line, with it removed entirely, with every
drawing element on its own, and with comments but no header; the 256×256 PNG
always composed too. The drawing is untouched, and the `.deb` and the AppImage
never rasterize the file, so only the flatpak build was ever exposed.

### Regenerating cargo-sources.json

A Flatpak build has no network, so cargo cannot fetch a single crate: every one
in `Cargo.lock` arrives as a declared source with its checksum.
`packaging/flatpak/cargo-sources.json` is that list, its hashes and the generated
cargo config that points at it, and it is committed for the same reason the icon
is: a package that cannot be built from the tree is not a package. Regenerate it
only when `Cargo.lock` changes:

```bash
curl -LO https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
python3 -m venv .venv && .venv/bin/pip install 'aiohttp<4' 'PyYAML<7' 'tomlkit'
.venv/bin/python flatpak-cargo-generator.py Cargo.lock \
  -o packaging/flatpak/cargo-sources.json
```

The generator is not vendored, just as `cargo-deb` and `appimagetool` are not.

## Snap

`make snap` builds `snap/snapcraft.yaml` through `packaging/build-snap.sh`:

```bash
make snap       # → target/limonada_<version>_<arch>.snap
```

It is confined (`strict`) on base `core26`, which is Ubuntu 26.04 — the
platform the app is developed and verified on. A base is a runtime the host's
snapd brings, so the snap runs on older distributions too.

Inside the snap:

| File | Installed as |
| --- | --- |
| the release binary, built in the build container by `cargo install` | `bin/limonada` |
| `snap/gui/limonada.desktop` | `meta/gui/limonada.desktop` (the launcher entry) |
| the manifest's `icon:`, the committed 256×256 PNG | `meta/gui/icon.png` |
| `snap/command-chain/*` | the wrappers the GNOME extension supplies |

That is the whole payload. Unlike the other three packages, the snap carries no
`usr/share/metainfo`, no icons in the internal theme and no README, because
nothing inside a snap reads them: what a desktop and the store show comes from
`meta/snap.yaml`, which snapcraft writes from the manifest, and the launcher
entry names `meta/gui/icon.png` directly. That is measured rather than assumed —
an earlier version installed all three, and adding them leaves `meta/snap.yaml`
byte-identical, so they were carried weight.

The snap's identity matches the other packages where it matters to a desktop:
`StartupWMClass=com.gatochalupa.limonada`, the app id the binary sets, ties its
window to its launcher entry the same way.

The GNOME extension supplies the desktop runtime as *content* snaps rather than
copies in this package: `gnome-core26` is mounted at `$SNAP/gnome-platform`,
`gpu-2604` at `$SNAP/gpu-2604` for the machine's own Mesa driver (as
`--device=dri` does for the Flatpak), and `gtk-common-themes` at
`$SNAP/data-dir`. With them come the `desktop`, `desktop-legacy`, `opengl`,
`wayland` and `x11` interfaces; the manifest adds only `network`, for the Lemmy
API and for handing a post's link to the browser. There is no `home` plug and no
`filesystem` permission: settings live in `$XDG_DATA_HOME/limonada`, which inside
the snap is `~/snap/limonada/current/.local/share/limonada` and is already
persistent.

### Why the build runs somewhere else

Two measurements shape `build-snap.sh`:

- **snapcraft copies every source it is given into `parts/`.** This repository's
  `target/` was 21 GB when the snap was built, so the script stages the sources
  in `target/snap/project` with `rsync` (`target/` and `.git/` excluded) and
  runs `snapcraft pack` there. The copy lives under `target/`, so `make clean`
  removes it, and it is kept between builds; the crate graph is compiled inside
  snapcraft's container, which also keeps its parts between runs (measured: a
  rebuild's `cargo install` finished in 1.8 s against 385 crates in the first).
- **The build needs snaps of its own.** The GNOME extension builds against the
  `gnome-core26-sdk` snap and the rust plugin against `rustup`, and installing
  those on the host costs root. Snapcraft's LXD provider installs them inside
  its container instead, so the snap builds without a privileged step. LXD has
  to be `lxd init`-ed once and the invoking user in the `lxd` group;
  `SNAP_PROVIDER=destructive` builds on the host for a machine where those snaps
  are already present.

The script also refuses to run when `snap/snapcraft.yaml`'s `version` and
`Cargo.toml`'s disagree: a snap's metadata is written before any part is built,
so a release that bumps one and not the other would install under a version the
app does not report.

The desktop entry in `snap/gui/` is the snap's own entry — snapcraft only reads
desktop entries from there — and it carries the same `Name`, `Exec` and
`StartupWMClass` as `packaging/com.gatochalupa.limonada.desktop`. Its `Icon=` is
`${SNAP}/meta/gui/icon.png`: a file, not a theme name, because a bare name would
be looked up in the host's icon theme, where a snap's icons are not.

The `${SNAP}` form is not cosmetic. Snapcraft copies the entry into `meta/gui`
untouched, but snapd does **not** install that file: it writes a sanitised copy
to `/var/lib/snapd/desktop/applications/` and **drops any `Icon=` path that is
not under `${SNAP}`** (*icon path … is not part of the snap*). The absolute
`/snap/limonada/current/meta/gui/icon.png` an earlier version used therefore
produced an exported entry with no `Icon=` line at all — measured on the
installed snap, which showed the generic icon — while snapd rewrites `${SNAP}`
to that same path for every other snapped app on the machine. The catch is that
`desktop-file-validate` rejects the unexpanded `${SNAP}` as not absolute, so it
is run on a copy with the variable substituted.

### The compose table and the GPU snap

xkbcommon builds its compose table from `$XLOCALEDIR`, which the GPU content
snap's wrapper points at `gpu-2604/mesa-2604/usr/share/X11/locale`. Fifteen of
the `Compose` files there — `pt_PT.UTF-8` among them — include the absolute
`/usr/share/X11/locale/en_US.UTF-8/Compose`, and the `core26` base ships no X11
locale data at all. Every launch therefore printed

```
xkbcommon: ERROR: …/pt_PT.UTF-8/Compose:1:9: failed to open included Compose file "/usr/share/X11/locale/en_US.UTF-8/Compose": No such file or directory
xkbcommon: ERROR: …/pt_PT.UTF-8/Compose:1:9: failed to parse file
```

and dropped the compose table — the thing dead keys and compose sequences use.
The core22 GPU extension exposed this directory with a `/usr/share/X11/locale`
layout; the core26 one exposes only `/usr/share/X11/XErrorDB`. The manifest adds
the missing entry:

```yaml
layout:
  /usr/share/X11/locale:
    symlink: $SNAP/gpu-2604/mesa-2604/usr/share/X11/locale
```

A symlink rather than a bind: it costs nothing at startup and does not depend on
the order in which snapd mounts the content snap and applies layouts, since the
target only has to exist when a compose file is opened. Measured on the built
snap: the entry survives into `meta/snap.yaml` beside the five the extension
contributes, and the file the include names — `en_US.UTF-8/Compose`, 456,061
bytes — is present in the mounted content snap.

### What was measured

`make snap` wrote `target/limonada_0.2.0_amd64.snap` — 12,001,280 bytes, sha256
`f146f994…`, holding **9 files**: the binary, the launcher entry, the icon that
entry names, the four command-chain wrappers, `meta/snap.yaml` and the configure
hook the GNOME extension adds. The file is `amd64`, not `x86_64`, because
snapcraft names it after the snap architecture and the script names what it
writes the same way. Nothing is staged from the runtime, which is why it is that
small; the snap installs its content snaps separately.

The binary inside is 43,057,512 bytes, unstripped (`file` calls it *not stripped*,
BuildID `bd603bea…`), the same size as the one the AppImage ships for
the same reason: neither package strips. The `.deb` and the Flatpak do, at
32 MB. Stripping a copy before installing it is the obvious saving, and it is
not done here for consistency with the AppImage rather than by accident.

Read back from the extracted tree rather than assumed: `meta/snap.yaml` carries
the name, `title`, version, summary, description, `license: AGPL-3.0-only`,
base, `confinement: strict`, the app's command chain (`gpu-2604-wrapper` then
`desktop-launch`), the six plugs, the `gpu-2604`, `gnome-core26` and
`gtk-common-themes` content plugs, and the `/usr/share/X11/locale` symlink
layout; `meta/gui/limonada.desktop` has
`Exec=limonada`, `Icon=${SNAP}/meta/gui/icon.png` and
`StartupWMClass=com.gatochalupa.limonada`, and the icon that line resolves to is
`meta/gui/icon.png` (15,704 bytes). `desktop-file-validate` passes on a copy
with `${SNAP}` expanded to the installed path; the unexpanded form is not
absolute and the validator rejects it. That `meta/snap.yaml` is also byte-identical
(`diff`) to the one an earlier build produced while installing the AppStream
file, both icons and the README under `usr/` — which is why this package carries
none of them.

`snapcraft`'s linters pass with one warning and three infos, all of the same
kind and left deliberately: `contact` is missing (warning), `issues`,
`source-code` and `website` are missing (info). Like the AppStream file's absent
homepage, this repository has no public address, and inventing one would ship a
link that 404s. They have to be filled in before publishing to the store.

The packaged binary was **run**: extracted from the snap and launched on the
host against the captured fixtures (`LIMONADA_DEMO=1 LIMONADA_LOG=1`), it opened
its window, printed

```
limonada: connected to Lemmy (lemmy 0.19.20), 58846 users
limonada: feed loaded 20 posts (20 total, more: true)
limonada: loaded 50 communities
```

and was still running when `timeout` killed it after 20 s. That is the compiled
artifact from inside the snap, not a `cargo run`. The rebuilt package
(BuildID `bd603bea…`) was launched the same way and also stayed up until
`timeout` killed it, after 18 s.

What that does **not** cover, and this is the one limit worth stating plainly:
**the snap was never installed** — `snap install` needs root, and nothing here
installs system-wide. So confinement itself is unverified: that the interfaces
auto-connect, that `gnome-core26` and `gpu-2604` are pulled in as the providers
this manifest names, that wgpu opens a driver under `strict`, that the launcher
icon and `StartupWMClass` pair on a real desktop, and that settings land in
`~/snap/limonada/current/` rather than somewhere the sandbox hides. The run
above was unconfined, on the host, with the host's libraries. Install it with
`sudo snap install --dangerous target/limonada_0.2.0_amd64.snap` to check
those; `--dangerous` is required because the snap is unsigned.

### arm64

The manifest declares two platforms, so the same source produces both builds:

```bash
ARCH=aarch64 make snap     # cross-build on an amd64 machine
make snap                  # on an arm64 machine; amd64 on an amd64 machine
```

`build-snap.sh` passes `--build-for` on every run, which is not decoration: with
a build plan holding both platforms, `snapcraft pack` on an amd64 host builds
both and writes two snaps.

Cross-building needs a linker and a C compiler for the target, because `ring` —
reached through rustls — compiles C and assembly for the machine it will run on.
Both are declared in the part's `build-packages` and both are native to the
build machine (`gcc-aarch64-linux-gnu`, `libc6-dev-arm64-cross` from the amd64
archive), so a cross build pulls nothing from the ports archive.

The target goes into a `.cargo/config.toml` written into the pulled source, and
that is measured rather than assumed. The rust plugin runs `cargo install`,
which builds for the build machine, and `craftctl default` runs the plugin in
snapcraft's own environment — variables exported by `override-build` never reach
cargo. The first attempt did exactly that: `CARGO_BUILD_TARGET` exported, the
pack succeeded, `limonada_0.2.0_arm64.snap` was written, and `file` on the binary
inside said `ELF 64-bit LSB pie executable, x86-64`. snapcraft gives no sign of
it, because the file name comes from the build plan and not from the code.
`build-snap.sh` therefore reads the ELF header (`e_machine`) out of the packed
snap and refuses to write one for the wrong machine — which is why it needs
`unsquashfs`.

#### What was measured (arm64)

`ARCH=aarch64 make snap` wrote `target/limonada_0.2.0_arm64.snap` — 11,087,872
bytes, sha256 `c71cafbf…`, holding the same nine files as the amd64 one. Read
back from the extracted tree: `bin/limonada` is `ELF 64-bit LSB pie executable,
ARM aarch64`, 39,329,936 bytes, `not stripped`, BuildID `87b3f635…`;
`meta/snap.yaml` carries `architectures: [arm64]` and the
`/usr/share/X11/locale` symlink layout beside the GNOME extension's own entries
(it names `/usr/lib/aarch64-linux-gnu/webkit2gtk-4.1` where the amd64 snap names
`/usr/lib/x86_64-linux-gnu/…`); `meta/gui/limonada.desktop` has the same `Exec=`,
`Icon=` and `StartupWMClass=` lines, and the icon it points at is the same
15,704-byte file.

The source was also compiled for aarch64 on this host and its test suite **run
under `qemu-aarch64`**, with the arm64 loader and glibc from
`/usr/aarch64-linux-gnu`: **143 tests passed, 6 ignored** (the `live_smoke`
ones, which need a real instance) across the unit tests, `api_shapes`,
`http_auth`, `image_formats` — AVIF through `rav1d-safe` included — `tree` and
the headless UI renders.

The packaged arm64 binary was launched under that same `qemu-aarch64`: it loads
through the arm64 loader and runs as far as `winit`, which stops with
`WaylandError(Connection(NoWaylandLib))` because this host's desktop is x86-64
and has no aarch64 Wayland client library. There is no window to open here.

What that does **not** cover, plainly: **no Raspberry Pi was available**, so the
snap was not installed and the app was never seen running on ARM hardware. The
wgpu change this build exists for — requesting `adapter.limits()` instead of
wgpu's defaults, see `src/main.rs` — is therefore still unverified on the machine
that reported `LimitsExceeded(FailedLimit { name: "max_color_attachments",
requested: 8, allowed: 4 })`. On the Pi: `make snap`, then
`sudo snap install --dangerous target/limonada_0.2.0_arm64.snap`, then
`limonada`.

One warning is expected on a cross build and is left as it is: the `library`
linter prints `Unable to determine library dependencies for 'bin/limonada'`,
because it resolves dependencies with the build machine's loader while these are
arm64. The base carries them.

## Windows

`make windows` cross-compiles `x86_64-pc-windows-gnu` on a Linux host and zips
the result:

```
target/Limonada-<version>-x86_64-windows.zip
└── Limonada-<version>/
    ├── limonada.exe
    └── README.md
```

There is nothing to install and nothing to bundle. The executable is one file
that imports only DLLs Windows itself provides — `kernel32`, `user32`, `gdi32`,
`opengl32`, `dxgi`, `dwmapi`, `ws2_32`, `shell32`, `msvcrt` and friends — so
there is no mingw runtime DLL to ship beside it, and no C library to install.
Graphics come from the machine's GL/Vulkan drivers through `wgpu` (with `glow`
selectable via `LIMONADA_RENDERER=glow`), TLS is `rustls`, and the AVIF decoder
is compiled in, exactly as on Linux.

A release build is linked as a **GUI** subsystem program
(`#![cfg_attr(all(not(debug_assertions), target_os = "windows"),
windows_subsystem = "windows")]` in `src/main.rs`). Without that attribute the
executable is marked `Windows CUI` and launching it from Explorer opens a console
window behind the app for as long as it runs. Debug builds keep the console, so
the startup logging stays visible.

On Windows itself this script is not needed: an ordinary `cargo build --release`
with the MSVC toolchain produces the same program.

### What was measured

`make windows` wrote `target/Limonada-0.2.0-x86_64-windows.zip` — 18,129,839
bytes holding a 54,272,224-byte `limonada.exe`. Read back rather than trusted:
the zip's `limonada.exe` is byte-identical to
target/x86_64-pc-windows-gnu/release/limonada.exe (sha256 `b3787b4c…`); the PE
header says `PE32+` / `i386:x86-64`; `Subsystem` is `Windows GUI` (`00000002`),
not `Windows CUI`; every imported DLL is a system one, with no `libgcc_s_seh-1`,
`libstdc++-6` or `libwinpthread-1` beside it; and the strings the current source
puts there are present — `Limonada/0.2.0 (Lemmy desktop client)` and the
clickable-community hover text. `cargo clippy --all-targets --target
x86_64-pc-windows-gnu -- -D warnings` is clean, so the test code compiles for
Windows too.

What that does **not** cover: the executable was never launched. This host has
the cross toolchain but no Windows machine and no Wine, so the claims above are
about the file, not about a window appearing. Two consequences worth knowing:

- **No code signing.** SmartScreen shows "Windows protected your PC" on first
  run; that needs a certificate, not a build flag.
- **No version resource.** There is no `.rc` file, so Explorer's Properties →
  Details shows no version, and Explorer and the taskbar show the generic
  application icon. The version the app itself reports — in its About line — is
  still the build's (`CARGO_PKG_VERSION` → 0.2.0).

## What was measured in the Linux packages

Every Linux artifact was built and then **read back**, rather than trusted from
a successful exit.

**`.AppImage`.** `make appimage` wrote
`target/Limonada-0.2.0-x86_64.AppImage` — about 14 MB, a zstd-compressed
squashfs holding eight files. Extracted with `--appimage-extract` and compared
byte-for-byte against the sources: every file identical, the release binary
included. Launched, it mounts its own filesystem and runs the payload from
inside it, which `/proc/<pid>/exe` confirms resolves to
`/tmp/.mount_LimonaXXXXXX/usr/bin/limonada` — that is what proves `AppRun`
works, where an extracted tree that merely contains files would not. A live run
through the AppImage reached a real instance and reported it:

```
limonada: connected to Lemmy Português (lemmy 0.19.20), 1036 users
limonada: feed loaded 20 posts (20 total, more: true)
limonada: loaded 50 communities
```

Unlike the `.deb`, the AppImage ships that binary *unstripped*: the script
installs `target/release/limonada` as it is, so the payload carries the debug
symbols and is 43.1 MB (43,092,456 bytes) before squashfs compresses it to a
14 MB file. Stripping it first — `strip` on a copy, not on the build output — is
the obvious way to shrink the download.

`appimagetool` is not vendored in the source tree: it is fetched from its own
release page. The script looks for it in `APPIMAGETOOL`, then at
`target/tools/appimagetool` (where a checkout that already fetched it keeps it,
and where `make appimage` finds it with nothing exported), then on `PATH`. With
none of those present the script stops and prints the download command instead
of writing a broken file, and `make appimage` exits non-zero.

**`.deb`.** Inspected with `dpkg-deb`. The entry, both icons and the AppStream
file are byte-identical to `packaging/`. The binary is the same compiled build —
the packaged copy carries the identical `BuildID`
(`3b9dc30406f2af145b2b4a4ac69c8bd9e63d5e6a`) to `target/release/limonada` — but
`cargo-deb` strips it, so it is 32,249,096 bytes against the unstripped
43,092,456. That is the one file in either package that is not byte-identical to
its source, and deliberately: the symbols are not what ships.

**`.flatpak`.** `make flatpak` wrote `target/Limonada-0.2.0-x86_64.flatpak`:
9,123,944 bytes, which `flatpak install --user` reports as 32.7 MB installed.
The binary inside is 32,251,184 bytes and stripped, and it is **not** the same
build as `target/release/limonada` — it is compiled inside the sandbox by the
SDK's rust-stable 1.98.1, so its BuildID is its own (`358b11ab…`, against the
`.deb`'s `3b9dc304…`). All 551 crates came from `cargo-sources.json` with
`--offline --locked`; cargo took 3 m 35 s over them.

Read back from the exported tree rather than assumed: the entry, the metadata and
both icons are installed under the app-id names, the entry carries
`Icon=com.gatochalupa.limonada`, the metadata says
`<id>com.gatochalupa.limonada</id>` and points its `launchable` at
`com.gatochalupa.limonada.desktop`, and the composed catalogue
(`share/app-info/xmls/com.gatochalupa.limonada.xml.gz`) carries the 64 and 128 px
icons. `flatpak info` shows the runtime, the version, the licence and the summary
taken from that metadata; `flatpak info --show-permissions` shows exactly the
sandbox this manifest asks for — `shared=network;ipc;`,
`sockets=x11;wayland;fallback-x11;`, `devices=dri;`.

Installed and **run**, which is the one thing a package cannot prove any other
way: `flatpak ps` lists the instance while it runs, and a launch with
`LIMONADA_LOG=1` printed

```
limonada: connected to Lemmy (lemmy 0.19.21), 58999 users
limonada: feed loaded 20 posts (20 total, more: true)
limonada: loaded 50 communities
```

from *inside* the sandbox — so network, TLS and the graphics driver work through
`--share=network` and `--device=dri`, not just in a `cargo run` on the host. A
demo-mode launch (`LIMONADA_DEMO=1`) also survived 25 s under `timeout` before
being killed, which is a window that did not crash — not a window whose pixels
were inspected, for the same reason no screenshot of this app exists anywhere
here. Nothing was installed system-wide, and nothing was published to a remote.

The first build installed the freedesktop 25.08 SDK, its Platform and the
rust-stable extension from flathub into the *per-user* installation (`--user`
needs no root), and left 1.6 GB of build state and cache in `target/flatpak/`.
A rebuild reuses both, and `packaging/build-flatpak.sh` explains how to switch to
a system installation with `FLATPAK_INSTALLATION_ARGS=--system`.

### One deliberate AppStream warning

`desktop-file-validate` passes on the packaged entry. `appstreamcli validate`
exits non-zero with a single warning, `url-homepage-missing`: the component
declares no homepage URL. `flatpak-builder`'s compose step is not held up by it —
it reports the same thing as a warning and succeeds, which the `app-info`
catalogue in the built bundle confirms. That is left as it is on purpose — this repository has
no public address, and inventing one would ship a link that 404s in every
software centre that reads the file. Add `<url type="homepage">` to
`limonada.metainfo.xml` once the project has somewhere real to point at.

## Before a real release

- **Set a maintainer.** `Cargo.toml`'s `[package.metadata.deb] maintainer` is a
  placeholder, because no individual's name and address belong in this
  repository. Override it per build:

  ```bash
  make deb DEB_MAINTAINER='Your Name <you@example.org>'
  ```

- **Licence.** `limonada.metainfo.xml` declares `AGPL-3.0-only` rather than the
  project's `AGPL-3.0-or-later`. A built binary includes the AVIF decoder, and
  `zenavif` and the `rav1d-safe` decoder beneath it are AGPL-3.0-only, so no
  later version can be granted for the artifact as distributed. See the README's
  licence section.

- **Fill in the snap's store metadata.** `snap/snapcraft.yaml` leaves
  `contact`, `website`, `source-code` and `issues` out on purpose (no public
  address exists), and snapcraft reports them as a warning plus three infos.
  Publishing also needs a registered name: `name: limonada` is unregistered, so
  the first upload claims it or has to change.

- **Add a licence file** to the `.deb` if you redistribute it. The project has
  no `LICENSE` file at the time of writing; add one and a `license-file` entry to
  `[package.metadata.deb]` so the package carries the terms it is offered under.

## Notes on dependencies

The `.deb`, the AppImage and the Flatpak rely on the system for a C library, a graphics driver (GL or
Vulkan) and the X11/Wayland client libraries.

Measured on the built `.deb`, `$auto` resolves to **`libc6` alone**. The binary
links only `libc`, `libm` and `libgcc_s`; the windowing and graphics libraries —
`libGL`, `libEGL`, `libvulkan`, `libwayland-client`, `libX11`, `libxkbcommon` —
are opened at *run* time with `dlopen`, so they are neither linked nor listable
as dependencies. They are therefore the user's choice to have, and both packages
leave them where they are: bundling a graphics driver in an AppImage would pin
the wrong one. The same reasoning is why the AppImage ships no libraries at all.

The AVIF decoder is compiled in, so no `libdav1d` (or other codec package) is
needed at run time — that is the whole point of `zenavif`. See the README's
image-format section.
