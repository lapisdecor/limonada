# 🍋 Limonada

A native desktop client for [Lemmy](https://join-lemmy.org), built in Rust with
[egui](https://github.com/emilk/egui) / eframe. Runs on Linux, macOS and Windows;
developed and verified on Ubuntu 26.04 (Wayland) against a live `lemmy.ml`
running Lemmy 0.19.20.

## Scope

**This version browses, searches, and — once you sign in — votes and comments.**
It reads posts, communities and comment threads from any Lemmy instance. It
writes exactly three things, each because you asked for it: a sign-in, a vote,
and a comment. Creating posts is still out of scope.

Implemented:

- **Feed** — hot/active/new/top/… sort, All / Local listing, cursor pagination,
  thumbnails, score and comment chips, explicit loading, empty and error states.
  Each row names its community, and clicking that name scopes the feed to it
  instead of opening the post.
- **Community sidebar** — the instance's *active* communities (`sort=TopAll`;
  without a sort the endpoint answers newest-first, which is a page of brand-new
  communities with nothing in them), filterable both in that page and across the
  whole instance: once a filter holds still for 300 ms, `search?type_=Communities`
  is asked and matches beyond the loaded page are listed under "Elsewhere on
  this instance". Clicking one scopes the feed to it. An empty community says so
  in its own words and offers "Show all posts".
- **Post detail** — Markdown-rendered body (headings, links, lists, code, inline
  images), the post's link, and the full comment thread. The community in the
  header is a link: clicking it closes the thread and shows that community's
  posts.
- **Comment threads** — nested rendering with collapsible subthreads and
  selectable comment sort.
- **Pictures at full resolution** — clicking a post's picture opens it over the
  post, at the best resolution the instance offers, either fitted to the window
  or at one image pixel per screen pixel (scrolling as needed). The overlay names
  the format and size, hands the URL to the browser on request, and closes on
  `Esc`, a click outside, or its own button.
- **Search** — posts, comments, communities and users, with a type filter and a
  sort, always across every instance this one federates with (never the feed's
  `Local` listing), and optionally narrowed to one community by an explicit
  "Only in !x" toggle — never inherited silently from whatever the feed is
  showing. Community results show their instance — `!linux@lemmy.world` — where
  the same name exists on several. Ranked with `TopAll` by default: the server ranks the community
  lens with the same sort, and `Hot` answered `q=linux` with a six-subscriber
  `!fedora` first and never listed `!linux` (68,166) at all. A result opens the
  post it belongs to, or scopes the feed to the community. An empty query is
  refused rather than sent (a bare `q=` returns the whole instance), "nothing
  found" is kept distinct from "nothing searched yet", and a narrowed search
  that finds nothing says it was narrowed and offers to search the instance.
- **Sign in** — username or email, password, and an optional two-factor code,
  against whichever instance is configured. The password is sent once and never
  stored; what is kept is the token the instance returns, in the settings file,
  readable by the owner alone. Signing out forgets it. An account is needed for
  voting, commenting and the `Subscribed` listing, and for nothing else.
- **Voting** — up and down on posts and comments. Clicking the arrow you already
  voted with takes the vote back, which is Lemmy's `score` of `0` and the only
  way to undo one. The row is left untouched until the instance answers, and is
  then replaced with the instance's own view, so the score on screen is the
  server's rather than a local guess. A vote with no session is never sent; the
  status bar says to sign in instead.
- **Commenting** — a new comment on the post, or a reply to any comment. The
  thread is reloaded after posting so the comment appears where the server
  sorted it rather than spliced in at a guess. An empty draft is refused before
  it is sent.
- **Settings** — instance URL, demo mode, theme, default sort preferences.
  Persisted to disk and restored on launch.
- **Demo mode** — serves captured responses from `fixtures/` with no network.
  It refuses every write, sign-in included, and says so.

Deliberately **not** implemented, in rough priority order: creating posts,
inbox and notifications, multiple accounts, and a macOS package. The
`Subscribed` listing needs a session, so signed out the UI says so in plain
language rather than failing obscurely.

## Packaging

Four Linux artifacts, all driven by `make` from the repository root and all
built from the same release binary, plus a Windows executable cross-built from
the same source:

```bash
make deb        # → target/debian/limonada_<version>_amd64.deb  (needs cargo-deb)
make appimage   # → target/Limonada-<version>-<arch>.AppImage   (needs appimagetool)
make flatpak    # → target/Limonada-<version>-<arch>.flatpak    (needs flatpak-builder)
make snap       # → target/limonada_<version>_<arch>.snap      (needs snapcraft)
make windows    # → target/Limonada-<version>-x86_64-windows.zip (needs mingw-w64)
```

All four pair a window with its launcher through one app id,
`com.gatochalupa.limonada` — the app id the binary sets, the component id in the
AppStream metadata, and every package's `StartupWMClass` — and the Flatpak and
the snap are the two that run confined.

None of these tools is needed to build or run the app. See
[`packaging/README.md`](packaging/README.md) for what is installed where, and
for the two things to settle before a real release: a maintainer address (the
repository carries a deliberate placeholder) and a `LICENSE` file to ship.

Two measured details shape these packages, and neither is the obvious guess:

- **The package depends on `libc6` and nothing else.** `cargo-deb`'s `$auto`
  resolves from `ldd`, and the built binary links only `libc`, `libm` and
  `libgcc_s`. The graphics and windowing libraries it actually needs (`libGL`,
  `libEGL`, `libvulkan`, `libwayland-client`, `libX11`, `libxkbcommon`) are
  opened at *run* time with `dlopen`, so they never reach a dependency list.
  That is also why the AppImage bundles no libraries: bundling a graphics driver
  would pin the wrong one.
- **The icon is one SVG, exported once to a 256×256 PNG** with Inkscape and
  committed. So producing a package needs no image tool, and the two sizes
  desktops ask for cannot drift apart.

## Build and run

Requires Rust 1.95 or later (egui 0.36's minimum; developed on 1.97.1), and the
usual Linux GUI development libraries (xkbcommon, fontconfig, and a GL/Vulkan
driver).

```bash
cargo run --release
```

On Windows there is nothing extra to install: `cargo build --release` with the
MSVC toolchain produces a self-contained `limonada.exe` that imports only DLLs
Windows itself ships, with TLS and the AVIF decoder compiled in. `make windows`
cross-builds that executable from Linux and zips it — see
[`packaging/README.md`](packaging/README.md).

### Image formats

`egui_extras` decodes images through the `image` crate but enables no formats of
its own, and `eframe`/`arboard` only enable `png`. `Cargo.toml` therefore adds
`image` as a direct dependency with `jpeg`, `png`, `webp` and `gif`, and turns on
`egui_extras`' `gif`, `webp` and `svg` loaders.

This is not optional polish. A missing codec is neither a compile error nor a
runtime error — egui just draws an empty space — so JPEG thumbnails, which are
the most common kind Lemmy serves, disappeared silently. `tests/image_formats.rs`
decodes real bytes in each format so that cannot happen again unnoticed.

**AVIF** — which instances do serve (a live scan found 3 among 13 image posts) —
needs a decoder of its own, because the one `image` has is behind `avif-native`,
which links the C library libdav1d. That would make a system package a build
requirement: `dav1d-sys` fails its build script with *"The system library `dav1d`
required by crate `dav1d-sys` was not found"* without `dav1d.pc`, so a plain
`cargo build` would stop being self-contained. AVIF is therefore decoded by
[`zenavif`](https://crates.io/crates/zenavif) — pure Rust, no system libraries —
through the loader in [`src/avif.rs`](src/avif.rs). That loader is installed
*after* `egui_extras`, which matters: egui asks the most recently added loader
first, and every image whose bytes are not AVIF is declined so egui_extras still
gets it. AVIF is recognised from the container header, not from the URL, because
a transcoding image proxy serves AVIF under whatever name it was asked for —
`lemmy.zip` answers `image/avif` for a URL ending in `.jpeg` or in
`/api/v3/image_proxy?url=…`, and judging by the extension left every one of those
thumbnails blank.

That choice was weighed, because AVIF decoders are usually heavy. Measured here:
the AVIF packages are 52 crates nothing else in the tree needs, and they cold
compile in ~37 s (release) against ~16 s for the `image` subtree — real, but a
cost paid at build time only. Nothing outside cargo is required to pay it, which
is the part that matters: this machine has no `nasm` and no `dav1d`, and the
build works, because `rav1d-safe`'s `nasm-rs` build dependency is inert unless
its `asm` feature is on and only `bitdepth_8`/`bitdepth_16` are enabled. The
features that would make it heavy — `unsafe-asm` (C FFI), `encode` (an AV1
*encoder*, via `ravif`) — are all off, so no second decoder and no encoder enter
the tree.

The alternatives were measured rather than assumed. The MIT-licensed `avif-rust`
was tried first and **failed on a real post's picture** — *"AV1 tile 0 entropy
validation failed: AV1 entropy decoder exited after too many padding bits"* —
which is why the fixture is a real captured AVIF rather than a generated one, and
why the live test decodes AVIF from three live instances. A live 47,536-byte AVIF
decodes to 1920x1200 with 33,066 distinct colours, and takes ~1.8 s; five more
from one later run returned 640x640, 1000x1500, 1080x562, 586x751 and 1536x1024
in 0.5-2.6 s each. That is far too long to spend inside a frame, so the decode
runs on a worker thread and the result is left for the next frame to find.

Formats with no decoder at all — BMP, TIFF, HEIC — are the reverse case: the
picture viewer names the format it cannot read and points at **Open in browser**,
rather than leaving the user with egui's bare warning glyph. `state::decodes` is
the single list of what the build can show.

Run against captured data with no network access at all:

```bash
LIMONADA_DEMO=1 cargo run --release
```

### Environment variables

All optional.

| Variable | Purpose |
| --- | --- |
| `LIMONADA_DEMO=1` | Serve responses from `fixtures/` instead of the network. Overrides the saved setting. |
| `LIMONADA_FIXTURES=<dir>` | Read fixtures from somewhere other than `./fixtures`. |
| `LIMONADA_RENDERER=glow\|wgpu` | Choose the rendering backend. Default is `wgpu`. |
| `LIMONADA_LOG=1` | Print one line per completed load to stderr, so a terminal launch reports what it actually fetched. |
| `LIMONADA_OPEN_POST=<id>` | Open a specific post at startup. A demo and diagnostic aid. |
| `LIMONADA_OPEN_PICTURE=1` | Open the startup post's picture at full resolution. The same kind of aid: the overlay is otherwise only reachable by clicking. |

Settings live in `$XDG_DATA_HOME/limonada/settings.json`
(`~/.local/share/limonada/settings.json`). A corrupt settings file is reported
and ignored rather than being allowed to stop the app from starting.

## Architecture

```
src/
  main.rs            window setup and backend selection
  app.rs             Limonada: state, lifecycle, per-frame draw, eframe::App
  state.rs           settings, persistence, session, search, the async resource holders
  tree.rs            comment-thread assembly from materialised paths
  time.rs            RFC 3339 parsing and age formatting (no date dependency)
  avif.rs            the AVIF image loader (pure Rust, no C library)
  api/
    transport.rs     Transport trait: HttpTransport | FixtureTransport | RefusingTransport,
                     and the session token the HTTP one carries
    client.rs        typed Lemmy v3 methods over a Transport: the reads and the four writes
    types.rs         serde models for the wire shapes
    job.rs           worker-thread requests, so the UI never blocks
    params.rs        Sort, CommentSort and SearchType query vocabularies
  ui/                one module per region: topbar, feed, detail, communities,
                     search, auth, settings, statusbar, image_viewer
fixtures/            real captured API responses and pictures, used by demo mode and tests
packaging/           .desktop, AppStream metadata, the icon, the package scripts
snap/                snapcraft.yaml and the snap's launcher entry
tests/               API shape, HTTP auth, tree, image formats, headless render, live smoke
```

Three decisions shape everything else:

**A transport seam.** Everything above `Transport` is synchronous, so the whole
API layer is unit-testable with no network and no mocking framework.
`FixtureTransport` doubles as demo mode. `RefusingTransport` turns an unusable
instance URL into an ordinary error state instead of needing `Option<Client>`
checks throughout the UI. The same trait carries the session token
(`set_auth`/`auth`), defaulted to a no-op — so the two offline transports behave
exactly as they did before a session existed, and demo mode's refusal to write is
unaffected by the sign-in path.

**No async runtime.** egui is single-threaded and immediate-mode, so a blocking
HTTP call in the draw path would freeze the window. Each request instead runs on
a short-lived worker thread that reports back over a channel and asks egui to
repaint; the UI polls once per frame. This keeps the API layer synchronous and
avoids nesting or dropping an executor. Tokio will earn its place when something
needs streaming.

**Rendering never mutates state.** A draw pass collects what the user asked for
(open this post, refresh, change scope) into local variables, which are applied
after the frame. This removes a whole class of borrow-checker fights and
mid-draw inconsistency.

**No `unsafe`, enforced in this project.** Both of this project's crates carry
`#![forbid(unsafe_code)]`, so the property cannot regress silently, and nothing
here needs it. Dependencies are not covered by that promise, and the AVIF decoder
is the one that stretches it: `rav1d-safe` is the memory-safe Rust port of
dav1d, but it still contains `unsafe` of its own, as does `zenavif` — "safe"
there describes the port's design, not a guarantee this project can make about
its dependency tree.

## Verification

```bash
cargo test                                    # 129 tests, fully offline
cargo clippy --all-targets -- -D warnings     # clean
cargo test --test live_smoke -- --ignored --nocapture   # hits real instances
```

- **Wire shapes** (`tests/api_shapes.rs`) are decoded from real captured
  payloads, including the fields instances omit when null, the pagination
  cursor, and the `my_vote` a signed-out response leaves out entirely. A
  recording transport asserts the exact query parameters and JSON bodies sent —
  for a search and for each of the four writes (sign-in, post vote, comment
  vote, comment) — and one test asserts the read path issues **no writes at
  all**, enforced rather than assumed.
- **The token really reaches the wire** (`tests/http_auth.rs`). `api_shapes.rs`
  can only show that the client hands a token to the transport; only a socket
  can show what the transport then sends. So a one-shot HTTP server on a
  loopback port reads the request a real `HttpTransport` made, and asserts the
  token arrived as `Authorization: Bearer <token>`, that an anonymous request
  carries no `Authorization` header at all, that clearing the token stops it
  being sent, and that the HTTP 401 an unauthenticated write earns is reported
  as `ApiError::Unauthorized` rather than a bare status.
- **Thread assembly** (`tests/tree.rs`, plus unit tests) covers nesting, sibling
  order, and the cases that actually happen in the wild: a comment whose parent
  was truncated away, unusable or self-referential paths, contradictory
  parent cycles, and chains deeper than the nesting cap. Nothing is ever dropped
  or looped on.
- **Headless rendering** (`tests/ui_render.rs`) drives the real per-frame draw
  function through a bare `egui::Context` — no window, no GPU, no network — for
  the feed, a thread, a collapsed thread, every loading/empty/error state, and
  narrow windows. Each frame's **accessibility tree** is then checked for the
  expected text: all 20 captured post titles, community names, authors, the
  thread heading and Markdown-rendered comment bodies. That is a content
  assertion, not merely "something was drawn".
- **Interaction, not just layout.** The picture viewer is exercised by clicking
  it: the test reads the picture widget's rectangle from the frame's
  accessibility tree, sends a real press and release there, and asserts that the
  post's *own* image opened rather than the thumbnail, that the overlay's words
  are in the next frame, and that `Esc` removes it again. Dropping the widget's
  click handling makes that test fail — verified by mutation. No coordinates are
  hardcoded, so a layout change cannot silently invalidate it.
  Each of the overlay's three ways out has its own test, because covering some
  of them covered none of this one: **the ✕ button did nothing at all**, and the
  suite was green, because `Esc` and the backdrop were tested and the button was
  not. It reported the close through a flag of the overlay's own, and `show`
  then assigned egui's own close signal to that same flag, discarding it — a
  single `=` where `|=` would have done. It now reports through egui's signal
  (`ui.close()`) like every other egui container, so there is one channel rather
  than two. The test clicks the ✕ in *both* zoom modes, including the 1:1 view
  the report came from, and restoring the old assignment makes it fail with "the
  ✕ button should dismiss the picture in Fit" — verified by mutation, in that
  direction too.
- **The new surfaces are driven the same way, not just rendered.** The search
  view is painted from a real captured search response, with each of its four
  section headings asserted by text. Voting and commenting go through the real
  widgets: a frame's accessibility tree yields the rectangle of the ▲ arrow, the
  "✎ Add a comment" button or a comment's "↩ Reply", a real press and release is
  sent there, and the test then asserts both what the stub transport received
  *and* that the app applied the answer — the post's score moved by exactly one
  and `my_vote` became `Some(1)`, or the reply carried the `parent_id` of the
  comment that was clicked. Clicking the same arrow twice sends `score` 0 the
  second time, and voting signed out sends nothing at all and says why.

  The first version of those tests was wrong in an instructive way: the wait
  ended as soon as the *worker thread* wrote the request, before the app had
  polled the answer, so the assertions saw the state from before the vote. The
  predicate now observes app state, which is the thing under test.
- **Live smoke test** (`tests/live_smoke.rs`, ignored by default) drives the
  shipping HTTP client end to end against real instances: instance info, feed
  page one, pagination with no overlap, community list, a community-scoped feed,
  a post, and a comment thread assembled from live data. Two assertions there
  exist because the sidebar used to be a directory of emptiness: the *first*
  listed community must have posts (the old test scanned ten and stopped at the
  first non-empty, so it passed while the UI showed "No posts" for most
  clicks), and a community search for `linux` must lead with a well known
  `!linux` rather than a tiny namesake. It also asserts an
  unreachable instance produces a retryable, user-facing error rather than a
  panic. For pictures it scans two instances and, for each candidate, checks that
  the URL the viewer would open really is the larger one — downloading both and
  comparing pixel dimensions — and that a post linking a web page falls back to
  its thumbnail. It skips a format this build cannot decode only after confirming
  from the bytes that it really is one (see limits).

  Search is covered there too, and being read-only can be run on its own:
  `cargo test --test live_smoke -- --ignored searches_a_live_instance`. The
  write path — sign in, vote, take the vote back, comment — is covered as well,
  but gated on credentials from the environment (`LIMONADA_TEST_USER` /
  `LIMONADA_TEST_PASSWORD`) so that no account is committed anywhere and the
  test cannot run by accident. It was **not run here**: there was no account to
  run it with. See the limits below for what that leaves unverified.

### Limits of this verification

- **No window pixels were captured.** eframe's single-frame capture
  (`EFRAME_SCREENSHOT_TO`, behind the hidden `__screenshot` feature) writes a
  correctly sized but *entirely blank* PNG in this environment — confirmed blank
  under both Wayland and X11, because the capture reads the framebuffer after
  the buffer swap. External capture was not possible either: the session is
  Wayland, so `:0` is XWayland and `ffmpeg -f x11grab` sees an empty root
  window, and no Wayland capture tool is installed. Rendering is therefore
  verified structurally (shapes plus accessibility text) rather than visually.
- The app was confirmed to run on the real display against live lemmy.ml and to
  load data — `LIMONADA_LOG=1` reports the instance, 20 posts and 50 communities.
- Image formats are verified by decoding real encoded bytes of each kind
  (`tests/image_formats.rs`) and by probing live thumbnail URLs (all 18 on a
  sampled page returned valid JPEG or PNG). Which of them now *appear on screen*
  was not confirmed pixel-wise, for the capture reason above. The picture viewer
  is in the same position: that it opens the larger image, explains an
  undecodable one and closes on `Esc` is asserted through real input events and
  the accessibility tree, but no screenshot shows the picture itself.

  What a real window *did* prove is that the picture arrives. A release launch
  against live `lemmy.world`, with the overlay opened at startup
  (`LIMONADA_OPEN_PICTURE=1`), logged `picture loaded at 3104x4000 px` — the
  measured size of that post's image, and 7.8× its thumbnail in each dimension
  (61× the pixels).
  That can only be printed once egui_extras' HTTP loader has fetched the bytes,
  the `image` crate has decoded them and egui has uploaded the texture, which is
  exactly the path the headless tests leave out.
- **AVIF now has a decoder, but from an outside dependency.** It is `zenavif`
  0.1.6, a young crate (the one before it, `avif-rust`, failed on a real file),
  so the AVIF path is covered by a real captured fixture, a loader test through
  egui, and a live test over three instances — but it is the one decoder here
  that this project does not control, and its licence narrows the project's
  distribution licence as described above.
- **The AVIF decoder panics intermittently, and it is an upstream bug.** Over a
  series of launches, `rav1d-safe` 0.5.7 — the AV1 decoder beneath `zenavif` —
  panics inside its own worker threads: `thread 'rav1d-worker-N' panicked`,
  `overlapping DisjointMut`, at `include/dav1d/picture.rs:628` and `:641`. It
  happened on three of six launches observed here, kills a decoder worker rather
  than the app, and shows up once a frame actually draws a picture — which is why
  the headless tests never saw it. It is **not** a packaging fault: the plain
  `target/release/limonada` and the AppImage hit it at the same rate.
  Upstream [`zenavif` #30](https://github.com/imazen/zenavif/issues/30) matches
  this exact signature and traces it to a genuine access-set conflict between the
  loop filter's write-back and a concurrent CDEF read, fixed in `rav1d-safe`
  commit `49df1fc0`; `rav1d-safe` 0.6.0 carries that fix. This tree still pins
  **0.5.7**, so the repair is an upgrade away. Silencing the check is not one:
  the same report warns that an unchecked build can corrupt pixels instead.
- **Formats with no decoder are explained, not dropped.** The overlay names the
  format (BMP, TIFF, HEIC) and offers the browser rather than showing egui's bare
  warning glyph. A post whose `url` is an image but whose thumbnail is a format
  this build *can* read is still opened at `url`, i.e. explained rather than
  silently downgraded to the smaller readable copy.
- **The write path was verified offline, and against the server source — not
  against a live account here.** An unauthenticated write was measured (HTTP
  401 `{"error":"incorrect_login"}`), and the request bodies, the Bearer header
  and the answer being applied are each asserted. But no account was available,
  so the *success* shapes of `user/login`, `post/like`, `comment/like` and
  `comment` were read from the Lemmy 0.19.20 server source rather than captured
  — and the credential-gated live test in `tests/live_smoke.rs` has not been
  run. Nothing here has cast a real vote or written a real comment.
- **Search is verified both ways.** Its fixture is a real capture, and
  `searches_a_live_instance` was run against lemmy.ml: each `type_` filled only
  its own array (5 posts, or 5 comments, or 5 communities, or 5 users), and
  `type_=All` returned 3 of each.
- **The community directory was measured, not assumed.** Reading the sidebar
  request without a `sort` returned fifty communities of which **31 had zero
  posts and 41 had two or fewer** — `!technology` and `!linux` were absent — so
  clicking one showed an empty feed and filtering could not find them. The same
  request with `sort=TopAll` returned fifty with **none** empty, led by
  `!technology` (21,278 posts), `!world`, `!memes`, `!asklemmy`, `!linux`. Two
  further live findings shaped the fix: `q=linux&type_=All&sort=Hot` ranks a
  six-subscriber `!fedora` first and never lists `!linux`, while `TopAll` does;
  and `q=technology&type_=All&community_id=<another community>` returns empty
  arrays for all four kinds, which is why a search must not inherit the feed's
  scope. `q=!technology` returns no communities at all, so the `!` is stripped
  before a lookup.
- **Search is federated, and was silently local.** `listing_type` really does
  filter community results: `q=linux&type_=Communities&sort=TopAll&listing_type=Local`
  returned twenty `lemmy.ml` communities and nothing else, while
  `listing_type=All` returned `!linuxmemes@lemmy.world`,
  `!linux_gaming@lemmy.world` and `!linux@programming.dev` among them. The search
  screen used to copy the feed's listing, so a feed set to `Local` narrowed every
  search without saying so; it is now always `All`. Both community-name clicks
  are driven for real in `tests/ui_render.rs`: the feed's chip and the thread's
  header each scope the feed and load its posts, and the feed's chip does not
  also open the post.
- **The packages were built and read back, not installed.** `make deb` produced
  a `.deb` whose `dpkg-deb` listing, control fields and `Depends: libc6` are as
  intended, and whose binary's `ldd` reports no unresolved library. `make
  appimage` produced an AppImage whose extracted contents are byte-identical to
  the sources and which, launched, runs its payload from inside its own mount
  (`/proc/<pid>/exe` → `/tmp/.mount_Limona*/usr/bin/limonada`); a live run through
  it reached a real instance. Neither was *installed*, and the AppImage has not
  been launched on a second distribution — which is the only test that would
  speak to the portability it exists for. What a package looks like in an
  application menu is therefore still unobserved. See
  [packaging/README.md](packaging/README.md) for the AppStream warning that is
  deliberate.
- egui 0.36 postdates this assistant's training data, and every API used here was
  read from docs.rs or the crate source before use. Two panel APIs changed in
  0.36 (`SidePanel`/`TopBottomPanel` were replaced by a unified `Panel`).

## API notes

Seventeen behaviours were established by probing live instances, and each one cost
a bug on the way:

1. **Pagination uses `page_cursor`, not `page`.** `next_page` is an opaque token
   (`P3381851`). Sending it as `page` fails with HTTP 400
   *"invalid digit found in string"*, because `page` is a numeric page number.
   A `cursor=` parameter is silently ignored. `page_cursor` chains correctly.
2. **Comments have their own sort vocabulary.** `post/list` accepts `Hot`,
   `Active`, `New`, `Old`, `TopDay`…`TopAll`, `MostComments`, `NewComments`,
   `Controversial`, `Scaled`. `comment/list` accepts only `Hot`, `Top`, `New`,
   `Old`, `Controversial` — note `Top`, not `TopAll`. They are separate types
   here so an invalid value cannot be sent.
3. **A post's reported comment count can exceed what its instance will serve.**
   A federated post can report comments the instance cannot fetch; one observed
   post reported 13 and returned none. An empty thread is therefore a normal
   state, not an error, and the UI shows both the rendered count and the
   instance's own figure.
4. **`comment/list` needs `type_=All`.** The parameter looks optional and is
   not: it defaults to `Local`, which hides every comment on a federated post.
   On the same federated posts, omitting it returned **0** comments where
   `type_=All` returned 1, 2, 7 and 8. Local posts are unaffected either way,
   and since the default feed is `All`, omitting it made nearly every thread
   look empty. `Subscribed` could not work without a session in any case.
5. **`limit` is capped at 50.** `comment/list?limit=51` and `limit=100` both
   return HTTP 400 `{"error":"couldnt_get_comments"}`; `limit=50` returns 200.
   Asking for a bigger page therefore loads *nothing*, which is how every thread
   came to show "Could not load comments". The client now clamps every request
   to `MAX_PAGE_SIZE`, and `THREAD_PAGE_SIZE` is derived from it so the two
   cannot drift apart again.
6. **AVIF is served as AVIF, proxy included.** A live post's picture was
   `image/avif` at 47,536 bytes, and `lemmy.ml`'s `image_proxy` passed the same
   bytes and content type through unchanged — so a client without an AVIF decoder
   cannot show it by asking differently, it has to decode AVIF. For one such
   post the thumbnail was the very same AVIF URL, so there was no smaller copy to
   fall back to either. The URL is not the format either: `lemmy.zip`'s proxied
   thumbnails redirect to `images.lemmy.zip`, which **transcodes** them to AVIF
   and answers `image/avif` for a URL naming a `.jpeg` or carrying no extension
   at all — two of two proxied thumbnails on a live feed. Judging AVIF by the
   extension (or the `image` crate by the MIME) missed every one of those, so
   `src/avif.rs` now sniffs the `ftyp` brand in the bytes.
7. **AVIF only turns up in the top-of-day and top-of-week listings.** Scanning
   `New` and `Hot` found no AVIF at all — 300 posts across three instances, on
   two consecutive runs. That reads as a broken format detector, and it is not:
   picture-heavy posts need time to rise. Probing six instances found AVIF under
   `TopDay` (2-3 per instance) and `TopWeek` (3 per instance) on **every one of
   them**, and under `New` and `Hot` on none. The detector and the decoder were
   right the whole time; the scan was pointed at listings AVIF does not appear
   in. This is why the live AVIF test scans four listings rather than two — an
   empty sample is a property of the listing, not of the code.
8. **`thumbnail_url` is not the image.** It is a preview, and how close it comes
   to the original is a per-instance setting. Measured: `lemmy.world` served
   397x512 at the thumbnail URL against **3104x4000** for the picture the post
   itself links to, while `lemmy.ml` served several federated thumbnails at their
   original size (`url == thumbnail_url` for 22 of 42 posts on a sampled page).
   So the feed and the post header stay on the thumbnail — that is what makes a
   page of posts cheap — and the original is fetched only when the picture is
   clicked. `state::best_image_url` picks the post's own image when its URL is an
   image, judged by the `url_content_type` the server sends (a linked article's
   URL is `text/html` while its thumbnail is a JPEG, so the extension alone would
   choose a web page), falling back to the extension for instances that send
   none.

9. **Writes authenticate with a `Bearer` header, not an `auth` field.** The
   0.19.20 server reads the token from `Authorization: Bearer <jwt>`, and the
   request types carry no `auth` field. An unauthenticated write answers HTTP
   **401** with `{"error":"incorrect_login"}` — and so does an expired token,
   which is why 401 is mapped to its own error rather than a generic status: it
   has one remedy, and the UI offers it.
10. **`user/validate_auth` reports the same condition with a different status.**
    With no token it answers HTTP **400** `{"error":"not_logged_in"}`, not 401,
    and success is `{"success": true}`.
11. **`search` has its own type vocabulary, and fills only the array it was
    asked for.** `type_` accepts `All`, `Posts`, `Comments`, `Communities`,
    `Users` and `Url`; the response echoes it back and populates one array.
    Measured on a live lemmy.ml: `type_=Posts` returned 5 posts and 0 of
    everything else, while `type_=All` returned 3 of each. So `type_` is a type
    here rather than free text, and only the matching array is rendered.
12. **`search`'s `listing_type` is a different parameter from its `type_`.**
    Separate names, separate vocabularies, and both are sent.
13. **An empty `q` is not an error — it is the whole instance.** A live
    `search?q=&type_=All` returned comments rather than refusing, so the client
    refuses an empty query itself instead of letting a stray press scan the
    instance.
14. **A vote's `score` is `-1`, `0` or `1`, and `0` removes the vote.** It is the
    state to end in, not a delta. `my_vote` is the other half: `Option<i16>`,
    omitted entirely when there is no vote or nobody is signed in — so a missing
    field must never be read as a neutral vote.
15. **An anonymous post score is the vote-less score.** That is what makes the
    arithmetic work: with `base` the score in an anonymous response, casting a
    vote of `s` yields `base + s`, so withdrawing it (`s = 0`) returns the total
    exactly where it started rather than one short.16. **`$auto` resolves to `libc6` alone.** `cargo-deb` derives dependencies from
   `ldd`, and the built binary links only `libc`, `libm` and `libgcc_s`. The
   graphics and windowing libraries are `dlopen`ed at run time and so never
   reach a dependency list — measured on the package built here.
17. **`community/list` defaults to newest-first, and `search` ranks communities
   with the post `sort`.** Two harmless-looking defaults that together make a
   community browser useless. Without `sort`, `community/list` returned the
   fifty newest communities — 31 with no posts at all, `!technology` and
   `!linux` absent — so clicking one showed an empty feed. And `q=linux`
   ordered by `Hot` led with a six-subscriber `!fedora` and never listed
   `!linux` (68,166). `sort=TopAll` fixes both. Two more traps in the same area:
   `search` with a `community_id` that cannot contain the query returns **empty
   arrays for every type**, so a search must never quietly inherit a feed's
   scope; `listing_type` is not decorative either — `Local` returns only the
   configured instance's own communities — so search always sends `All`; and
   `q=!technology` matches nothing, so a leading `!` has to be stripped before a
   lookup.

Requests carry a descriptive `User-Agent`, and the app fetches only on demand —
there is no background polling, and a vote or a comment leaves only when the
user asks for it.

## Licence

AGPL-3.0-or-later for this project's own code.

Two dependencies narrow it as *distributed*: `zenavif` and the `rav1d-safe` AV1
decoder beneath it are AGPL-3.0-only (or commercial), so a build including the
AVIF decoder can be offered under AGPL-3.0 but not "or later" — no later
version can be granted for that part. `zenavif-parse` is MPL-2.0, whose
file-level copyleft does not extend to this project's own files. Everything else
is permissive (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0) with a permissive
option available in each case; the C route to AVIF would have been
BSD-2-Clause, at the cost of the system-package build requirement described
above.
