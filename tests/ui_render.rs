//! Headless rendering of the real UI with captured data.
//!
//! These tests drive [`Limonada::draw`] — the same function `eframe` calls each
//! frame — through a bare `egui::Context`, with no window, GPU or network. They
//! render the actual layouts from real captured payloads.
//!
//! Two levels of assertion:
//!
//! 1. The frame runs to completion and emits paint shapes (no panic, no layout
//!    blow-up), including for narrow windows and every error/loading/empty state.
//! 2. The frame's accessibility tree contains the expected *text* — post titles,
//!    community names, comment bodies — which is what proves the widgets were
//!    actually built from the data rather than merely that something was drawn.
//!
//! Scope, stated honestly: this verifies structure and content, not pixel
//! appearance. A real-window capture is not possible in this environment (see
//! the README's verification notes).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use limonada::api::{
    ApiError, FixtureTransport, LemmyClient, ListingType, SearchType, Sort, Transport,
};
use limonada::app::{Limonada, View};
use limonada::state::{CommunityScope, Phase, Session, Settings, Zoom};

/// A post id present in the captured fixtures.
///
/// `FixtureTransport` answers by endpoint rather than by id, so the value only
/// needs to be the one the fixtures were captured for.
const FIXTURE_POST_ID: i64 = 53_989_052;

/// The instance the fixture-backed headless apps are configured for.
///
/// A loopback address nothing listens on, so if any code path ever did try the
/// network it would fail loudly rather than silently reach a real instance.
const TEST_INSTANCE: &str = "http://127.0.0.1:9";

fn demo_app() -> Limonada {
    // Demo mode reads `fixtures/`, so no request can leave the process. The URL
    // is never used in this mode.
    Limonada::headless(Settings {
        instance_url: TEST_INSTANCE.into(),
        demo: true,
        ..Default::default()
    })
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn read_fixture(name: &str) -> serde_json::Value {
    let path = fixtures_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

/// A fixture-backed transport that also answers the writes, so a headless frame
/// can exercise a vote or a comment with no network and no account.
///
/// The vote answers are modelled on what a real instance does — it returns the
/// updated view, whose score it has already moved and whose `my_vote` records
/// the vote — which is what lets the tests assert the row changed rather than
/// only that a request was sent.
struct StubTransport {
    fixtures: FixtureTransport,
    writes: Mutex<Vec<(String, serde_json::Value)>>,
}

impl StubTransport {
    fn new() -> Self {
        Self {
            fixtures: FixtureTransport::new(fixtures_dir()),
            writes: Mutex::new(Vec::new()),
        }
    }

    fn writes(&self) -> Vec<(String, serde_json::Value)> {
        self.writes.lock().expect("lock").clone()
    }

    /// The body sent to `path`, if any.
    fn write_to(&self, path: &str) -> Option<serde_json::Value> {
        self.writes()
            .into_iter()
            .find(|(p, _)| p == path)
            .map(|(_, body)| body)
    }
}

impl Transport for StubTransport {
    fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, ApiError> {
        self.fixtures.get(path, query)
    }

    fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, ApiError> {
        self.writes
            .lock()
            .expect("lock")
            .push((path.to_string(), body.clone()));
        match path {
            "/api/v3/post/like" => {
                let mut view = read_fixture("post_detail.json")["post_view"].clone();
                let score = body["score"].as_i64().unwrap_or(0);
                // The captured score is this account's *vote-less* score, so the
                // new total is that plus the vote just cast — which makes score
                // 0 (withdrawing) return the total to where it started.
                let base = view["counts"]["score"].as_i64().unwrap_or(0);
                view["counts"]["score"] = serde_json::json!(base + score);
                // No vote is an absent field, which is what an instance sends.
                view["my_vote"] = if score == 0 {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(score)
                };
                Ok(serde_json::json!({"post_view": view}))
            }
            "/api/v3/comment/like" => {
                let mut view = read_fixture("comment_list.json")["comments"][0].clone();
                let score = body["score"].as_i64().unwrap_or(0);
                let before = view["counts"]["score"].as_i64().unwrap_or(0);
                view["counts"]["score"] = serde_json::json!(before + score);
                view["my_vote"] = serde_json::json!(score);
                Ok(serde_json::json!({"comment_view": view, "recipient_ids": []}))
            }
            "/api/v3/comment" => Ok(serde_json::json!({
                "comment_view": read_fixture("comment_list.json")["comments"][0].clone(),
                "recipient_ids": []
            })),
            other => Err(ApiError::Refused(format!("the stub has no answer for {other}"))),
        }
    }

    fn description(&self) -> String {
        "stub fixtures".into()
    }

    fn set_auth(&self, _token: Option<String>) {}

    fn auth(&self) -> Option<String> {
        None
    }
}

/// A headless app on the stub transport, signed in or not.
fn app_with_writes(signed_in: bool) -> (Limonada, Arc<StubTransport>) {
    let stub = Arc::new(StubTransport::new());
    let session = signed_in.then(|| Session {
        instance_url: TEST_INSTANCE.into(),
        username: "tester".into(),
        token: "a-session-token".into(),
    });
    let settings = Settings {
        instance_url: TEST_INSTANCE.into(),
        demo: false,
        session,
        ..Default::default()
    };
    let client = LemmyClient::new(stub.clone());
    let mut app = Limonada::headless_with_client(settings, client);
    app.prime_blocking().expect("fixtures should load");
    (app, stub)
}

/// Render frames until `done` holds of the current state, or the budget runs
/// out.
///
/// The predicate is given the app rather than a captured reference on purpose:
/// a request finishes on a worker thread, and the result is only applied when a
/// later frame polls it. Waiting on what the *transport* recorded would return
/// before the app had seen the answer — which is exactly the race these tests
/// hit when they waited on the stub instead.
fn render_until(
    app: &mut Limonada,
    ctx: &egui::Context,
    mut done: impl FnMut(&Limonada) -> bool,
) {
    for _ in 0..400 {
        if done(app) {
            return;
        }
        run_frame(app, ctx, vec![]);
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("condition never became true within the frame budget");
}

/// The score the open post currently shows, and this account's own vote on it.
fn post_score_and_vote(app: &Limonada) -> (i64, Option<i16>) {
    app.detail
        .as_ref()
        .and_then(|detail| detail.post.as_ref())
        .map(|post| (post.counts.score, post.my_vote))
        .expect("a post should be open")
}

/// Render one frame and return the text the UI exposed through egui's
/// accessibility tree — the labels and values of every node it built.
///
/// This is the strongest available check that the UI contains the right content:
/// a frame that draws nothing useful still emits shapes, but it will not carry
/// the post titles or comment text.
fn accessibility_text(app: &mut Limonada, ctx: &egui::Context, width: f32, height: f32) -> String {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, height),
        )),
        ..Default::default()
    };

    let mut output = ctx.run_ui(input, |ui| app.draw(ui));
    let mut text = String::new();
    if let Some(update) = output.platform_output.accesskit_update.as_ref() {
        for (_id, node) in &update.nodes {
            if let Some(label) = node.label() {
                text.push_str(label);
                text.push('\n');
            }
            if let Some(value) = node.value() {
                text.push_str(value);
                text.push('\n');
            }
        }
    }
    output.textures_delta.clear();
    text
}

/// Render one frame at a given size and return how many shapes were emitted.
fn render(app: &mut Limonada, ctx: &egui::Context, width: f32, height: f32) -> usize {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, height),
        )),
        ..Default::default()
    };

    let mut output = ctx.run_ui(input, |ui| app.draw(ui));
    let shapes = output.shapes.len();

    // There is no renderer here to consume the texture uploads, and epaint
    // asserts if a `TexturesDelta` is dropped unhandled. Clearing it is the
    // documented way to say "intentionally ignored" — a real backend applies
    // these instead.
    output.textures_delta.clear();

    shapes
}

#[test]
fn the_rendered_feed_carries_the_captured_post_text() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    let (first_title, first_community, first_creator) = {
        let row = &app.feed.items[0];
        (
            row.post.name.clone(),
            row.community.name.clone(),
            // What the row actually renders: the display name when the account
            // has one, otherwise the handle. Asserting against `name` would be
            // asserting the wrong string for any user with a display name.
            row.creator.label().to_string(),
        )
    };

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);

    assert!(
        text.contains("Limonada"),
        "the window title text should be in the tree; got {} chars",
        text.len()
    );
    assert!(
        text.contains(&first_title),
        "the top post's title should be rendered; looked for {first_title:?}"
    );
    assert!(
        text.contains(&first_community),
        "its community should be rendered; looked for {first_community:?}"
    );
    assert!(
        text.contains(&first_creator),
        "its author should be rendered; looked for {first_creator:?}"
    );

    // How much of the page made it into the tree. Not asserted tightly, because
    // egui may skip nodes outside the clip rect, but reported so a regression
    // that drops most rows is visible.
    let present = app
        .feed
        .items
        .iter()
        .filter(|row| text.contains(&row.post.name))
        .count();
    println!(
        "accessibility tree carried {present}/{} post titles ({} chars total)",
        app.feed.items.len(),
        text.len()
    );
}

#[test]
fn the_rendered_thread_carries_the_captured_comment_text() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    let (post_title, first_comment_body, first_comment_author) = {
        let detail = app.detail.as_ref().expect("detail");
        let post = detail.post.as_ref().expect("post");
        let node = &detail.comments[0];
        (
            post.post.name.clone(),
            node.comment.comment.content.clone(),
            node.comment.creator.name.clone(),
        )
    };

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);

    assert!(text.contains("Comments"), "the thread heading should render");
    assert!(
        text.contains(&post_title),
        "the post title should render; looked for {post_title:?}"
    );
    assert!(
        text.contains(&first_comment_author),
        "a comment author should render; looked for {first_comment_author:?}"
    );

    // Comment bodies go through the Markdown renderer, which may split a body
    // into several nodes, so match on a distinctive opening fragment.
    let fragment: String = first_comment_body.chars().take(24).collect();
    assert!(
        text.contains(fragment.trim()),
        "comment text should render; looked for {fragment:?}"
    );
}

#[test]
fn renders_the_feed_from_captured_data() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    assert_eq!(app.feed.items.len(), 20, "the captured page has 20 posts");
    assert!(app.feed.next_page.is_some(), "the captured page has a cursor");
    assert_eq!(app.communities.items.len(), 50);

    let first = render(&mut app, &ctx, 1280.0, 820.0);
    assert!(first > 0, "the feed frame must paint something");

    // A second frame exercises state reuse (ids, caches) rather than a cold
    // first paint.
    let second = render(&mut app, &ctx, 1280.0, 820.0);
    assert!(second > 0, "the second feed frame must paint something");
}

#[test]
fn renders_a_comment_thread_from_captured_data() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID)
        .expect("the post fixture should load");

    // Copy what the assertions need out of the borrow, so `render` can take
    // `&mut app` afterwards.
    let (comment_count, root_count, root_id) = {
        let detail = app.detail.as_ref().expect("a detail view should be open");
        (
            detail.comment_count(),
            detail.comments.len(),
            detail.comments[0].comment.comment.id,
        )
    };
    assert_eq!(comment_count, 19, "the captured thread has 19 comments");
    assert_eq!(root_count, 10, "with 10 top-level comments");

    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);

    // Collapsing a top-level comment must render without blowing up.
    if let Some(detail) = app.detail.as_mut() {
        detail.collapsed.insert(root_id);
    }
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);
}

#[test]
fn renders_with_the_sidebar_and_settings_panel_toggled() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    // Exercises the settings panel and the status bar together.
    app.sidebar_open = false;
    app.settings_open = true;
    assert!(
        render(&mut app, &ctx, 1280.0, 820.0) > 0,
        "settings panel should paint"
    );

    app.sidebar_open = true;
    app.settings_open = false;
    assert!(render(&mut app, &ctx, 900.0, 600.0) > 0);

    // A very narrow window must not panic either.
    assert!(render(&mut app, &ctx, 480.0, 360.0) > 0);
}

/// The second reported bug, driven through the real widgets: a post row names
/// its community but clicking that name did nothing. The chip must scope the
/// feed, and must not also open the post the way a click on the card does.
#[test]
fn clicking_a_posts_community_scopes_the_feed_without_opening_the_post() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    // So the only `!name` labels on screen are the feed's own chips.
    app.sidebar_open = false;

    let (id, name) = {
        let row = &app.feed.items[0];
        (row.community.id, row.community.name.clone())
    };
    let label = format!("!{name}");

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let rect = announced_rect(&frame, &label)
        .unwrap_or_else(|| panic!("the first post should show its community {label:?}"));
    let (press, release) = click_at(rect.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert_eq!(
        app.feed.scope.as_ref().map(|scope| (scope.id, scope.name.clone())),
        Some((id, name.clone())),
        "clicking the community chip should scope the feed to !{name}"
    );
    assert!(
        app.detail.is_none(),
        "the chip must not fall through to the card and open the post"
    );

    for _ in 0..600 {
        app.feed.poll();
        if app.feed.phase == Phase::Ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.feed.items.is_empty(), "the scoped feed should show posts");
}

/// The other half of the same rule: the card must still open the post. The
/// chip's click target is registered after the card's, so a regression here
/// would be a card that stopped opening anything at all.
#[test]
fn clicking_a_post_card_still_opens_the_post() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.sidebar_open = false;

    let (post_id, name) = {
        let row = &app.feed.items[0];
        (row.post.id, row.community.name.clone())
    };
    let chip_label = format!("!{name}");

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let chip = announced_rect(&frame, &chip_label)
        .unwrap_or_else(|| panic!("the first post should show its community {chip_label:?}"));

    // Below the chip's row but still inside the card: the title and the body
    // live there, and the card covers them.
    let pos = chip.center() + egui::vec2(0.0, 30.0);
    let (press, release) = click_at(pos);
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert_eq!(
        app.detail.as_ref().map(|detail| detail.post_id),
        Some(post_id),
        "a click on the card should open its post"
    );
    assert!(
        app.feed.scope.is_none(),
        "a card click must not be mistaken for a community click"
    );
}

/// The same chip in the open thread, where there is no card to compete with.
#[test]
fn clicking_the_community_in_a_thread_scopes_the_feed() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.sidebar_open = false;
    app.open_post_blocking(FIXTURE_POST_ID)
        .expect("the post fixture should load");

    let (id, name) = {
        let post = app
            .detail
            .as_ref()
            .and_then(|detail| detail.post.as_ref())
            .expect("an open post");
        (post.community.id, post.community.name.clone())
    };
    let label = format!("!{name}");

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let rect = announced_rect(&frame, &label)
        .unwrap_or_else(|| panic!("the thread header should show its community {label:?}"));
    let (press, release) = click_at(rect.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert_eq!(
        app.feed.scope.as_ref().map(|scope| (scope.id, scope.name.clone())),
        Some((id, name.clone())),
        "the thread's community chip should scope the feed"
    );
    assert_eq!(app.view, View::Feed, "and show the feed it was scoped to");

    // The scoped feed is what the click was asking for, so it has to arrive.
    for _ in 0..600 {
        app.feed.poll();
        if app.feed.phase == Phase::Ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.feed.phase, Phase::Ready, "the scoped feed should answer");
    assert!(
        !app.feed.items.is_empty(),
        "the community should show its posts, not an empty feed"
    );
}

/// Search used to inherit the feed's listing. With `listing_type=Local` a live
/// instance returned only its own communities, which is the "just local" the
/// user saw; search must always span the federation the instance knows.
#[test]
fn search_is_never_narrowed_to_the_feed_listing() {
    let (mut app, _stub) = app_with_writes(false);
    app.settings.listing = ListingType::Local;

    app.open_search();

    assert_eq!(
        app.search.listing,
        ListingType::All,
        "a search must span every instance this one federates with"
    );
}

/// The reported bug, driven through the real widgets: clicking a community
/// showed no posts. The row is clicked for real, and the feed must then be
/// scoped to it *and* have loaded its posts — a recorded scope with an empty
/// list is the failure being guarded against.
#[test]
fn clicking_a_community_scopes_the_feed_and_loads_its_posts() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    let label = app
        .communities
        .items
        .iter()
        .map(|view| limonada::ui::widgets::community_handle(&view.community))
        .find(|label| label.chars().count() <= 30)
        .expect("a sidebar row whose handle is not truncated");
    let name = app
        .communities
        .items
        .iter()
        .find(|view| limonada::ui::widgets::community_handle(&view.community) == label)
        .map(|view| view.community.name.clone())
        .expect("the row's community");

    // Two frames: the first lays the sidebar out, the second has the rectangles
    // the accessibility tree reports.
    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let rect = announced_rect(&frame, &label)
        .unwrap_or_else(|| panic!("the sidebar should announce the row {label:?}"));
    let (press, release) = click_at(rect.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert_eq!(
        app.feed.scope.as_ref().map(|scope| scope.name.clone()),
        Some(name.clone()),
        "clicking the row should scope the feed to !{name}"
    );

    // The scoped page loads on a worker thread; drain it the way frames do.
    // `refresh` set the phase to Loading, so reaching Ready again means the
    // scoped request answered rather than the previously loaded page.
    for _ in 0..600 {
        app.feed.poll();
        if app.feed.phase == Phase::Ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.feed.phase, Phase::Ready, "the scoped feed should answer");
    assert!(
        !app.feed.items.is_empty(),
        "clicking !{name} must show posts, not an empty feed"
    );

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        text.contains(&format!("!{name}")),
        "the scoped feed heading should name the community"
    );
}

/// The sidebar holds one page of communities, so its filter also asks the
/// instance. Without that lookup a well known community which the page happens
/// not to contain could not be found at all — the "I searched a well known
/// community and it does not show" failure.
#[test]
fn the_sidebar_lookup_renders_communities_beyond_the_page() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    // A filter that matches nothing in the loaded page, so every row on show
    // comes from the lookup rather than from `items`.
    app.communities.filter = "zzz-not-in-the-page".into();
    assert!(
        app.communities.filtered().is_empty(),
        "the loaded page should not match this filter"
    );
    let query = app.communities.lookup_text();
    app.communities.find(&ctx, &app.client, &query);

    // The lookup runs on a worker thread; drain it the way a frame would.
    for _ in 0..400 {
        app.communities.poll();
        if !app.communities.found.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        app.communities.found.len(),
        3,
        "the captured lookup response has three communities"
    );

    let name = app.communities.found[0].community.name.clone();
    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        text.contains("Elsewhere on this instance"),
        "the lookup section should be labelled so the rows are not mistaken for the page"
    );
    assert!(
        text.contains(&name),
        "the instance-wide match should be rendered; looked for {name:?}"
    );
}

/// A search used to inherit the feed's community scope. A live instance answers
/// `type_=All` with empty arrays when `community_id` is not the community being
/// searched for, so searching for a well known community after clicking any
/// community returned "no results" with nothing to explain it.
#[test]
fn opening_search_does_not_inherit_the_feed_scope() {
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    app.feed.scope = Some(CommunityScope {
        id: 7,
        name: "technology".into(),
    });
    app.open_search();

    assert!(
        app.search.community_id.is_none(),
        "opening the search screen must not narrow it silently"
    );
    assert_eq!(
        app.search.sort,
        Sort::TopAll,
        "a search is ranked for finding, not by a feed's Hot"
    );
    // Narrowing is still available, and explicit when chosen.
    app.search.community_id = Some(7);
    assert_eq!(app.search.community_id, Some(7));
}

/// An empty community is a normal state, not a failure — and it needs a way
/// out, since the feed stays scoped until the user leaves it.
#[test]
fn an_empty_community_says_so_and_offers_the_way_out() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    app.feed.scope = Some(CommunityScope {
        id: 4_242,
        name: "quietplace".into(),
    });
    app.feed.items.clear();
    app.feed.next_page = None;
    app.feed.phase = Phase::Ready;

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        text.contains("!quietplace has no posts yet"),
        "the empty state should name the community rather than blame the filters"
    );
    assert!(
        text.contains("Show all posts"),
        "and it should offer the way back to the instance feed"
    );
}

#[test]
fn renders_error_and_empty_states() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    // Feed failure with nothing to show.
    app.feed.items.clear();
    app.feed.next_page = None;
    app.feed.phase = Phase::Failed("the instance is unreachable".into());
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0, "error state should paint");

    // Empty-but-successful feed.
    app.feed.phase = Phase::Ready;
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0, "empty state should paint");

    // Loading state.
    app.feed.phase = Phase::Loading;
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0, "loading state should paint");
}

#[test]
fn renders_an_empty_thread_as_a_normal_state() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    // What a federated post with unservable comments looks like.
    if let Some(detail) = app.detail.as_mut() {
        detail.comments.clear();
        detail.thread_phase = Phase::Ready;
    }
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);

    // ...and the failure path for the thread specifically.
    if let Some(detail) = app.detail.as_mut() {
        detail.thread_phase = Phase::Failed("thread request failed".into());
    }
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);
}

/// Run one frame with the given input events, returning everything egui
/// produced for it.
fn run_frame(
    app: &mut Limonada,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 820.0),
        )),
        events,
        ..Default::default()
    };

    let mut output = ctx.run_ui(input, |ui| app.draw(ui));
    output.textures_delta.clear();
    output
}

/// Where the widget announcing `label` was in the last frame.
///
/// The accessibility tree carries each widget's rectangle, which is how a test
/// can act on a piece of the interface without hardcoding pixels that any
/// layout change would invalidate.
fn announced_rect(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
    let update = output.platform_output.accesskit_update.as_ref()?;
    update.nodes.iter().find_map(|(_id, node)| {
        if node.label() != Some(label) {
            return None;
        }
        node.bounds().map(|bounds| {
            egui::Rect::from_min_max(
                egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
            )
        })
    })
}

/// The press and the release of a left click at `pos`, as two frames.
///
/// Split across frames because egui resolves a click from a press it has
/// already registered, exactly as a real mouse behaves.
fn click_at(pos: egui::Pos2) -> (Vec<egui::Event>, Vec<egui::Event>) {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    (
        vec![egui::Event::PointerMoved(pos), button(true)],
        vec![button(false)],
    )
}

/// The image the test post points at, and the thumbnail the page shows for it.
///
/// Measured live on lemmy.world: the thumbnail URL served 397x512 while the
/// post's own image served 3104x4000.
const FULL_SIZE_PICTURE: &str =
    "https://lemmy.world/pictrs/image/f70abc78-26b9-4914-8204-f54c5d01e80f.png";
const THUMBNAIL_PICTURE: &str =
    "https://lemmy.world/pictrs/image/b3760eb3-e588-455d-ae69-13c85f4bed14.png";

/// A description the test can find the picture's widget by.
const PICTURE_DESCRIPTION: &str = "a very tall chart";

/// Give the open post a picture whose own image is larger than its thumbnail.
fn give_the_post_a_picture(app: &mut Limonada) {
    let post = &mut app
        .detail
        .as_mut()
        .expect("the fixture post should be open")
        .post
        .as_mut()
        .expect("the fixture post should be loaded")
        .post;
    post.url = Some(FULL_SIZE_PICTURE.to_string());
    post.url_content_type = Some("image/png".to_string());
    post.thumbnail_url = Some(THUMBNAIL_PICTURE.to_string());
    post.alt_text = Some(PICTURE_DESCRIPTION.to_string());
}

#[test]
fn clicking_a_posts_picture_opens_it_at_full_resolution() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");
    give_the_post_a_picture(&mut app);

    // Frame one: nothing is open, and the picture is on the page.
    let first = run_frame(&mut app, &ctx, vec![]);
    let picture = announced_rect(&first, PICTURE_DESCRIPTION)
        .expect("the post's picture should be in the accessibility tree");
    assert!(
        picture.width() > 1.0 && picture.height() > 1.0,
        "the picture should occupy a clickable area, got {picture:?}"
    );
    assert!(app.image.is_none(), "nothing should be open before the click");

    // Frames two and three: click it, as a user would.
    let (press, release) = click_at(picture.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    let viewer = app
        .image
        .as_ref()
        .expect("clicking the picture should open the viewer");
    assert_eq!(
        viewer.url, FULL_SIZE_PICTURE,
        "the viewer must open the post's own image, not the thumbnail it was told to show"
    );
    assert!(viewer.shows_more_than_the_page());

    // The overlay is drawn, not merely recorded in state: its own words are in
    // the tree of the next frame.
    let overlay_text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    for expected in [
        "lemmy.world",
        "smaller preview of this image",
        "Actual size (100%)",
        "Open in browser",
        "Esc closes",
        PICTURE_DESCRIPTION,
    ] {
        assert!(
            overlay_text.contains(expected),
            "the open picture should show {expected:?}; the overlay exposed: {overlay_text:?}"
        );
    }

    // The zoom toggle works rather than merely rendering: clicking what it
    // offers flips the mode, and the button then offers the way back.
    let overlay_frame = run_frame(&mut app, &ctx, vec![]);
    let toggle = announced_rect(&overlay_frame, "Actual size (100%)")
        .expect("the overlay should offer the 1:1 view");
    let (press, release) = click_at(toggle.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);
    assert_eq!(
        app.image.as_ref().expect("still open").zoom,
        Zoom::Actual,
        "clicking the toggle should switch to one image pixel per screen pixel"
    );
    assert!(
        accessibility_text(&mut app, &ctx, 1280.0, 820.0).contains("Fit to window"),
        "the button should now offer the way back"
    );

    // Escape closes it, as its own caption promises.
    run_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }],
    );
    assert!(app.image.is_none(), "Esc should dismiss the picture");

    let after = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        !after.contains("smaller preview of this image"),
        "the overlay should be gone from the frame, not just from state"
    );
    assert!(
        after.contains(&app.detail.as_ref().expect("detail").post.as_ref().expect("post").post.name),
        "the post should still be there behind it"
    );
}

/// The ✕ in the overlay's caption, in both zoom modes — including the 1:1 view
/// the report came from.
///
/// This button reported the close through a flag of the overlay's own, and
/// `show` then assigned egui's own close signal to that same flag, discarding
/// it: the button did nothing whatsoever. Nothing caught it, because the two
/// other dismissal paths (the backdrop and `Esc`) were covered and this one was
/// not — a test per *path*, not per promise.
#[test]
fn the_overlay_close_button_dismisses_the_picture() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");
    give_the_post_a_picture(&mut app);

    let title = app
        .detail
        .as_ref()
        .and_then(|detail| detail.post.as_ref())
        .expect("the fixture post should be loaded")
        .post
        .name
        .clone();

    for zoom in [Zoom::Fit, Zoom::Actual] {
        assert!(app.show_open_post_image(), "the post has a picture");
        app.image.as_mut().expect("a viewer").zoom = zoom;

        // Two frames so the button's rect exists to be aimed at: egui resolves
        // a click against the layout its previous pass reported.
        run_frame(&mut app, &ctx, vec![]);
        let frame = run_frame(&mut app, &ctx, vec![]);
        let button = announced_rect(&frame, "✕").unwrap_or_else(|| {
            panic!("the overlay should offer a close button in {zoom:?}")
        });

        let (press, release) = click_at(button.center());
        run_frame(&mut app, &ctx, press);
        run_frame(&mut app, &ctx, release);

        assert!(
            app.image.is_none(),
            "the ✕ button should dismiss the picture in {zoom:?}"
        );

        // Gone from the frame, not just from state — and the post behind it is
        // still there, so closing the overlay must not close the thread too.
        let after = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
        assert!(
            !after.contains("Esc closes"),
            "the overlay should be gone from the frame in {zoom:?}; it exposed: {after:?}"
        );
        assert!(
            after.contains(&title),
            "the post should still be behind the closed overlay in {zoom:?}"
        );
    }
}

/// The two other ✕ buttons in the interface.
///
/// Neither shares the overlay's bug — each writes one flag that nothing else
/// touches — but they were the *other* untested close buttons, and an untested
/// close button is exactly how the overlay's went a whole release doing
/// nothing. Each test first asserts the thing is open, so it cannot pass by
/// finding nothing to click.
#[test]
fn the_settings_views_close_button_closes_it() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    app.settings_open = true;
    // No status message, so the settings panel's ✕ is the only one on screen.
    app.status = None;

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    assert!(
        accessibility_text(&mut app, &ctx, 1280.0, 820.0).contains("Instance"),
        "the settings panel should be open before the click"
    );
    let button = announced_rect(&frame, "✕").expect("the settings panel should offer a ✕");

    let (press, release) = click_at(button.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert!(!app.settings_open, "the ✕ should close the settings panel");
    assert!(
        !accessibility_text(&mut app, &ctx, 1280.0, 820.0).contains("Instance"),
        "the panel should be gone from the frame, not just from state"
    );
}

#[test]
fn a_status_messages_dismiss_button_clears_it() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    const MESSAGE: &str = "could not load more posts";
    app.settings_open = false;
    app.status = Some(limonada::app::Status::error(MESSAGE));

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    assert!(
        accessibility_text(&mut app, &ctx, 1280.0, 820.0).contains(MESSAGE),
        "the message should be on screen before the click"
    );
    let button =
        announced_rect(&frame, "✕").expect("a status message should offer a dismiss button");

    let (press, release) = click_at(button.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert!(app.status.is_none(), "the ✕ should dismiss the message");
    assert!(
        !accessibility_text(&mut app, &ctx, 1280.0, 820.0).contains(MESSAGE),
        "the message should be gone from the frame, not just from state"
    );
}

#[test]
fn clicking_outside_the_picture_dismisses_it() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");
    give_the_post_a_picture(&mut app);

    assert!(app.show_open_post_image(), "the post has a picture");
    // Two frames to lay the overlay out: egui attributes a click to the widget
    // its *previous* pass reported, and an overlay's backdrop has no usable rect
    // until the pass that first draws it. Then a click far outside the picture —
    // over the community sidebar, which the backdrop covers while it is up.
    run_frame(&mut app, &ctx, vec![]);
    run_frame(&mut app, &ctx, vec![]);
    let (press, release) = click_at(egui::pos2(40.0, 400.0));
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert!(
        app.image.is_none(),
        "a click outside the picture should dismiss it"
    );
    assert!(
        app.detail.is_some(),
        "dismissing the picture must leave the post it was opened from"
    );
}

#[test]
fn an_undecodable_format_is_explained_in_the_overlay() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    // A format this build has no decoder for at all. egui draws a bare warning
    // glyph for one, so the overlay has to say why.
    const BMP: &str = "https://example.org/wallpaper.bmp";
    if let Some(post) = app.detail.as_mut().and_then(|detail| detail.post.as_mut()) {
        post.post.url = Some(BMP.to_string());
        post.post.url_content_type = Some("image/bmp".to_string());
        post.post.thumbnail_url = Some(BMP.to_string());
    }

    assert!(
        app.show_open_post_image(),
        "there is a picture to open, even if this build cannot decode it"
    );
    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        text.contains("BMP cannot be decoded by this build"),
        "the overlay should name the format it cannot read; it exposed: {text:?}"
    );
    assert!(
        text.contains("Open in browser"),
        "...and offer the way to actually see it"
    );
}

#[test]
fn avif_no_longer_gets_an_explanation_instead_of_a_picture() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    // The live federated post that used to be the example of an undecodable
    // format, with its thumbnail the very same URL. A loader of its own now
    // covers it (`src/avif.rs`), so the viewer must treat it as an ordinary
    // picture and say nothing about formats.
    const AVIF: &str =
        "https://lemmy.zip/pictrs/image/f1c404f0-eda7-4f8a-9495-8ca9dbf3d3b6.avif";
    if let Some(post) = app.detail.as_mut().and_then(|detail| detail.post.as_mut()) {
        post.post.url = Some(AVIF.to_string());
        post.post.url_content_type = Some("image/avif".to_string());
        post.post.thumbnail_url = Some(AVIF.to_string());
    }

    assert!(app.show_open_post_image(), "the post has a picture");
    let viewer = app.image.as_ref().expect("a viewer");
    assert!(viewer.decodable, "AVIF is decodable in this build");
    assert_eq!(viewer.format.as_deref(), Some("AVIF"));

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        !text.contains("cannot be decoded"),
        "an AVIF picture should need no explanation; the overlay exposed: {text:?}"
    );
}

#[test]
fn a_post_without_a_picture_leaves_the_click_path_alone() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    // The fixture post's own picture fields, which for this capture are empty.
    if let Some(post) = app.detail.as_mut().and_then(|d| d.post.as_mut()) {
        post.post.url = None;
        post.post.thumbnail_url = None;
    }

    assert!(
        render(&mut app, &ctx, 1280.0, 820.0) > 0,
        "a post with no picture should still render"
    );
    assert!(
        !app.show_open_post_image(),
        "there is nothing to open, and the caller is told so"
    );
    assert!(app.image.is_none());
}

#[test]
fn a_transient_status_message_renders() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.status = Some(limonada::app::Status::error("could not load more posts"));
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[test]
fn the_search_view_renders_captured_results() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");

    // Captured from a live lemmy.ml: `q=lemmy&type_=All&limit=3`, three of each.
    app.search.query = "lemmy".into();
    app.search.type_ = SearchType::All;
    app.open_search();
    app.search_blocking()
        .expect("the captured search should load");

    assert_eq!(app.search.posts.len(), 3);
    assert_eq!(app.search.users.len(), 3);

    let (post_title, community, user) = {
        let post = app.search.posts[0].post.name.clone();
        let community = app.search.communities[0].community.name.clone();
        let user = app.search.users[0].person.label().to_string();
        (post, community, user)
    };

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(text.contains("Search"), "the search heading should render");
    assert!(
        text.contains(&post_title),
        "a post result should render; looked for {post_title:?}"
    );
    assert!(
        text.contains(&community),
        "a community result should render; looked for {community:?}"
    );
    assert!(
        text.contains(&user),
        "a user result should render; looked for {user:?}"
    );
    // Each kind is under its own heading, so a result's kind is unambiguous.
    for heading in [
        "Posts (3)",
        "Communities (3)",
        "Comments (3)",
        "Users (3)",
    ] {
        assert!(
            text.contains(heading),
            "the {heading:?} heading should render; it exposed: {text:?}"
        );
    }
}

#[test]
fn the_search_view_renders_its_prompt_empty_loading_and_error_states() {
    let ctx = egui::Context::default();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.open_search();

    // Nothing searched yet, which is not the same as "nothing found".
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);

    // A query that returned nothing.
    app.search.attempted = true;
    app.search.phase = Phase::Ready;
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);

    // A failure, and the loading state that precedes it.
    app.search.phase = Phase::Failed("the instance could not answer".into());
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);
    app.search.phase = Phase::Loading;
    assert!(render(&mut app, &ctx, 1280.0, 820.0) > 0);
}

// ---------------------------------------------------------------------------
// The account
// ---------------------------------------------------------------------------

#[test]
fn the_account_panel_shows_who_is_signed_in() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, _stub) = app_with_writes(true);
    app.auth_open = true;

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        text.contains("Signed in as tester"),
        "the account panel should name the account; it exposed: {text:?}"
    );
    assert!(text.contains("Sign out"), "and offer the way out");
    assert!(
        !text.contains("Username or email"),
        "a signed-in panel must not ask for credentials again"
    );
}

#[test]
fn a_signed_out_account_panel_offers_the_sign_in_form() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, _stub) = app_with_writes(false);
    app.auth_open = true;

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    for expected in [
        "Username or email",
        "Password",
        "Two-factor code",
        "Sign in",
    ] {
        assert!(
            text.contains(expected),
            "the form should offer {expected:?}; it exposed: {text:?}"
        );
    }
    assert!(!text.contains("Signed in as"), "nobody is signed in");
}

/// Demo mode refuses every write, so it cannot sign in — and says so rather than
/// offering a form that could only fail.
#[test]
fn demo_mode_says_it_cannot_sign_in() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = demo_app();
    app.prime_blocking().expect("fixtures should load");
    app.auth_open = true;

    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        text.contains("Demo mode cannot sign in"),
        "demo mode should explain itself; it exposed: {text:?}"
    );
}

// ---------------------------------------------------------------------------
// Voting and commenting
// ---------------------------------------------------------------------------

/// Give the open post an empty thread, so the only vote controls on screen are
/// the post header's own. Then `announced_rect("▲")` is unambiguous.
fn clear_the_thread(app: &mut Limonada) {
    if let Some(detail) = app.detail.as_mut() {
        detail.comments.clear();
        detail.thread_phase = Phase::Ready;
    }
}

#[test]
fn clicking_the_upvote_sends_a_vote_and_updates_the_post() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, stub) = app_with_writes(true);
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");
    clear_the_thread(&mut app);

    let (before_score, before_vote) = {
        let post = app.detail.as_ref().and_then(|d| d.post.as_ref()).expect("post");
        (post.counts.score, post.my_vote)
    };
    assert_eq!(before_vote, None, "the captured fixture records no vote");

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let up = announced_rect(&frame, "▲").expect("the header should offer an upvote arrow");
    let (press, release) = click_at(up.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    // Wait for the app to have *applied* the answer, not merely for the worker
    // thread to have sent it.
    render_until(&mut app, &ctx, |app| post_score_and_vote(app).1 == Some(1));

    let body = stub.write_to("/api/v3/post/like").expect("a vote write");
    assert_eq!(body["post_id"], FIXTURE_POST_ID);
    assert_eq!(body["score"], 1);
    assert_eq!(
        post_score_and_vote(&app).0,
        before_score + 1,
        "the instance's own answer should update the score"
    );
}

/// Clicking the arrow already voted with takes the vote back, which is the only
/// way to undo one — Lemmy's `score` of 0.
#[test]
fn clicking_the_arrow_again_takes_the_vote_back() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, stub) = app_with_writes(true);
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");
    clear_the_thread(&mut app);

    let before_score = post_score_and_vote(&app).0;

    // Vote once, and wait for the answer.
    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let up = announced_rect(&frame, "▲").expect("an upvote arrow");
    let (press, release) = click_at(up.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);
    render_until(&mut app, &ctx, |app| post_score_and_vote(app).1 == Some(1));

    // Now click it again; the second vote should be the withdrawal, score 0.
    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let up = announced_rect(&frame, "▲").expect("the arrow, now selected");
    let (press, release) = click_at(up.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    // The withdrawal returns the total to where it started and clears the vote.
    render_until(&mut app, &ctx, |app| {
        let (score, vote) = post_score_and_vote(app);
        score == before_score && vote.is_none()
    });
    let bodies: Vec<serde_json::Value> = stub
        .writes()
        .into_iter()
        .filter(|(path, _)| path == "/api/v3/post/like")
        .map(|(_, body)| body)
        .collect();
    assert_eq!(bodies.len(), 2, "two vote writes");
    assert_eq!(
        bodies[1]["score"], 0,
        "the second click on the same arrow must withdraw the vote"
    );
}

#[test]
fn voting_signed_out_asks_for_a_sign_in_and_sends_nothing() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, stub) = app_with_writes(false);
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");
    clear_the_thread(&mut app);

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let up = announced_rect(&frame, "▲").expect("the controls are still drawn when signed out");
    let (press, release) = click_at(up.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert!(
        stub.writes().is_empty(),
        "a signed-out vote must never be sent: {:?}",
        stub.writes()
    );
    assert!(
        app.status
            .as_ref()
            .is_some_and(|status| status.text.contains("Sign in")),
        "the user should be told why nothing happened; status was {:?}",
        app.status
    );
}

#[test]
fn adding_a_comment_opens_the_editor_and_posts_it() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, stub) = app_with_writes(true);
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    assert!(
        !accessibility_text(&mut app, &ctx, 1280.0, 820.0).contains("Sign in to comment or vote"),
        "a signed-in post must not be told to sign in"
    );
    let add = announced_rect(&frame, "✎ Add a comment")
        .expect("a signed-in post should offer Add a comment");

    let (press, release) = click_at(add.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert_eq!(
        app.detail.as_ref().expect("post").reply_to,
        Some(None),
        "the editor should target a new top-level comment"
    );
    assert!(
        accessibility_text(&mut app, &ctx, 1280.0, 820.0).contains("New comment on this post"),
        "the editor should say what it is writing"
    );

    // Write the draft, then send it through the editor's own button.
    app.detail.as_mut().expect("post").reply_draft = "a considered reply".into();
    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let post = announced_rect(&frame, "Post comment").expect("the editor should offer Post comment");
    let (press, release) = click_at(post.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    // The editor closing is the app having applied the answer.
    render_until(&mut app, &ctx, |app| {
        app.detail.as_ref().is_some_and(|detail| {
            detail.reply_to.is_none() && detail.reply_draft.is_empty()
        })
    });

    let body = stub.write_to("/api/v3/comment").expect("a comment write");
    assert_eq!(body["content"], "a considered reply");
    assert_eq!(body["post_id"], FIXTURE_POST_ID);
    assert!(
        body.get("parent_id").is_none(),
        "a top-level comment must not carry a parent"
    );
    assert_eq!(
        app.detail.as_ref().expect("post").reply_to,
        None,
        "the editor should close once the comment is posted"
    );
    assert!(
        app.status
            .as_ref()
            .is_some_and(|status| status.text.contains("Comment posted")),
        "the status bar should confirm it; status was {:?}",
        app.status
    );
}

/// A reply names its parent, so it lands under the comment it answers.
#[test]
fn replying_to_a_comment_targets_that_comment() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, stub) = app_with_writes(true);
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    let (root_id, author) = {
        let detail = app.detail.as_ref().expect("post");
        let root = &detail.comments[0];
        (root.comment.comment.id, root.comment.creator.label().to_string())
    };

    // Scroll the root comment's own Reply button into view, then click it.
    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let reply = announced_rect(&frame, "↩ Reply").expect("a comment should offer Reply");
    let (press, release) = click_at(reply.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    assert_eq!(
        app.detail.as_ref().expect("post").reply_to,
        Some(Some(root_id)),
        "the editor should target the comment that was clicked"
    );
    let text = accessibility_text(&mut app, &ctx, 1280.0, 820.0);
    assert!(
        text.contains(&format!("Replying to {author}")),
        "the editor should name who it is answering; it exposed: {text:?}"
    );

    app.detail.as_mut().expect("post").reply_draft = "point taken".into();
    run_frame(&mut app, &ctx, vec![]);
    let frame = run_frame(&mut app, &ctx, vec![]);
    let post = announced_rect(&frame, "Post comment").expect("Post comment");
    let (press, release) = click_at(post.center());
    run_frame(&mut app, &ctx, press);
    run_frame(&mut app, &ctx, release);

    render_until(&mut app, &ctx, |app| {
        app.detail.as_ref().is_some_and(|detail| detail.reply_to.is_none())
    });
    let body = stub.write_to("/api/v3/comment").expect("a comment write");
    assert_eq!(body["content"], "point taken");
    assert_eq!(body["parent_id"], root_id);
}

/// An empty draft is refused before it is sent: the instance would reject it,
/// and a round trip to be told so is worse than saying it in place.
#[test]
fn an_empty_comment_is_not_sent() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut app, stub) = app_with_writes(true);
    app.open_post_blocking(FIXTURE_POST_ID).expect("post");

    if let Some(detail) = app.detail.as_mut() {
        detail.start_reply(None);
        detail.submit_reply(&ctx, &app.client);
    }
    assert!(stub.writes().is_empty(), "nothing should have been sent");
    assert!(
        app.detail
            .as_ref()
            .and_then(|detail| detail.reply_phase.error())
            .is_some_and(|message| message.contains("empty")),
        "the editor should say why"
    );
}
