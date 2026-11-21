//! Application state: settings, and the three asynchronously loaded resources.
//!
//! Each resource follows the same shape — a [`Phase`], the data itself, and at
//! most one in-flight [`Request`]. The UI polls those requests once per frame
//! and never blocks.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::Context;
use serde::{Deserialize, Serialize};

use crate::api::job::{Request, spawn};
use crate::api::types::{
    CommentResponse, CommentView, CommunityView, GetPostResponse, GetSiteResponse,
    ListCommentsResponse, ListCommunitiesResponse, ListPostsResponse, LoginResponse, PersonView,
    PostView, SearchResponse,
};
use crate::api::{
    ApiError, CommentSort, DEFAULT_PAGE_SIZE, LemmyClient, ListingType, MAX_PAGE_SIZE, SearchType,
    Sort,
};
use crate::tree::{ThreadNode, build_threads};

/// How many comments to request for a thread. Deep threads are truncated by the
/// server regardless; `build_threads` keeps anything orphaned by that.
///
/// Pinned to [`MAX_PAGE_SIZE`] rather than a number of its own, because asking
/// for **more** than the server allows does not load a bigger thread, it loads
/// none: a live lemmy.ml 0.19.20 answers `limit=51` and `limit=100` with HTTP
/// 400 `{"error":"couldnt_get_comments"}`. Tying the constant to the cap means
/// this cannot silently drift above it again.
pub const THREAD_PAGE_SIZE: u32 = MAX_PAGE_SIZE;

/// How many results to ask for per search. Modest on purpose: a search is a
/// scan of the instance, so it is the cheapest useful page rather than the
/// largest one.
pub const SEARCH_PAGE_SIZE: u32 = 20;

/// How many communities the sidebar lists, and how many instance-wide matches
/// its lookup asks for. One page each: the sidebar is a directory for scoping
/// the feed, not an index of every community an instance knows.
pub const COMMUNITY_PAGE_SIZE: u32 = 50;

/// How long the sidebar filter must hold still before its lookup is sent.
/// Typing is bursty; a request per keystroke would be one per letter.
pub const COMMUNITY_FIND_DELAY: Duration = Duration::from_millis(300);

/// The shortest filter worth asking the instance about. One letter matches
/// hundreds of communities and is almost always a keystroke on the way to a
/// word.
pub const COMMUNITY_FIND_MIN_CHARS: usize = 2;

/// Where a loadable resource stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(String),
}

impl Phase {
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message),
            _ => None,
        }
    }
}

/// Which colour scheme to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }

    /// Apply to an egui context.
    pub fn apply(self, ctx: &Context) {
        ctx.set_theme(self.preference());
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// A signed-in session, persisted between runs.
///
/// Only the token is a secret; it is never logged, and the settings file it
/// lives in is written owner-only. Everything else here is for display and for
/// deciding whether the token still belongs to the configured instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// The instance that issued the token. A token is only valid where it was
    /// issued, so it is stored with the token and checked before reuse.
    pub instance_url: String,
    /// The account handle, for the status bar.
    pub username: String,
    /// The session token (`jwt`). A secret.
    pub token: String,
}

/// Everything that survives a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub instance_url: String,
    /// Serve responses from `fixtures/` instead of the network.
    #[serde(default)]
    pub demo: bool,
    pub sort: Sort,
    pub listing: ListingType,
    /// Comments use their own sort vocabulary — see [`CommentSort`].
    pub comment_sort: CommentSort,
    pub theme: Theme,
    /// The signed-in session, if any. `None` means browsing anonymously, which
    /// is the only mode that works everywhere and needs no account.
    #[serde(default)]
    pub session: Option<Session>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            instance_url: "https://lemmy.ml".to_string(),
            demo: env_flag("LIMONADA_DEMO"),
            sort: Sort::Hot,
            listing: ListingType::All,
            comment_sort: CommentSort::Top,
            theme: Theme::System,
            session: None,
        }
    }
}

impl Settings {
    /// Read settings, falling back to defaults when the file is absent or
    /// unreadable. A corrupt config must never stop the app from starting.
    pub fn load(path: Option<&Path>) -> Self {
        let mut settings = Self::read(path);
        // The environment wins over the saved file, so `LIMONADA_DEMO=1 app`
        // always runs in demo mode whatever was persisted.
        if std::env::var("LIMONADA_DEMO").is_ok() {
            settings.demo = env_flag("LIMONADA_DEMO");
        }
        settings
    }

    fn read(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(settings) => settings,
            Err(err) => {
                eprintln!(
                    "limonada: ignoring unreadable settings at {} ({err})",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Persist settings, creating the parent directory if needed.
    pub fn save(&self, path: Option<&Path>) -> Result<(), String> {
        let Some(path) = path else {
            return Err("no settings path is available on this platform".into());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;

        // These may hold a session token, so keep the file for its owner only.
        // Best effort: a platform without Unix permissions is not a failure,
        // and neither is a filesystem that ignores the mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// The instance URL to actually use, trimmed of whitespace and slashes.
    pub fn normalised_instance(&self) -> String {
        self.instance_url.trim().trim_end_matches('/').to_string()
    }

    /// The session, but only while it belongs to the configured instance.
    ///
    /// A token is issued by one instance and rejected by every other, so a
    /// stored session that no longer matches is treated as absent rather than
    /// sent somewhere it cannot work.
    pub fn active_session(&self) -> Option<&Session> {
        self.session
            .as_ref()
            .filter(|session| session.instance_url == self.normalised_instance())
    }
}

/// Where Limonada keeps `settings.json`.
pub fn config_path() -> Option<PathBuf> {
    eframe::storage_dir("limonada").map(|dir| dir.join("settings.json"))
}

/// The directory holding the captured API fixtures used by demo mode.
pub fn fixtures_dir() -> PathBuf {
    match std::env::var("LIMONADA_FIXTURES") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
    }
}

/// True when an environment variable is set to something truthy.
pub(crate) fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Feed
// ---------------------------------------------------------------------------

/// Restricts the feed to a single community.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityScope {
    pub id: i64,
    pub name: String,
}

/// The post feed, with its paging cursor.
#[derive(Default)]
pub struct Feed {
    pub items: Vec<PostView>,
    pub next_page: Option<String>,
    pub phase: Phase,
    pub sort: Sort,
    pub listing: ListingType,
    /// `Some` when the feed is limited to one community.
    pub scope: Option<CommunityScope>,
    request: Option<Request<ListPostsResponse>>,
    /// True when the in-flight request extends the list rather than replacing.
    appending: bool,
}

impl Feed {
    pub fn new(sort: Sort, listing: ListingType) -> Self {
        Self {
            sort,
            listing,
            ..Default::default()
        }
    }

    pub fn is_busy(&self) -> bool {
        self.request.is_some()
    }

    pub fn can_load_more(&self) -> bool {
        self.next_page.is_some() && self.request.is_none()
    }

    /// Heading for the feed, naming the community when scoped to one.
    pub fn scope_label(&self) -> String {
        match &self.scope {
            Some(scope) => format!("!{}", scope.name),
            None => self.listing.label().to_string(),
        }
    }

    /// Load page one, replacing what is on screen.
    pub fn refresh(&mut self, ctx: &Context, client: &LemmyClient) {
        self.appending = false;
        self.phase = Phase::Loading;
        self.next_page = None;
        let (sort, listing, scope, client) = (
            self.sort,
            self.listing,
            self.scope.as_ref().map(|s| s.id),
            client.clone(),
        );
        self.request = Some(spawn(ctx.clone(), move || {
            match scope {
                Some(id) => client.list_posts_in_community(sort, id, None, DEFAULT_PAGE_SIZE),
                None => client.list_posts(sort, listing, None, DEFAULT_PAGE_SIZE),
            }
        }));
    }

    /// Append the next page.
    pub fn load_more(&mut self, ctx: &Context, client: &LemmyClient) {
        let Some(cursor) = self.next_page.clone() else {
            return;
        };
        if self.request.is_some() {
            return;
        }
        self.appending = true;
        if self.items.is_empty() {
            self.phase = Phase::Loading;
        }
        let (sort, listing, scope, client) = (
            self.sort,
            self.listing,
            self.scope.as_ref().map(|s| s.id),
            client.clone(),
        );
        self.request = Some(spawn(ctx.clone(), move || {
            match scope {
                Some(id) => {
                    client.list_posts_in_community(sort, id, Some(&cursor), DEFAULT_PAGE_SIZE)
                }
                None => client.list_posts(sort, listing, Some(&cursor), DEFAULT_PAGE_SIZE),
            }
        }));
    }

    /// Replace a post already in the list with the server's updated copy.
    ///
    /// Used after a vote: the row then shows the instance's own new score and
    /// the user's own vote rather than a locally guessed one.
    pub fn replace_post(&mut self, view: PostView) {
        if let Some(slot) = self
            .items
            .iter_mut()
            .find(|row| row.post.id == view.post.id)
        {
            *slot = view;
        }
    }

    /// Apply a finished request. Returns a transient, non-fatal message (used
    /// when appending fails, where blanking the feed would be wrong).
    pub fn poll(&mut self) -> Option<String> {
        let result = self.request.as_ref()?.poll()?;
        self.request = None;
        let was_appending = std::mem::take(&mut self.appending);

        match result {
            Ok(page) => {
                let received = page.posts.len();
                if was_appending {
                    let known: HashSet<i64> = self.items.iter().map(|v| v.post.id).collect();
                    self.items
                        .extend(page.posts.into_iter().filter(|v| !known.contains(&v.post.id)));
                } else {
                    self.items = page.posts;
                }
                self.next_page = page.next_page;
                self.phase = Phase::Ready;
                // Opt-in diagnostics, so a run from a terminal reports what it
                // actually loaded.
                if diagnostics_enabled() {
                    eprintln!(
                        "limonada: feed loaded {received} posts ({} total, more: {})",
                        self.items.len(),
                        self.next_page.is_some()
                    );
                }
                None
            }
            Err(err) => {
                if was_appending {
                    // Keep the posts already on screen.
                    Some(err.to_string())
                } else {
                    self.phase = Phase::Failed(err.to_string());
                    None
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Post detail
// ---------------------------------------------------------------------------

/// One post and its comment thread.
pub struct Detail {
    pub post_id: i64,
    pub post: Option<PostView>,
    pub comments: Vec<ThreadNode>,
    pub collapsed: HashSet<i64>,
    pub post_phase: Phase,
    pub thread_phase: Phase,
    pub comment_sort: CommentSort,
    post_request: Option<Request<GetPostResponse>>,
    comments_request: Option<Request<ListCommentsResponse>>,
    /// The open reply editor's target: `None` is no editor, `Some(None)` is a
    /// new top-level comment, and `Some(Some(id))` is a reply to that comment.
    pub reply_to: Option<Option<i64>>,
    /// The text being written. Kept when the editor is re-targeted, so moving
    /// from one comment's reply to another does not throw the draft away.
    pub reply_draft: String,
    pub reply_phase: Phase,
    reply_request: Option<Request<CommentResponse>>,
}

impl Detail {
    /// Open a post. `seed` is the feed row that was clicked, so the header can
    /// render immediately instead of flashing empty while the request runs.
    pub fn open(
        ctx: &Context,
        client: &LemmyClient,
        post_id: i64,
        comment_sort: CommentSort,
        seed: Option<PostView>,
    ) -> Self {
        let mut detail = Self {
            post_id,
            post: seed,
            comments: Vec::new(),
            collapsed: HashSet::new(),
            post_phase: Phase::Loading,
            thread_phase: Phase::Loading,
            comment_sort,
            post_request: None,
            comments_request: None,
            reply_to: None,
            reply_draft: String::new(),
            reply_phase: Phase::Idle,
            reply_request: None,
        };
        detail.reload(ctx, client);
        detail
    }

    /// A detail view that is already fully loaded.
    ///
    /// Used by [`crate::app::Limonada::open_post_blocking`] so tests (and any
    /// future offline demo) can reach a rendered thread without async.
    pub fn ready(post: PostView, comments: Vec<ThreadNode>, comment_sort: CommentSort) -> Self {
        Self {
            post_id: post.post.id,
            post: Some(post),
            comments,
            collapsed: HashSet::new(),
            post_phase: Phase::Ready,
            thread_phase: Phase::Ready,
            comment_sort,
            post_request: None,
            comments_request: None,
            reply_to: None,
            reply_draft: String::new(),
            reply_phase: Phase::Idle,
            reply_request: None,
        }
    }

    /// Re-fetch both the post and its thread.
    pub fn reload(&mut self, ctx: &Context, client: &LemmyClient) {
        self.post_phase = Phase::Loading;
        self.thread_phase = Phase::Loading;

        let (id, client_for_post) = (self.post_id, client.clone());
        self.post_request = Some(spawn(ctx.clone(), move || client_for_post.get_post(id)));

        let (id, sort, client_for_comments) = (self.post_id, self.comment_sort, client.clone());
        self.comments_request = Some(spawn(ctx.clone(), move || {
            client_for_comments.list_comments(id, sort, THREAD_PAGE_SIZE)
        }));
    }

    /// Replace the post header with the server's updated copy (after a vote).
    pub fn replace_post(&mut self, view: PostView) {
        if self.post.as_ref().is_some_and(|seed| seed.post.id == view.post.id) {
            self.post = Some(view);
        }
    }

    /// The author of a comment in the rendered tree, for labelling a reply.
    pub fn comment_author(&self, id: i64) -> Option<&str> {
        self.comments
            .iter()
            .find_map(|root| root.find(id))
            .map(|view| view.creator.label())
    }

    /// Replace one comment in the rendered tree with the server's updated copy.
    ///
    /// A comment can be anywhere in the tree, so this walks it; a comment that
    /// is no longer present (the thread was reloaded under it) is left alone.
    pub fn replace_comment(&mut self, view: CommentView) {
        for root in &mut self.comments {
            if root.replace(&view) {
                break;
            }
        }
    }

    /// Open the reply editor for a new top-level comment (`None`) or a reply to
    /// `parent_id`. Any draft already typed is kept.
    pub fn start_reply(&mut self, parent_id: Option<i64>) {
        self.reply_to = Some(parent_id);
        if self.reply_phase.error().is_some() {
            self.reply_phase = Phase::Idle;
        }
    }

    /// Close the editor and discard the draft.
    pub fn cancel_reply(&mut self) {
        self.reply_to = None;
        self.reply_draft.clear();
        self.reply_phase = Phase::Idle;
    }

    /// Send the draft as a comment on the open editor's target.
    ///
    /// Empty text is refused here rather than sent, because the instance
    /// rejects it and a round trip to be told so is worse than saying it now.
    pub fn submit_reply(&mut self, ctx: &Context, client: &LemmyClient) {
        if self.reply_request.is_some() {
            return;
        }
        let Some(parent_id) = self.reply_to else {
            return;
        };
        let content = self.reply_draft.trim().to_string();
        if content.is_empty() {
            self.reply_phase = Phase::Failed("A comment cannot be empty.".into());
            return;
        }
        let post_id = self.post_id;
        let client = client.clone();
        self.reply_phase = Phase::Loading;
        self.reply_request = Some(spawn(ctx.clone(), move || {
            client.create_comment(&content, post_id, parent_id)
        }));
    }

    /// Whether a comment is currently being sent.
    pub fn reply_is_busy(&self) -> bool {
        self.reply_request.is_some()
    }

    /// Apply finished requests. Returns a transient message when a comment was
    /// posted and the thread was reloaded to place it.
    ///
    /// Unlike [`Feed::poll`] a *load* failure always belongs to one of the two
    /// phases, and both are rendered; only the post-a-comment case reports back
    /// as a transient message, because there the thread is still on screen.
    pub fn poll(&mut self, ctx: &Context, client: &LemmyClient) -> Option<String> {
        if let Some(result) = self.post_request.as_ref().and_then(Request::poll) {
            self.post_request = None;
            match result {
                Ok(response) => {
                    self.post = Some(response.post_view);
                    self.post_phase = Phase::Ready;
                }
                Err(err) => self.post_phase = Phase::Failed(err.to_string()),
            }
        }

        if let Some(result) = self.comments_request.as_ref().and_then(Request::poll) {
            self.comments_request = None;
            match result {
                Ok(response) => {
                    // An empty batch is a normal outcome, not an error: a
                    // federated post can report comments its instance cannot
                    // serve. `comment_count()` returning 0 drives the message.
                    self.comments = build_threads(&response.comments);
                    self.thread_phase = Phase::Ready;
                    if diagnostics_enabled() {
                        eprintln!(
                            "limonada: thread for post {} has {} comments in {} roots",
                            self.post_id,
                            response.comments.len(),
                            self.comments.len()
                        );
                    }
                }
                Err(err) => self.thread_phase = Phase::Failed(err.to_string()),
            }
        }

        self.poll_reply(ctx, client)
    }

    /// Apply a finished comment post, reloading the thread on success so the new
    /// comment appears where the server put it rather than spliced in at a guess.
    fn poll_reply(&mut self, ctx: &Context, client: &LemmyClient) -> Option<String> {
        let result = self.reply_request.as_ref()?.poll()?;
        self.reply_request = None;
        match result {
            Ok(response) => {
                if diagnostics_enabled() {
                    eprintln!(
                        "limonada: posted comment {} on post {}",
                        response.comment_view.comment.id, self.post_id
                    );
                }
                self.reply_to = None;
                self.reply_draft.clear();
                self.reply_phase = Phase::Idle;
                self.reload(ctx, client);
                Some("Comment posted.".to_string())
            }
            Err(err) => {
                self.reply_phase = Phase::Failed(err.to_string());
                None
            }
        }
    }

    /// Total comments in the rendered tree.
    pub fn comment_count(&self) -> usize {
        self.comments
            .iter()
            .map(|root| 1 + root.descendant_count())
            .sum()
    }
}

// ---------------------------------------------------------------------------
// Full-resolution pictures
// ---------------------------------------------------------------------------

/// How the picture viewer scales what it shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Zoom {
    /// As large as the window allows, and never larger than the image itself.
    #[default]
    Fit,
    /// One image pixel per screen pixel, scrolling when the window is too small.
    ///
    /// Needed because Lemmy images are routinely taller than any window: a live
    /// post served 3104x4000, which no screen can show at once.
    Actual,
}

impl Zoom {
    /// The label of the mode this would switch to, so the button reads as the
    /// action it performs rather than as the state it is in.
    pub fn toggled(self) -> Self {
        match self {
            Self::Fit => Self::Actual,
            Self::Actual => Self::Fit,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Fit to window",
            Self::Actual => "Actual size (100%)",
        }
    }
}

/// Extensions treated as images when an instance sends no content type.
const IMAGE_EXTENSIONS: [&str; 8] = [
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".bmp", ".svg",
];

/// Whether a URL points at an image, given the content type sent for it.
///
/// The server's own answer wins when it gives one; the extension is only a
/// fallback for instances that omit `url_content_type` entirely.
pub fn looks_like_image(url: &str, content_type: Option<&str>) -> bool {
    if let Some(content_type) = content_type {
        let content_type = content_type.trim().to_ascii_lowercase();
        if !content_type.is_empty() {
            return content_type.starts_with("image/");
        }
    }
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .any(|extension| path.ends_with(extension))
}

/// The picture the feed row and the post header show.
///
/// Instances treat this as a preview and may cap it far below the original, so
/// it is not "the image": a live lemmy.world post served 397x512 here against
/// 3104x4000 at the URL the post itself links to.
pub fn inline_image_url(view: &PostView) -> Option<&str> {
    view.post
        .thumbnail_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
}

/// The format an image URL names, as a short upper-case word: `"PNG"`, `"AVIF"`.
///
/// From the content type when the server sends one, otherwise from the file
/// extension. Used to tell the user which format this is, so an image that
/// cannot be decoded is named rather than merely missing.
pub fn image_format(url: &str, content_type: Option<&str>) -> Option<String> {
    if let Some(content_type) = content_type {
        let content_type = content_type.trim().to_ascii_lowercase();
        if let Some(subtype) = content_type.strip_prefix("image/") {
            let subtype = subtype.split([';', '+']).next().unwrap_or(subtype).trim();
            if !subtype.is_empty() {
                return Some(normalise_format(subtype));
            }
        }
    }

    let path = url.split(['?', '#']).next().unwrap_or(url);
    let extension = path.rsplit_once('.')?.1;
    // A guess, not a parse: long or non-alphanumeric tails are file names, not
    // formats, and naming one "AVIF" would be worse than naming none.
    if extension.is_empty()
        || extension.len() > 5
        || !extension.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(normalise_format(extension))
}

fn normalise_format(name: &str) -> String {
    match name.to_ascii_uppercase().as_str() {
        "JPG" | "JPE" => "JPEG".to_string(),
        other => other.to_string(),
    }
}

/// Whether this build can decode an image.
///
/// The `image` crate's gif, jpeg, png and webp features, egui_extras' SVG
/// loader, and AVIF — which comes from [`crate::avif`] rather than from that
/// crate, because the only AVIF decoder `image` has needs the C library
/// libdav1d. Anything else (BMP and TIFF, say) decodes to nothing, and egui
/// draws a warning glyph where the picture should be with no error anywhere
/// else. Knowing this in advance is what lets the viewer say why.
pub fn decodes(url: &str, content_type: Option<&str>) -> bool {
    matches!(
        image_format(url, content_type).as_deref(),
        Some("PNG" | "JPEG" | "GIF" | "WEBP" | "SVG" | "AVIF")
    )
}

/// The best picture a post offers, which is what the viewer opens.
///
/// The post's own image when its URL is one, otherwise the thumbnail. The
/// content type sent with `url` decides when present, and it has to: for a
/// linked article the `url` is `text/html` while its thumbnail is a perfectly
/// good JPEG, so choosing `url` unconditionally would draw a web page — or,
/// worse, nothing — in the picture viewer.
pub fn best_image_url(view: &PostView) -> Option<&str> {
    best_image(view).map(|(url, _)| url)
}

/// The best picture a post offers, together with the content type that
/// describes it — the server's own answer for the post's image, or the image
/// details that describe the thumbnail.
fn best_image(view: &PostView) -> Option<(&str, Option<&str>)> {
    let post = &view.post;
    let linked = post
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());
    match linked {
        Some(url) if looks_like_image(url, post.url_content_type.as_deref()) => {
            Some((url, post.url_content_type.as_deref()))
        }
        _ => Some((
            inline_image_url(view)?,
            view.image_details
                .as_ref()
                .and_then(|details| details.content_type.as_deref()),
        )),
    }
}

/// An open picture, drawn over the rest of the interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    /// The image being shown: the post's own image when it has one.
    pub url: String,
    /// What the page behind the overlay shows. Equal to `url` when the instance
    /// offers no larger copy, which the caption states plainly instead of
    /// letting the user believe they are seeing more than they are.
    pub inline_url: Option<String>,
    /// The poster's own description of the picture, when there is one.
    pub alt: Option<String>,
    /// The format the picture is in, for naming it when it cannot be decoded.
    pub format: Option<String>,
    /// Whether this build can decode it at all. False means the viewer will show
    /// an explanation and the browser button rather than a picture.
    pub decodable: bool,
    pub zoom: Zoom,
    /// Whether the size actually served has been reported once, so a
    /// `LIMONADA_LOG=1` run says the picture arrived rather than only that the
    /// overlay opened.
    pub reported_size: bool,
}

impl Viewer {
    /// The viewer for a post, or `None` when it has no picture at all.
    pub fn for_post(view: &PostView) -> Option<Self> {
        let post = &view.post;
        let (url, content_type) = best_image(view)?;

        Some(Self {
            decodable: decodes(url, content_type),
            format: image_format(url, content_type),
            url: url.to_string(),
            inline_url: inline_image_url(view).map(str::to_string),
            alt: post
                .alt_text
                .as_deref()
                .map(str::trim)
                .filter(|alt| !alt.is_empty())
                .map(str::to_string),
            zoom: Zoom::Fit,
            reported_size: false,
        })
    }

    /// Why nothing will appear, when the instance serves a format this build
    /// cannot decode. `None` when the picture will render.
    pub fn undecodable_note(&self) -> Option<String> {
        if self.decodable {
            return None;
        }
        Some(format!(
            "{} cannot be decoded by this build — use Open in browser",
            self.format.as_deref().unwrap_or("this image format")
        ))
    }

    /// Whether opening this shows more than the page already did. False for an
    /// instance whose thumbnail is the original, where the gain is size on
    /// screen rather than resolution.
    pub fn shows_more_than_the_page(&self) -> bool {
        self.inline_url.as_deref() != Some(self.url.as_str())
    }

    /// What to say about how much of the picture this is.
    pub fn resolution_note(&self) -> &'static str {
        match self.inline_url.as_deref() {
            Some(inline) if inline == self.url.as_str() => {
                "the page was already showing the largest copy the instance offers"
            }
            Some(_) => "the page behind is showing a smaller preview of this image",
            None => "the page had no preview of this image",
        }
    }

    /// The texture to release when the viewer closes, if any.
    ///
    /// Only when it is not the image the page itself shows: releasing that one
    /// would blank the picture the user came from until it is fetched again. A
    /// server-side 4K image costs tens of megabytes as a texture, so keeping
    /// every one of them alive would grow the process for as long as the user
    /// browses image posts.
    pub fn texture_to_release(&self) -> Option<&str> {
        self.shows_more_than_the_page()
            .then_some(self.url.as_str())
    }
}

// ---------------------------------------------------------------------------
// Communities
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Communities {
    pub items: Vec<CommunityView>,
    pub phase: Phase,
    pub filter: String,
    request: Option<Request<ListCommunitiesResponse>>,
    /// Matches for [`Self::filter`], asked of the instance rather than found
    /// in [`Self::items`].
    ///
    /// The sidebar holds one page of communities out of the thousands an
    /// instance knows, so filtering that page in memory can never surface a
    /// community outside it. That is exactly the "a well known community does
    /// not show up" failure; this lookup is what fixes it.
    pub found: Vec<CommunityView>,
    /// Where the lookup behind [`Self::found`] stands.
    pub find_phase: Phase,
    /// The text [`Self::found`] answers for; empty when nothing was looked up.
    pub find_query: String,
    /// The query of the lookup currently in flight, so a response can be
    /// checked against the text in the box when it lands.
    find_in_flight: Option<String>,
    /// The text whose typing burst ends at this instant, for the debounce.
    find_due: Option<(String, Instant)>,
    find_request: Option<Request<SearchResponse>>,
}

impl Communities {
    pub fn is_busy(&self) -> bool {
        self.request.is_some()
    }

    /// Whether the instance-wide lookup is running.
    pub fn is_finding(&self) -> bool {
        self.find_phase.is_loading()
    }

    pub fn refresh(&mut self, ctx: &Context, client: &LemmyClient, listing: ListingType) {
        if self.request.is_some() {
            return;
        }
        self.phase = Phase::Loading;
        let client = client.clone();
        self.request = Some(spawn(ctx.clone(), move || {
            // `TopAll`, deliberately: without a sort this endpoint answers
            // newest-first, which is a page of brand-new communities with
            // nothing in them.
            client.list_communities(listing, Sort::TopAll, COMMUNITY_PAGE_SIZE)
        }));
    }

    /// The text the filter is looked up with.
    pub fn lookup_text(&self) -> String {
        community_query(&self.filter)
    }

    /// Ask the instance for communities matching `query`, whatever the loaded
    /// page happens to hold.
    pub fn find(&mut self, ctx: &Context, client: &LemmyClient, query: &str) {
        if self.find_request.is_some() {
            return;
        }
        self.find_in_flight = Some(query.to_string());
        self.find_phase = Phase::Loading;
        let (client, query) = (client.clone(), query.to_string());
        self.find_request = Some(spawn(ctx.clone(), move || {
            client.search(
                &query,
                SearchType::Communities,
                // Ranked by activity, not by a feed's `Hot`: a live instance
                // answered `q=linux` with a six-subscriber `!fedora` first and
                // never listed `!linux` at all, while `TopAll` listed it first.
                Sort::TopAll,
                ListingType::All,
                None,
                COMMUNITY_PAGE_SIZE,
            )
        }));
    }

    /// Debounce the filter and run its lookup once the typing stops.
    pub fn tick(&mut self, ctx: &Context, client: &LemmyClient) {
        let wanted = self.lookup_text();
        if wanted == self.find_query || self.find_in_flight.as_deref() == Some(wanted.as_str()) {
            return;
        }
        if wanted.chars().count() < COMMUNITY_FIND_MIN_CHARS {
            // Too short to be worth asking, so drop results for text that is no
            // longer in the box rather than leaving stale rows under it.
            self.found.clear();
            self.find_query.clear();
            self.find_phase = Phase::Idle;
            self.find_due = None;
            return;
        }
        // One lookup at a time: a newer query waits for the older answer
        // instead of piling requests onto the instance.
        if self.find_request.is_some() {
            return;
        }
        let now = Instant::now();
        let due = match &self.find_due {
            Some((text, at)) if *text == wanted => *at,
            _ => {
                let at = now + COMMUNITY_FIND_DELAY;
                self.find_due = Some((wanted.clone(), at));
                at
            }
        };
        if now < due {
            // Wake up when the burst goes quiet; egui would otherwise wait for
            // the next input event and the lookup would feel stuck.
            ctx.request_repaint_after(due - now);
            return;
        }
        self.find_due = None;
        self.find(ctx, client, &wanted);
    }

    pub fn poll(&mut self) {
        if let Some(result) = self.request.as_ref().and_then(Request::poll) {
            self.request = None;
            match result {
                Ok(response) => {
                    if diagnostics_enabled() {
                        eprintln!("limonada: loaded {} communities", response.communities.len());
                    }
                    self.items = response.communities;
                    self.phase = Phase::Ready;
                }
                Err(err) => self.phase = Phase::Failed(err.to_string()),
            }
        }

        let Some(result) = self.find_request.as_ref().and_then(Request::poll) else {
            return;
        };
        self.find_request = None;
        let answered = self.find_in_flight.take();
        let current = self.lookup_text();
        // A reply for text that is no longer in the box says nothing about what
        // is in it now; the next tick asks again.
        if answered.as_deref() != Some(current.as_str()) {
            return;
        }
        match result {
            Ok(response) => {
                if diagnostics_enabled() {
                    eprintln!(
                        "limonada: community lookup \"{current}\" found {}",
                        response.communities.len()
                    );
                }
                self.found = response.communities;
                self.find_query = current;
                self.find_phase = Phase::Ready;
            }
            Err(err) => {
                // Record the query as answered so a failure does not retry in a
                // loop every debounce interval; the panel offers a Retry.
                self.find_query = current;
                self.find_phase = Phase::Failed(err.to_string());
            }
        }
    }

    /// Communities matching the sidebar filter, case-insensitively on name or
    /// title.
    pub fn filtered(&self) -> Vec<&CommunityView> {
        let needle = self.lookup_text().to_ascii_lowercase();
        if needle.is_empty() {
            return self.items.iter().collect();
        }
        self.items
            .iter()
            .filter(|view| {
                view.community.name.to_ascii_lowercase().contains(&needle)
                    || view
                        .community
                        .title
                        .as_deref()
                        .is_some_and(|t| t.to_ascii_lowercase().contains(&needle))
            })
            .collect()
    }
}

/// The lookup text for a sidebar filter.
///
/// Strips the `!` people write before a community name: a live lemmy.ml
/// 0.19.20 answers `q=!technology` with zero communities, so keeping the prefix
/// would make the most natural way to type a community the one way that never
/// matches. A trailing `@instance` is dropped too — the lookup runs against one
/// instance, where the qualifier only removes matches.
fn community_query(filter: &str) -> String {
    let text = filter.trim().trim_start_matches('!').trim();
    match text.split_once('@') {
        Some((name, _instance)) => name.trim().to_string(),
        None => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Site
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct SiteState {
    pub info: Option<GetSiteResponse>,
    pub phase: Phase,
    request: Option<Request<GetSiteResponse>>,
}

impl SiteState {
    pub fn refresh(&mut self, ctx: &Context, client: &LemmyClient) {
        if self.request.is_some() {
            return;
        }
        self.phase = Phase::Loading;
        let client = client.clone();
        self.request = Some(spawn(ctx.clone(), move || client.site()));
    }

    pub fn poll(&mut self) {
        let Some(result) = self.request.as_ref().and_then(Request::poll) else {
            return;
        };
        self.request = None;
        match result {
            Ok(info) => {
                if diagnostics_enabled() {
                    eprintln!(
                        "limonada: connected to {} (lemmy {}), {} users",
                        info.site_view.site.name, info.version, info.site_view.counts.users
                    );
                }
                self.info = Some(info);
                self.phase = Phase::Ready;
            }
            Err(err) => self.phase = Phase::Failed(err.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// A search, its query and its results.
///
/// The four result arrays are kept separate rather than merged because the
/// instance fills only the one matching the requested type: a `type_=Posts`
/// search returns posts and three empty arrays, so a merged list would report
/// three kinds of nothing.
#[derive(Default)]
pub struct Search {
    pub query: String,
    pub type_: SearchType,
    pub sort: Sort,
    /// The listing scope sent with the query. Always [`ListingType::All`].
    ///
    /// Search is a lookup across everything the instance federates with, and it
    /// must not inherit the feed's listing: with `listing_type=Local` a live
    /// lemmy.ml returned only its own communities, so a search could never find
    /// `!linuxmemes@lemmy.world` or `!linux@programming.dev`.
    pub listing: ListingType,
    /// A community to search within, when the feed is scoped to one.
    pub community_id: Option<i64>,
    pub posts: Vec<PostView>,
    pub comments: Vec<CommentView>,
    pub communities: Vec<CommunityView>,
    pub users: Vec<PersonView>,
    pub phase: Phase,
    /// The type the displayed results actually answer for.
    pub shown_type: Option<String>,
    /// True once a search has been run, so "nothing found" is distinguishable
    /// from "nothing searched yet".
    pub attempted: bool,
    request: Option<Request<SearchResponse>>,
}

impl Search {
    /// A fresh search, ranked for *finding* rather than for a feed.
    ///
    /// The derived default is `Sort::Hot`, and the server ranks the community
    /// lens with the same sort as the rest: a live lemmy.ml 0.19.20 answers
    /// `q=linux&type_=All&sort=Hot` with `!fedora` (6 subscribers) first and
    /// never lists `!linux` (68,166), while `sort=TopAll` returns `!linux`
    /// first. The picker still offers every sort and shows the choice, so this
    /// is a default rather than a hidden override.
    pub fn new() -> Self {
        Self {
            sort: Sort::TopAll,
            ..Self::default()
        }
    }

    pub fn is_busy(&self) -> bool {
        self.request.is_some()
    }

    /// Run the configured search again even if one is already in flight.
    ///
    /// Used when the user changes what the search is narrowed to: the answer to
    /// the old narrowing must not land after the change, and a toggle that did
    /// nothing while a request ran would look broken.
    pub fn resubmit(&mut self, ctx: &Context, client: &LemmyClient) {
        self.request = None;
        self.submit(ctx, client);
    }

    /// How many results are on screen, across every kind.
    pub fn total(&self) -> usize {
        self.posts.len() + self.comments.len() + self.communities.len() + self.users.len()
    }

    pub fn clear_results(&mut self) {
        self.posts.clear();
        self.comments.clear();
        self.communities.clear();
        self.users.clear();
        self.shown_type = None;
        self.phase = Phase::Idle;
    }

    /// Run the search as currently configured. An empty query is refused rather
    /// than sent: a live lemmy.ml 0.19.20 answers a bare `q=` by returning the
    /// whole instance, which is not a search the user asked for.
    pub fn submit(&mut self, ctx: &Context, client: &LemmyClient) {
        if self.request.is_some() {
            return;
        }
        let query = self.query.trim().to_string();
        if query.is_empty() {
            self.clear_results();
            self.phase = Phase::Failed("Type something to search for.".into());
            return;
        }
        self.phase = Phase::Loading;
        let (client, type_, sort, listing, community_id) = (
            client.clone(),
            self.type_,
            self.sort,
            self.listing,
            self.community_id,
        );
        self.request = Some(spawn(ctx.clone(), move || {
            client.search(&query, type_, sort, listing, community_id, SEARCH_PAGE_SIZE)
        }));
    }

    pub fn poll(&mut self) {
        let Some(result) = self.request.as_ref().and_then(Request::poll) else {
            return;
        };
        self.request = None;
        self.attempted = true;
        match result {
            Ok(response) => {
                if diagnostics_enabled() {
                    eprintln!(
                        "limonada: search \"{}\" ({}) returned {} posts, {} comments, {} communities, {} users",
                        self.query,
                        response.type_.as_deref().unwrap_or("?"),
                        response.posts.len(),
                        response.comments.len(),
                        response.communities.len(),
                        response.users.len()
                    );
                }
                self.shown_type = response.type_.clone();
                self.posts = response.posts;
                self.comments = response.comments;
                self.communities = response.communities;
                self.users = response.users;
                self.phase = Phase::Ready;
            }
            Err(err) => {
                self.clear_results();
                self.phase = Phase::Failed(err.to_string());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------

/// The sign-in form and its in-flight request.
///
/// The password lives here only while a sign-in is being attempted and is never
/// written to disk — what is persisted is the token the instance returns.
pub struct Auth {
    pub username: String,
    pub password: String,
    pub totp: String,
    pub phase: Phase,
    request: Option<Request<LoginResponse>>,
    /// The account and instance the in-flight sign-in was for, captured when it
    /// was sent so the session can be built from the answer.
    for_username: String,
    for_instance: String,
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            totp: String::new(),
            phase: Phase::Idle,
            request: None,
            for_username: String::new(),
            for_instance: String::new(),
        }
    }
}

impl Auth {
    pub fn is_busy(&self) -> bool {
        self.request.is_some()
    }

    /// Attempt a sign-in with the current form contents.
    pub fn submit(&mut self, ctx: &Context, client: &LemmyClient, instance_url: &str) {
        if self.request.is_some() {
            return;
        }
        let username = self.username.trim().to_string();
        if username.is_empty() || self.password.is_empty() {
            self.phase = Phase::Failed("Enter a username or email and a password.".into());
            return;
        }
        self.phase = Phase::Loading;
        self.for_username = username.clone();
        self.for_instance = instance_url.to_string();
        let (client, password, totp) = (
            client.clone(),
            self.password.clone(),
            self.totp.trim().to_string(),
        );
        self.request = Some(spawn(ctx.clone(), move || {
            let totp = (!totp.is_empty()).then_some(totp.as_str());
            client.login(&username, &password, totp)
        }));
    }

    /// Apply a finished sign-in. `Ok` carries the session to install; `Err` is
    /// the message to show in the form.
    pub fn poll(&mut self) -> Option<Result<Session, String>> {
        let result = self.request.as_ref()?.poll()?;
        self.request = None;
        match result {
            Ok(response) => match response.jwt.filter(|token| !token.is_empty()) {
                Some(token) => {
                    self.password.clear();
                    self.totp.clear();
                    self.phase = Phase::Idle;
                    Some(Ok(Session {
                        instance_url: self.for_instance.clone(),
                        username: self.for_username.clone(),
                        token,
                    }))
                }
                // A 200 with no token is not a session; treat it as a refusal
                // rather than storing an empty one.
                None => {
                    let message = "The instance accepted the sign-in but issued no token.".to_string();
                    self.phase = Phase::Failed(message.clone());
                    Some(Err(message))
                }
            },
            Err(err) => {
                let message = err.to_string();
                self.phase = Phase::Failed(message.clone());
                Some(Err(message))
            }
        }
    }

    /// Forget a finished form, e.g. after signing out.
    pub fn reset(&mut self) {
        self.password.clear();
        self.totp.clear();
        self.phase = Phase::Idle;
    }
}

// ---------------------------------------------------------------------------
// Votes
// ---------------------------------------------------------------------------

/// A finished vote, tagged with what it changed so the answer can be applied to
/// whichever view holds it — a feed row, the post header, or one comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Voted {
    Post(PostView),
    Comment(CommentView),
}

/// The one in-flight vote, if any.
///
/// One at a time: a vote confirms something the user just did, so queueing
/// several would only race their own answers. The row is left untouched until
/// the instance replies, and then replaced with the instance's own view.
#[derive(Default)]
pub struct Votes {
    request: Option<Request<Voted>>,
    /// What was clicked, so a failure can say which vote it was.
    label: String,
}

impl Votes {
    pub fn is_busy(&self) -> bool {
        self.request.is_some()
    }

    pub fn cast_post(
        &mut self,
        ctx: &Context,
        client: &LemmyClient,
        post_id: i64,
        score: i16,
    ) {
        if self.request.is_some() {
            return;
        }
        self.label = format!("post {post_id}");
        let client = client.clone();
        self.request = Some(spawn(ctx.clone(), move || {
            client
                .like_post(post_id, score)
                .map(|response| Voted::Post(response.post_view))
        }));
    }

    pub fn cast_comment(
        &mut self,
        ctx: &Context,
        client: &LemmyClient,
        comment_id: i64,
        score: i16,
    ) {
        if self.request.is_some() {
            return;
        }
        self.label = format!("comment {comment_id}");
        let client = client.clone();
        self.request = Some(spawn(ctx.clone(), move || {
            client
                .like_comment(comment_id, score)
                .map(|response| Voted::Comment(response.comment_view))
        }));
    }

    /// Apply a finished vote, reporting which one it was alongside any error so
    /// the caller can both apply it and name it.
    pub fn poll(&mut self) -> Option<(String, Result<Voted, ApiError>)> {
        let result = self.request.as_ref()?.poll()?;
        self.request = None;
        Some((std::mem::take(&mut self.label), result))
    }
}

/// Whether to print one line per completed load to stderr.
///
/// Opt-in via `LIMONADA_LOG=1` so a normally launched app stays quiet, while a
/// run from a terminal can be checked for what it actually fetched.
pub fn diagnostics_enabled() -> bool {
    match std::env::var("LIMONADA_LOG") {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join(format!("limonada-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        let _ = std::fs::remove_file(&path);

        let settings = Settings {
            instance_url: "https://lemmy.world".into(),
            demo: true,
            sort: Sort::New,
            listing: ListingType::Local,
            comment_sort: CommentSort::Controversial,
            theme: Theme::Dark,
            session: Some(Session {
                instance_url: "https://lemmy.world".into(),
                username: "tester".into(),
                token: "a-secret-token".into(),
            }),
        };
        settings.save(Some(&path)).expect("save should succeed");

        let loaded = Settings::load(Some(&path));
        assert_eq!(loaded, settings, "settings must survive a round trip");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_missing_settings_file_falls_back_to_defaults() {
        let missing = std::env::temp_dir().join("limonada-does-not-exist/settings.json");
        let loaded = Settings::load(Some(&missing));
        assert_eq!(loaded.instance_url, "https://lemmy.ml");
        assert_eq!(loaded.sort, Sort::Hot);
    }

    #[test]
    fn a_corrupt_settings_file_falls_back_to_defaults_instead_of_failing() {
        let dir = std::env::temp_dir().join(format!("limonada-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ this is not json").expect("write");

        let loaded = Settings::load(Some(&path));
        assert_eq!(loaded, Settings::default());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn instance_url_is_normalised() {
        let settings = Settings {
            instance_url: "  https://lemmy.ml//  ".into(),
            ..Default::default()
        };
        assert_eq!(settings.normalised_instance(), "https://lemmy.ml");
    }

    #[test]
    fn community_filter_matches_name_and_title_case_insensitively() {
        use crate::api::types::{Community, CommunityAggregates, CommunityView};
        let make = |name: &str, title: &str| CommunityView {
            community: Community {
                id: 1,
                name: name.into(),
                title: Some(title.into()),
                description: None,
                icon: None,
                banner: None,
                actor_id: None,
                instance_id: None,
                local: true,
                nsfw: false,
                hidden: false,
                removed: false,
                deleted: false,
                visibility: None,
                posting_restricted_to_mods: false,
                published: None,
                updated: None,
            },
            counts: CommunityAggregates::default(),
            subscribed: None,
            blocked: false,
            banned_from_community: false,
        };

        let mut communities = Communities {
            items: vec![make("rust", "The Rust programming language"), make("news", "World News")],
            ..Default::default()
        };

        communities.filter = "RUST".into();
        assert_eq!(communities.filtered().len(), 1);
        assert_eq!(communities.filtered()[0].community.name, "rust");

        communities.filter = "world".into();
        assert_eq!(communities.filtered().len(), 1, "should match the title too");

        communities.filter = "  ".into();
        assert_eq!(communities.filtered().len(), 2, "blank filter matches all");
    }

    /// An empty query is refused, not sent: a live `search?q=` returns the whole
    /// instance rather than an error, so a stray press must not scan it.
    #[test]
    fn an_empty_search_query_is_refused_rather_than_sent() {
        let client = LemmyClient::new(std::sync::Arc::new(crate::api::FixtureTransport::new(
            fixtures_dir(),
        )));
        let ctx = Context::default();
        let mut search = Search {
            query: "   ".into(),
            ..Default::default()
        };

        search.submit(&ctx, &client);

        assert!(
            search.phase.error().is_some_and(|e| e.contains("something")),
            "the empty query should be reported, got {:?}",
            search.phase
        );
        assert!(!search.is_busy(), "nothing should have been sent");
        assert!(!search.attempted, "and nothing searched");
    }

    /// A post as an instance sends it: only the keys a case cares about are
    /// spelled out, and the real wire names are used, so a field renamed here
    /// stops matching what the server sends.
    fn post_view(post: &str) -> PostView {
        let json = format!(
            r#"{{"post": {post}, "creator": {{"id": 1, "name": "someone"}},
                 "community": {{"id": 1, "name": "pics"}}}}"#
        );
        serde_json::from_str(&json).expect("the captured post shape should decode")
    }

    /// Measured against a live lemmy.world: this post's thumbnail URL served
    /// 397x512 while the picture the post links to served 3104x4000.
    const TALL_IMAGE_POST: &str = r#"{
        "id": 53149782,
        "name": "A tall picture",
        "url": "https://lemmy.world/pictrs/image/f70abc78-26b9-4914-8204-f54c5d01e80f.png",
        "url_content_type": "image/png",
        "thumbnail_url": "https://lemmy.world/pictrs/image/b3760eb3-e588-455d-ae69-13c85f4bed14.png",
        "alt_text": "  a very tall diagram  "
    }"#;

    /// A linked article: its `url` is an HTML page and only the thumbnail is a
    /// picture. Measured on a live lemmy.ml post.
    const LINK_POST: &str = r#"{
        "id": 54036651,
        "name": "Dutch tax authorities cancel Microsoft 365 rollout",
        "url": "https://www.zdnet.com/business/dutch-tax-authorities-cancel-microsoft-365-cloud-rollout/",
        "url_content_type": "text/html; charset=utf-8",
        "thumbnail_url": "https://lemmy.ml/api/v3/image_proxy?url=https%3A%2F%2Fwww.zdnet.com%2Fog.jpg",
        "alt_text": ""
    }"#;

    #[test]
    fn a_picture_post_opens_its_own_image_rather_than_the_thumbnail() {
        let view = post_view(TALL_IMAGE_POST);
        assert_eq!(
            best_image_url(&view),
            Some("https://lemmy.world/pictrs/image/f70abc78-26b9-4914-8204-f54c5d01e80f.png")
        );

        let viewer = Viewer::for_post(&view).expect("a viewer");
        assert_eq!(
            viewer.url,
            "https://lemmy.world/pictrs/image/f70abc78-26b9-4914-8204-f54c5d01e80f.png",
            "the viewer must open the larger image, not the thumbnail"
        );
        assert!(viewer.shows_more_than_the_page());
        assert_eq!(
            viewer.resolution_note(),
            "the page behind is showing a smaller preview of this image"
        );
        assert_eq!(
            viewer.texture_to_release(),
            Some(viewer.url.as_str()),
            "the full-size copy belongs to the viewer, so it leaves with it"
        );
        // The poster's own description is trimmed of the whitespace real
        // instances pad it with.
        assert_eq!(viewer.alt.as_deref(), Some("a very tall diagram"));
    }

    #[test]
    fn a_link_post_uses_its_thumbnail_rather_than_the_page_it_links_to() {
        let viewer = Viewer::for_post(&post_view(LINK_POST)).expect("a viewer");
        assert!(
            viewer.url.contains("image_proxy"),
            "an HTML page must never be handed to the picture viewer; got {}",
            viewer.url
        );
        assert!(!viewer.shows_more_than_the_page());
        assert_eq!(
            viewer.resolution_note(),
            "the page was already showing the largest copy the instance offers"
        );
        assert_eq!(
            viewer.texture_to_release(),
            None,
            "releasing this would blank the picture the page is still showing"
        );
        assert_eq!(viewer.alt, None, "a blank alt text is not a description");
    }

    #[test]
    fn the_extension_decides_only_when_the_server_sends_no_content_type() {
        // Instances older than `url_content_type` still link straight at images.
        assert!(looks_like_image(
            "https://example.org/pictures/a.JPEG",
            None
        ));
        // Query strings and fragments are not part of the extension.
        assert!(looks_like_image(
            "https://example.org/a.webp?format=webp#x",
            Some("  ")
        ));
        assert!(!looks_like_image("https://example.org/a.html", None));
        assert!(!looks_like_image("https://example.org/photo.jpg.html", None));

        // ...but a content type, when there is one, is what the server says it
        // serves, whatever the path looks like.
        assert!(!looks_like_image(
            "https://example.org/a.png",
            Some("text/html; charset=utf-8")
        ));
        assert!(looks_like_image(
            "https://example.org/no-extension",
            Some("image/avif")
        ));
    }

    #[test]
    fn a_post_with_no_picture_has_nothing_to_open() {
        let bare = post_view(r#"{"id": 7, "name": "just text"}"#);
        assert_eq!(best_image_url(&bare), None);
        assert_eq!(Viewer::for_post(&bare), None);

        // A white-space thumbnail is an omission, not a URL.
        let blank =
            post_view(r#"{"id": 8, "name": "blank thumb", "thumbnail_url": "  "}"#);
        assert_eq!(Viewer::for_post(&blank), None);
    }

    /// A live federated post that served AVIF: `image/avif`, with the thumbnail
    /// the very same URL, so there was no smaller copy to fall back to either.
    /// It is decodable now — see `avif_is_decodable_through_its_own_loader` —
    /// and the captured bytes live in `fixtures/pictures/avif-post.avif`.
    const AVIF_POST: &str = r#"{
        "id": 53150680,
        "name": "An AVIF picture",
        "url": "https://lemmy.zip/pictrs/image/f1c404f0-eda7-4f8a-9495-8ca9dbf3d3b6.avif",
        "url_content_type": "image/avif",
        "thumbnail_url": "https://lemmy.zip/pictrs/image/f1c404f0-eda7-4f8a-9495-8ca9dbf3d3b6.avif"
    }"#;

    /// A format neither the `image` features nor the AVIF loader covers. BMP is
    /// one `image` could read with a feature this build does not enable.
    const BMP_POST: &str = r#"{
        "id": 9,
        "name": "A BMP picture",
        "url": "https://example.org/wallpaper.bmp",
        "url_content_type": "image/bmp",
        "thumbnail_url": "https://example.org/wallpaper.bmp"
    }"#;

    #[test]
    fn a_format_this_build_cannot_decode_is_named_rather_than_shown_as_broken() {
        let viewer = Viewer::for_post(&post_view(BMP_POST)).expect("a viewer");
        assert!(!viewer.decodable, "this build enables no BMP decoder");
        assert_eq!(viewer.format.as_deref(), Some("BMP"));
        assert_eq!(
            viewer.undecodable_note().as_deref(),
            Some("BMP cannot be decoded by this build — use Open in browser")
        );

        // The formats the build does have say nothing extra.
        let png = Viewer::for_post(&post_view(TALL_IMAGE_POST)).expect("a viewer");
        assert!(png.decodable);
        assert_eq!(png.undecodable_note(), None);
    }

    #[test]
    fn avif_is_decodable_through_its_own_loader() {
        let viewer = Viewer::for_post(&post_view(AVIF_POST)).expect("a viewer");
        assert_eq!(viewer.format.as_deref(), Some("AVIF"));
        assert!(
            viewer.decodable,
            "AVIF is decoded by the loader in crate::avif"
        );
        assert_eq!(viewer.undecodable_note(), None);
    }

    /// The formats this build can actually read, value by value: adding one to
    /// the match without a decoder behind it cannot pass unnoticed.
    #[test]
    fn decodable_formats_are_exactly_the_ones_this_build_can_read() {
        for (url, content_type, expected) in [
            ("https://e.org/a.png", Some("image/png"), true),
            ("https://e.org/a", Some("image/jpeg"), true),
            ("https://e.org/a.JPG", None, true),
            ("https://e.org/a.gif?x=1", None, true),
            ("https://e.org/a.webp", Some("  "), true),
            ("https://e.org/a.svg", Some("image/svg+xml"), true),
            // AVIF arrives through crate::avif's own loader, not the `image`
            // crate, which is why this is true despite Cargo.toml's features.
            ("https://e.org/a.avif", Some("image/avif"), true),
            ("https://lemmy.zip/pictrs/image/abc.avif", None, true),
            // Formats this build has no decoder for at all.
            ("https://e.org/a.bmp", Some("image/bmp"), false),
            ("https://e.org/a.tiff", None, false),
            ("https://e.org/a", Some("image/heic"), false),
            // Not an image at all, and not a format guess either.
            ("https://e.org/page", Some("text/html"), false),
            ("https://cdn.example.org/f/117/443/original/6a51006a", None, false),
        ] {
            assert_eq!(
                decodes(url, content_type),
                expected,
                "decodes({url:?}, {content_type:?})"
            );
        }

        assert_eq!(
            image_format("https://e.org/a.svg", Some("image/svg+xml")),
            Some("SVG".to_string())
        );
        assert_eq!(image_format("https://e.org/a", Some("text/html")), None);
    }

    #[test]
    fn zoom_switches_between_the_two_modes_and_labels_the_action() {
        assert_eq!(Zoom::default(), Zoom::Fit);
        assert_eq!(Zoom::Fit.toggled(), Zoom::Actual);
        assert_eq!(Zoom::Actual.toggled(), Zoom::Fit);
        assert_ne!(Zoom::Fit.label(), Zoom::Actual.label());
    }

    /// The settings file can hold a session token, so saving must leave it
    /// readable by its owner alone.
    #[cfg(unix)]
    #[test]
    fn saving_settings_keeps_the_file_for_the_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("limonada-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_file(&path);

        Settings::default().save(Some(&path)).expect("save");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "settings should be owner-only, got {:o}",
            mode & 0o777
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// A token is issued by one instance and rejected by every other, so a
    /// stored session must not be reused after the instance changes.
    #[test]
    fn a_session_only_counts_on_the_instance_that_issued_it() {
        let session = |instance: &str| Session {
            instance_url: instance.into(),
            username: "tester".into(),
            token: "a-token".into(),
        };
        let mut settings = Settings {
            instance_url: "https://lemmy.ml//".into(),
            ..Default::default()
        };

        settings.session = Some(session("https://lemmy.ml"));
        assert!(
            settings.active_session().is_some(),
            "a trailing slash in the configured URL must not hide the session"
        );

        settings.session = Some(session("https://lemmy.world"));
        assert!(
            settings.active_session().is_none(),
            "a token must never be offered to a different instance"
        );
    }

    /// The `!` and `@instance` forms are how people write community names, and
    /// a live instance answers `q=!technology` with nothing at all.
    #[test]
    fn a_community_filter_is_looked_up_without_its_decoration() {
        assert_eq!(community_query("  !technology "), "technology");
        assert_eq!(community_query("!linux@lemmy.ml"), "linux");
        assert_eq!(community_query("linux"), "linux");
        assert_eq!(community_query("!"), "");
    }

    /// The sidebar filter must not cost one request per keystroke, and a
    /// single letter is not worth asking the instance about.
    #[test]
    fn the_community_lookup_waits_for_the_typing_to_stop() {
        let ctx = Context::default();
        let client = LemmyClient::new(std::sync::Arc::new(crate::api::FixtureTransport::new(
            fixtures_dir(),
        )));

        let mut communities = Communities {
            filter: "l".into(),
            ..Communities::default()
        };
        communities.tick(&ctx, &client);
        assert!(
            !communities.is_finding(),
            "a one-letter filter is not a lookup"
        );
        assert!(communities.found.is_empty());

        communities.filter = "lin".into();
        communities.tick(&ctx, &client);
        assert!(
            !communities.is_finding(),
            "the first tick only starts the debounce"
        );

        // The debounce is wall-clock; wait it out rather than reaching inside.
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(5));
            communities.tick(&ctx, &client);
            if communities.is_finding() {
                break;
            }
        }
        assert!(
            communities.is_finding(),
            "the lookup should run once the text settles"
        );
    }

    /// Searching is a lookup, not a feed: `Hot` buries well known communities
    /// under brand-new ones (measured: `q=linux` never listed `!linux`).
    #[test]
    fn a_fresh_search_is_ranked_by_activity() {
        assert_eq!(Search::new().sort, Sort::TopAll);
        assert_eq!(Sort::default(), Sort::Hot, "the feed default is unchanged");
    }

    /// The filter used to match the raw text, so typing the `!` form matched
    /// nothing in the loaded page either.
    #[test]
    fn the_sidebar_filter_matches_the_undecorated_name() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/community_list.json"
        ))
        .expect("the captured community page should be readable");
        let page: ListCommunitiesResponse =
            serde_json::from_str(&text).expect("the captured community page should decode");
        let name = page.communities[0].community.name.clone();

        let mut communities = Communities::default();
        communities.items = page.communities;

        communities.filter = format!("!{name}");
        assert_eq!(
            communities.filtered().len(),
            1,
            "the `!` form is how people write a community, and it should match"
        );
        communities.filter = name;
        assert_eq!(communities.filtered().len(), 1);
        communities.filter = "a-name-that-is-not-in-the-page".into();
        assert!(communities.filtered().is_empty());
    }
}
