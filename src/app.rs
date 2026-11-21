//! The application: state, lifecycle and the `eframe::App` entry point.
//!
//! Everything the UI needs to render lives on [`Limonada`]. The per-frame flow
//! is deliberately simple:
//!
//! 1. **Poll** — take any results that finished on worker threads.
//! 2. **Render** — draw from current state; never block on the network.
//! 3. **Apply** — act on the actions the render pass recorded (open a post,
//!    refresh, change scope), so rendering never mutates state mid-draw.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;

use crate::api::{
    ApiError, FixtureTransport, HttpTransport, LemmyClient, ListingType, RefusingTransport, Sort,
    Transport,
};
use crate::state::{
    Auth, COMMUNITY_PAGE_SIZE, Communities, CommunityScope, Detail, Feed, Search, Session, Settings,
    SiteState, Voted, Viewer, Votes, config_path, diagnostics_enabled, env_flag, fixtures_dir,
};
use crate::time;
use crate::ui;

/// Which screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Feed,
    Post,
    Search,
}

/// A transient message for the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub is_error: bool,
}

impl Status {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

pub struct Limonada {
    pub settings: Settings,
    config_path: Option<PathBuf>,
    /// Snapshot of what is on disk, so saving happens only on real change.
    saved_settings: Settings,
    pub client: LemmyClient,
    pub site: SiteState,
    pub feed: Feed,
    pub communities: Communities,
    pub detail: Option<Detail>,
    /// The search screen's query, results and request.
    pub search: Search,
    /// The sign-in form and its request. Empty of any password while idle.
    pub auth: Auth,
    /// The one in-flight vote.
    pub votes: Votes,
    /// The post's picture opened at full resolution, drawn over everything else.
    pub image: Option<Viewer>,
    pub view: View,
    pub sidebar_open: bool,
    pub settings_open: bool,
    /// Whether the sign-in panel is showing.
    pub auth_open: bool,
    pub status: Option<Status>,
    /// Shared Markdown cache, so images and layout are not rebuilt per frame.
    pub markdown: egui_commonmark::CommonMarkCache,
    /// Current Unix time, refreshed once per frame instead of per row.
    pub now: i64,
}

impl Limonada {
    /// Build the real application window.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Required for post thumbnails and images inside Markdown bodies.
        egui_extras::install_image_loaders(&cc.egui_ctx);
        // After egui_extras on purpose: egui asks the most recently added loader
        // first, and AVIF is this one's to decode — egui_extras' `image`-crate
        // loader has no AVIF decoder and would fail on it.
        crate::avif::install(&cc.egui_ctx);

        let settings = Settings::load(config_path().as_deref());
        settings.theme.apply(&cc.egui_ctx);

        let mut app = Self::assemble(settings);
        app.reload_all(&cc.egui_ctx);
        app
    }

    /// Build the state without an eframe context, for headless use.
    ///
    /// Deliberately installs no image loaders, so nothing in a headless session
    /// can reach the network. That is what lets `tests/ui_render.rs` render real
    /// captured content offline.
    pub fn headless(settings: Settings) -> Self {
        Self::assemble(settings)
    }

    fn assemble(settings: Settings) -> Self {
        let client = Self::build_client(&settings);
        Self::with_client(settings, client)
    }

    /// Build the state around an already-chosen client.
    ///
    /// This is the seam tests use to drive an authenticated flow with no
    /// network: they hand in a client over a stub transport, and everything
    /// above the transport is exactly what ships.
    pub fn headless_with_client(settings: Settings, client: LemmyClient) -> Self {
        Self::with_client(settings, client)
    }

    fn with_client(settings: Settings, client: LemmyClient) -> Self {
        // Hand the saved session to the transport before the first request, so
        // a restart resumes a signed-in view rather than an anonymous one.
        client.set_token(settings.active_session().map(|session| session.token.clone()));
        Self {
            saved_settings: settings.clone(),
            settings,
            config_path: config_path(),
            client,
            site: SiteState::default(),
            feed: Feed::default(),
            communities: Communities::default(),
            detail: None,
            search: Search::new(),
            auth: Auth::default(),
            votes: Votes::default(),
            image: None,
            view: View::Feed,
            sidebar_open: true,
            settings_open: false,
            auth_open: false,
            status: None,
            markdown: egui_commonmark::CommonMarkCache::default(),
            now: time::now_secs(),
        }
    }

    /// Build a client for the configured settings.
    ///
    /// An unusable instance URL yields a [`RefusingTransport`] rather than a
    /// failure here, so the error appears in the ordinary error states.
    fn build_client(settings: &Settings) -> LemmyClient {
        if settings.demo {
            let transport: Arc<dyn Transport> = Arc::new(FixtureTransport::new(fixtures_dir()));
            LemmyClient::new(transport)
        } else {
            match HttpTransport::new(&settings.normalised_instance()) {
                Ok(transport) => LemmyClient::new(Arc::new(transport)),
                Err(err) => LemmyClient::new(Arc::new(RefusingTransport::new(err.to_string()))),
            }
        }
    }

    /// Discard everything and load afresh. Used at startup and whenever the
    /// instance or demo setting changes.
    pub fn reload_all(&mut self, ctx: &egui::Context) {
        // A token is issued by one instance and rejected by every other, so
        // pointing the app elsewhere signs out rather than sending it somewhere
        // it cannot work.
        if self.settings.session.is_some() && self.settings.active_session().is_none() {
            self.settings.session = None;
            self.status =
                Some(Status::info("Signed out: that session belongs to a different instance."));
        }

        self.client = Self::build_client(&self.settings);
        self.client
            .set_token(self.settings.active_session().map(|session| session.token.clone()));
        self.site = SiteState::default();
        self.feed = Feed::new(self.settings.sort, self.settings.listing);
        self.communities = Communities::default();
        self.detail = None;
        self.search.clear_results();
        self.search.listing = ListingType::All;
        self.auth.reset();
        self.close_image(ctx);
        self.view = View::Feed;

        if self.settings.demo {
            // Demo mode reads local fixtures, so the state can be filled in
            // immediately: the first frame shows real captured content instead
            // of a spinner. That also makes the app screenshot-able through
            // eframe's single-frame render path, which only reaches frame two.
            if let Err(err) = self.prime_blocking() {
                self.status = Some(Status::error(format!("Demo fixtures failed: {err}")));
            }
        } else {
            self.site.refresh(ctx, &self.client);
            self.feed.refresh(ctx, &self.client);
            // The sidebar is a directory of communities to scope the feed to,
            // so it always lists all of them — `Subscribed` is a choice about
            // the feed's listing, made in the feed's own control and honoured
            // there with the session's token.
            self.communities.refresh(ctx, &self.client, ListingType::All);
        }

        self.open_startup_post();
        self.open_startup_picture();
    }

    /// Open the open post's picture at startup when `LIMONADA_OPEN_PICTURE=1`.
    ///
    /// The same kind of aid as `LIMONADA_OPEN_POST`, and needed for the same
    /// reason: the overlay is otherwise reachable only by clicking, so this is
    /// what lets a real launch exercise the picture path — including the real
    /// image loader, which the headless render tests leave out on purpose.
    /// Unset by default.
    fn open_startup_picture(&mut self) {
        if !env_flag("LIMONADA_OPEN_PICTURE") {
            return;
        }
        if !self.show_open_post_image() {
            eprintln!("limonada: LIMONADA_OPEN_PICTURE=1, but the open post has no picture");
        }
    }

    /// Open a post at startup when `LIMONADA_OPEN_POST=<id>` is set.
    ///
    /// A demo and diagnostic aid: it makes one specific thread reachable
    /// without a click, which is what lets the render proof capture a thread
    /// rather than only the feed. Unset by default.
    fn open_startup_post(&mut self) {
        let Ok(value) = std::env::var("LIMONADA_OPEN_POST") else {
            return;
        };
        match value.trim().parse::<i64>() {
            Ok(id) => {
                if let Err(err) = self.open_post_blocking(id) {
                    self.status = Some(Status::error(format!(
                        "Could not open post {id} at startup: {err}"
                    )));
                }
            }
            Err(_) => {
                eprintln!("limonada: LIMONADA_OPEN_POST={value:?} is not a post id");
            }
        }
    }

    /// Take any results that have arrived. Never blocks.
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.feed.poll() {
            self.status = Some(Status::error(message));
        }
        self.communities.poll();
        self.communities.tick(ctx, &self.client);
        self.site.poll();
        self.search.poll();

        // A posted comment reports back as a message rather than a phase: the
        // thread is still on screen either way, and on success it is reloaded
        // in place to place the new comment where the server sorted it.
        let posted = match &mut self.detail {
            Some(detail) => detail.poll(ctx, &self.client),
            None => None,
        };
        if let Some(message) = posted {
            self.status = Some(Status::info(message));
        }

        if let Some(theme) = self.auth.poll() {
            match theme {
                Ok(session) => self.adopt_session(ctx, session),
                Err(message) => self.status = Some(Status::error(message)),
            }
        }

        if let Some((label, result)) = self.votes.poll() {
            self.apply_vote(label, result);
        }

        // Write settings through on change. Comparing against a snapshot means
        // no disk traffic on frames where nothing changed.
        if self.settings != self.saved_settings {
            match self.settings.save(self.config_path.as_deref()) {
                Ok(()) => self.saved_settings = self.settings.clone(),
                Err(err) => {
                    self.status = Some(Status::error(format!(
                        "Could not save settings: {err}"
                    )));
                    // Do not retry every frame; surface it once.
                    self.saved_settings = self.settings.clone();
                }
            }
        }

        // Relative ages drift, so repaint at least once a second even when the
        // user is idle.
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    /// Install a session: keep it, hand the token to the client, and reload
    /// what is on screen so it reflects the signed-in account.
    fn adopt_session(&mut self, ctx: &egui::Context, session: Session) {
        self.client.set_token(Some(session.token.clone()));
        let username = session.username.clone();
        self.settings.session = Some(session);
        self.auth_open = false;
        self.status = Some(Status::info(format!("Signed in as {username}.")));
        // Votes and the subscribed listing need the session, so replace the
        // anonymous copy rather than leaving it to be voted against.
        self.refresh_current(ctx);
    }

    /// Sign out, discarding the stored token.
    pub fn sign_out(&mut self, ctx: &egui::Context) {
        self.client.set_token(None);
        self.settings.session = None;
        self.auth.reset();
        self.status = Some(Status::info("Signed out."));
        // The same reload, in the other direction: the rows on screen may still
        // carry this account's own votes.
        self.refresh_current(ctx);
    }

    /// The signed-in account's handle, if there is one.
    pub fn signed_in_as(&self) -> Option<&str> {
        self.settings
            .active_session()
            .map(|session| session.username.as_str())
    }

    /// Vote on a post. Ignored when nobody is signed in, which the UI prevents,
    /// and when a vote is already in flight.
    pub fn cast_post_vote(&mut self, ctx: &egui::Context, post_id: i64, score: i16) {
        if self.settings.active_session().is_none() || self.votes.is_busy() {
            return;
        }
        self.votes.cast_post(ctx, &self.client, post_id, score);
    }

    /// Vote on a comment. See [`Self::cast_post_vote`].
    pub fn cast_comment_vote(&mut self, ctx: &egui::Context, comment_id: i64, score: i16) {
        if self.settings.active_session().is_none() || self.votes.is_busy() {
            return;
        }
        self.votes
            .cast_comment(ctx, &self.client, comment_id, score);
    }

    /// Apply a finished vote to whichever view holds the row, or report why not.
    fn apply_vote(&mut self, label: String, result: Result<Voted, ApiError>) {
        match result {
            Ok(Voted::Post(view)) => {
                self.feed.replace_post(view.clone());
                if let Some(detail) = &mut self.detail {
                    detail.replace_post(view);
                }
            }
            Ok(Voted::Comment(view)) => {
                if let Some(detail) = &mut self.detail {
                    detail.replace_comment(view);
                }
            }
            Err(ApiError::Unauthorized { .. }) => {
                // The token the session was built from is no longer accepted, so
                // drop it rather than repeating the same failing request.
                self.client.set_token(None);
                self.settings.session = None;
                self.status = Some(Status::error(format!(
                    "Your session is no longer accepted ({label}) — sign in again."
                )));
            }
            Err(err) => {
                self.status = Some(Status::error(format!("Could not vote on {label}: {err}")));
            }
        }
    }

    /// Open the search screen; the query box is focused by the user.
    pub fn open_search(&mut self) {
        self.view = View::Search;
        // Always the whole federation this instance knows, never the feed's
        // listing: `listing_type=Local` returned only local communities, so a
        // search for a well known community elsewhere came back empty.
        self.search.listing = ListingType::All;
        // Deliberately *not* inheriting the feed's community scope. That copy
        // looked like a convenience and was a trap: a live lemmy.ml 0.19.20
        // answers `q=technology&type_=All&community_id=<another community>`
        // with zero posts, zero comments, zero communities and zero users, so
        // after clicking any community a search for a well known one came back
        // as a clean "no results". Narrowing is now something the search panel
        // offers explicitly, with the toggle visible beside the results.
        self.search.community_id = None;
    }

    /// Leave the search screen.
    pub fn close_search(&mut self) {
        self.view = View::Feed;
    }

    /// Open a post, seeding the header with the row that was clicked so the
    /// view does not flash empty.
    pub fn open_post(
        &mut self,
        ctx: &egui::Context,
        post_id: i64,
        seed: Option<crate::api::types::PostView>,
    ) {
        // A picture from the post being left must not hang over the new one.
        self.close_image(ctx);
        self.detail = Some(Detail::open(
            ctx,
            &self.client,
            post_id,
            self.settings.comment_sort,
            seed,
        ));
        self.view = View::Post;
    }

    pub fn close_post(&mut self, ctx: &egui::Context) {
        self.close_image(ctx);
        self.detail = None;
        self.view = View::Feed;
    }

    /// Open the picture of the post on screen, at the best resolution it offers.
    /// Returns false when that post has no picture, so a caller can say so
    /// rather than leaving a click with no effect.
    pub fn show_open_post_image(&mut self) -> bool {
        let viewer = self
            .detail
            .as_ref()
            .and_then(|detail| detail.post.as_ref())
            .and_then(Viewer::for_post);
        let opened = viewer.is_some();
        self.image = viewer;
        if let Some(viewer) = self.image.as_ref().filter(|_| diagnostics_enabled()) {
            eprintln!(
                "limonada: picture {} ({}{}) — {}",
                viewer.url,
                viewer.format.as_deref().unwrap_or("unknown format"),
                if viewer.decodable {
                    ""
                } else {
                    ", no decoder in this build"
                },
                viewer.resolution_note()
            );
        }
        opened
    }

    /// Dismiss the open picture, releasing the texture it added.
    pub fn close_image(&mut self, ctx: &egui::Context) {
        // Only the copy this opened, never the one the page is still showing.
        if let Some(url) = self.image.as_ref().and_then(Viewer::texture_to_release) {
            ctx.forget_image(url);
        }
        self.image = None;
    }

    /// Scope the feed to one community, or clear the scope with `None`.
    pub fn set_scope(&mut self, ctx: &egui::Context, scope: Option<CommunityScope>) {
        self.feed.scope = scope;
        // A search's narrowing belongs to that search, so changing which
        // community the feed shows never re-scopes one silently.
        self.search.community_id = None;
        self.view = View::Feed;
        self.feed.refresh(ctx, &self.client);
    }

    /// Refresh whichever screen is showing.
    pub fn refresh_current(&mut self, ctx: &egui::Context) {
        match self.view {
            View::Feed => {
                self.feed.refresh(ctx, &self.client);
                self.site.refresh(ctx, &self.client);
            }
            View::Post => {
                if let Some(detail) = &mut self.detail {
                    detail.reload(ctx, &self.client);
                }
            }
            View::Search => self.search.submit(ctx, &self.client),
        }
    }

    /// Load the site, first feed page and community list synchronously.
    ///
    /// The UI never calls this; it exists so tests (and any future offline demo)
    /// can reach a fully populated state without driving the async machinery.
    pub fn prime_blocking(&mut self) -> Result<(), crate::api::ApiError> {
        // The same three lines the async path prints when a load completes
        // (`state.rs`), because `LIMONADA_LOG=1` promises one line per completed
        // load and demo mode completes its loads here instead of in a worker.
        // Without these it was the one mode where the flag stayed silent.
        let info = self.client.site()?;
        if diagnostics_enabled() {
            eprintln!(
                "limonada: connected to {} (lemmy {}), {} users",
                info.site_view.site.name, info.version, info.site_view.counts.users
            );
        }
        self.site.info = Some(info);
        self.site.phase = crate::state::Phase::Ready;

        let page = self.client.list_posts(
            self.feed.sort,
            self.feed.listing,
            None,
            crate::api::DEFAULT_PAGE_SIZE,
        )?;
        if diagnostics_enabled() {
            eprintln!(
                "limonada: feed loaded {} posts ({} total, more: {})",
                page.posts.len(),
                page.posts.len(),
                page.next_page.is_some()
            );
        }
        self.feed.items = page.posts;
        self.feed.next_page = page.next_page;
        self.feed.phase = crate::state::Phase::Ready;

        // The same ordering the sidebar asks for, so the printed picture
        // matches what the window shows.
        let communities = self
            .client
            .list_communities(ListingType::All, Sort::TopAll, COMMUNITY_PAGE_SIZE)?;
        if diagnostics_enabled() {
            eprintln!("limonada: loaded {} communities", communities.communities.len());
        }
        self.communities.items = communities.communities;
        self.communities.phase = crate::state::Phase::Ready;
        Ok(())
    }

    /// Run the current search synchronously. See [`Self::prime_blocking`].
    ///
    /// The async path is what ships; this exists so a headless test can reach a
    /// rendered page of search results without driving the worker threads.
    pub fn search_blocking(&mut self) -> Result<(), crate::api::ApiError> {
        let query = self.search.query.trim().to_string();
        let response = self.client.search(
            &query,
            self.search.type_,
            self.search.sort,
            self.search.listing,
            self.search.community_id,
            crate::state::SEARCH_PAGE_SIZE,
        )?;
        self.search.posts = response.posts;
        self.search.comments = response.comments;
        self.search.communities = response.communities;
        self.search.users = response.users;
        self.search.shown_type = response.type_;
        self.search.attempted = true;
        self.search.phase = crate::state::Phase::Ready;
        Ok(())
    }

    /// Open a post and its thread synchronously. See [`Self::prime_blocking`].
    pub fn open_post_blocking(&mut self, post_id: i64) -> Result<(), crate::api::ApiError> {
        let detail = self.client.get_post(post_id)?;
        let comments = self.client.list_comments(
            post_id,
            self.settings.comment_sort,
            crate::state::THREAD_PAGE_SIZE,
        )?;
        let threads = crate::tree::build_threads(&comments.comments);
        // Same wording as `Detail::poll`'s diagnostic, so a `LIMONADA_LOG=1`
        // launch reports the thread whether it was opened asynchronously or
        // through this blocking path.
        if diagnostics_enabled() {
            eprintln!(
                "limonada: thread for post {post_id} has {} comments in {} roots (limit {})",
                comments.comments.len(),
                threads.len(),
                crate::state::THREAD_PAGE_SIZE,
            );
        }
        self.detail = Some(Detail::ready(
            detail.post_view,
            threads,
            self.settings.comment_sort,
        ));
        self.view = View::Post;
        Ok(())
    }

    /// Draw one frame.
    ///
    /// Separate from the `eframe::App` impl so a test can drive it with a bare
    /// `egui::Context`, which needs no window or GPU.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.now = time::now_secs();
        self.poll(&ctx);

        // Panel order matters: side and bar panels must be added before the
        // central panel, and egui requires CentralPanel to come last.
        ui::topbar::show(self, ui);

        if self.sidebar_open {
            ui::communities::show(self, ui);
        }

        ui::statusbar::show(self, ui);

        if self.settings_open {
            ui::settings::show(self, ui);
        }

        if self.auth_open {
            ui::auth::show(self, ui);
        }

        ui::central::show(self, ui);

        // Last, so the picture sits above every panel — including one the user
        // opened while it was up.
        ui::image_viewer::show(self, &ctx);
    }

    /// Status-bar text describing where responses come from.
    pub fn source_label(&self) -> String {
        self.client.transport().description()
    }

    /// Whether the signed-in session belongs to the configured instance.
    pub fn signed_in(&self) -> bool {
        self.settings.active_session().is_some()
    }

    /// Where settings are stored, for display.
    pub fn settings_path_display(&self) -> Option<String> {
        self.config_path
            .as_ref()
            .map(|path| path.display().to_string())
    }
}

impl eframe::App for Limonada {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

/// The fixture directory reported in the settings panel, for transparency about
/// where demo data comes from.
pub fn demo_fixture_dir() -> PathBuf {
    fixtures_dir()
}
