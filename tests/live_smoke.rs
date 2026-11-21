//! Live end-to-end smoke test against a real Lemmy instance.
//!
//! Ignored by default so `cargo test` stays offline and deterministic. Run it
//! deliberately:
//!
//! ```text
//! cargo test --test live_smoke -- --ignored --nocapture
//! ```
//!
//! It drives the same `HttpTransport` + `LemmyClient` + `build_threads` path the
//! application uses, so a pass means the shipped code browses a real instance —
//! not that a fixture still decodes.

use std::sync::Arc;
use std::time::Duration;


use limonada::api::{
    CommentSort, HttpTransport, LemmyClient, ListingType, SearchType, Sort, Transport,
};
use limonada::state::{
    THREAD_PAGE_SIZE, Viewer, best_image_url, decodes, inline_image_url, looks_like_image,
};
use limonada::tree::build_threads;

const INSTANCE: &str = "https://lemmy.ml";

fn live_client() -> LemmyClient {
    let transport = HttpTransport::new(INSTANCE).expect("instance URL should be usable");
    assert!(!transport.is_offline());
    assert!(
        transport.description().contains("lemmy.ml"),
        "transport should describe its instance"
    );
    LemmyClient::new(Arc::new(transport))
}

#[test]
#[ignore = "requires network access to a live Lemmy instance"]
fn browses_a_live_instance_end_to_end() {
    let client = live_client();

    // --- instance --------------------------------------------------------
    let site = client.site().expect("GET /api/v3/site");
    assert!(!site.site_view.site.name.is_empty());
    assert!(
        site.version.starts_with("0.19") || site.version.starts_with("1."),
        "unexpected server version {:?}",
        site.version
    );
    println!(
        "instance: {} (lemmy {}), {} users",
        site.site_view.site.name, site.version, site.site_view.counts.users
    );

    // --- feed, first page ------------------------------------------------
    let page_one = client
        .list_posts(Sort::Hot, ListingType::All, None, 10)
        .expect("GET /api/v3/post/list");
    assert!(!page_one.posts.is_empty(), "the hot feed should not be empty");

    // --- pagination ------------------------------------------------------
    let cursor = page_one
        .next_page
        .clone()
        .expect("a full first page should carry a cursor");
    let page_two = client
        .list_posts(Sort::Hot, ListingType::All, Some(&cursor), 10)
        .expect("GET /api/v3/post/list with a cursor");
    assert!(!page_two.posts.is_empty(), "page two should not be empty");
    let overlap = page_one
        .posts
        .iter()
        .filter(|a| page_two.posts.iter().any(|b| b.post.id == a.post.id))
        .count();
    assert_eq!(overlap, 0, "page two should not repeat page one");
    println!(
        "feed: {} posts on page 1, {} on page 2, no overlap",
        page_one.posts.len(),
        page_two.posts.len()
    );

    // --- communities -----------------------------------------------------
    // `TopAll`, the ordering the sidebar uses. The old version of this test
    // scanned ten communities and stopped at the first non-empty one, which is
    // why it passed while the sidebar — which listed the *newest* fifty, 31 of
    // them empty — showed no posts for most clicks.
    let communities = client
        .list_communities(ListingType::All, Sort::TopAll, 10)
        .expect("GET /api/v3/community/list");
    assert!(!communities.communities.is_empty());
    let first = &communities.communities[0];
    assert!(
        first.counts.posts > 0,
        "the directory must lead with an active community, but !{} has no posts",
        first.community.name
    );
    println!(
        "communities: first of {} is !{} ({} posts, {} subscribers)",
        communities.communities.len(),
        first.community.name,
        first.counts.posts,
        first.counts.subscribers
    );

    // --- community-scoped feed ------------------------------------------
    // The first entry is what a person clicks first, so it is the one that has
    // to produce a feed.
    let target = &first.community;
    let scoped = client
        .list_posts_in_community(Sort::Hot, target.id, None, 5)
        .expect("GET /api/v3/post/list?community_id=");
    assert!(
        !scoped.posts.is_empty(),
        "clicking the first listed community (!{}) should show posts",
        target.name
    );
    assert!(
        scoped
            .posts
            .iter()
            .all(|view| view.community.id == target.id),
        "a community-scoped feed must only contain that community's posts"
    );
    println!(
        "community scope: {} posts from !{}",
        scoped.posts.len(),
        target.name
    );

    // --- community search ranking ---------------------------------------
    // The sidebar lookup and the search screen rank communities with `TopAll`.
    // With `Hot`, a live instance answered `q=linux` with a six-subscriber
    // `!fedora` and never listed `!linux` at all.
    let found = client
        .search(
            "linux",
            SearchType::Communities,
            Sort::TopAll,
            ListingType::All,
            None,
            20,
        )
        .expect("GET /api/v3/search?type_=Communities");
    let leader = found
        .communities
        .first()
        .expect("a community search for `linux` should find something");
    assert!(
        leader.community.name.to_ascii_lowercase().contains("linux"),
        "the ranking should lead with a name containing the query, got !{}",
        leader.community.name
    );
    assert!(
        leader.counts.subscribers >= 1_000,
        "the ranking should lead with a well known community, got !{} with {} subscribers",
        leader.community.name,
        leader.counts.subscribers
    );
    // `listing_type=All` is what makes this a federated search: with `Local` a
    // live instance returned only its own communities, so a well known
    // community hosted elsewhere could never be found.
    let federated = found
        .communities
        .iter()
        .find(|view| !view.community.local)
        .expect("a federated community search should include remote communities");
    println!(
        "community search: !{} first ({} subscribers), federated: !{}{}",
        leader.community.name,
        leader.counts.subscribers,
        federated.community.name,
        federated
            .community
            .actor_id
            .as_deref()
            .and_then(|actor| actor.split("//").nth(1))
            .and_then(|rest| rest.split('/').next())
            .map(|host| format!("@{host}"))
            .unwrap_or_default(),
    );

    // --- find a post with a readable thread ------------------------------
    // Not every post with a comment count returns comments: a federated post
    // can report comments the instance cannot serve. So scan for one that does.
    //
    // The scan deliberately starts from **federated** posts, because that is
    // where the `type_` parameter matters: `comment/list` defaults it to
    // `Local`, so leaving it off returns an empty thread for every federated
    // post. Asking with the same parameters the application sends (including
    // `THREAD_PAGE_SIZE`, the constant the UI uses rather than a hardcoded
    // number) is what makes this test able to catch either regression.
    let mut candidates = 0usize;
    let mut served = 0usize;
    let mut thread = None;
    for view in page_one
        .posts
        .iter()
        .chain(page_two.posts.iter())
        .filter(|view| !view.post.local && view.counts.comments > 0)
    {
        candidates += 1;
        let comments = client
            .list_comments(view.post.id, CommentSort::Top, THREAD_PAGE_SIZE)
            .unwrap_or_else(|e| {
                panic!(
                    "GET /api/v3/comment/list failed for post {} with THREAD_PAGE_SIZE={THREAD_PAGE_SIZE}: {e}",
                    view.post.id
                )
            });
        if !comments.comments.is_empty() {
            served += 1;
            if thread.is_none() {
                thread = Some((view.post.id, comments));
            }
        }
        if candidates >= 12 {
            break;
        }
    }

    assert!(
        candidates > 0,
        "the sampled pages should contain a federated post reporting comments"
    );
    assert!(
        served > 0,
        "none of the {candidates} federated candidates served a thread. A live \
         instance returns these with `type_=All`; an empty result for all of \
         them means `type_` is being omitted (it defaults to `Local`) or the \
         page size is above the server cap"
    );
    println!("federated threads: {served}/{candidates} candidates served comments");

    let (post_id, comments) = thread.expect(
        "at least one post across two pages should have a readable comment thread; \
         if this fails, every listed thread was empty (see the counts-vs-served note)",
    );

    // --- post detail -----------------------------------------------------
    let detail = client.get_post(post_id).expect("GET /api/v3/post");
    assert_eq!(detail.post_view.post.id, post_id);
    assert!(
        detail.community_view.is_some(),
        "post detail should include its community"
    );

    // --- thread assembly on live data ------------------------------------
    let threads = build_threads(&comments.comments);
    let total: usize = threads
        .iter()
        .map(|root| 1 + root.descendant_count())
        .sum();
    assert_eq!(
        total,
        comments.comments.len(),
        "every live comment must appear exactly once in the built tree"
    );

    let mut max_depth = 0;
    for root in &threads {
        root.walk(&mut |depth, _| {
            max_depth = max_depth.max(depth);
            true
        });
    }

    // Every comment that has a parent inside the batch must be nested; anything
    // else must have been promoted to a root rather than dropped.
    let ids: std::collections::HashSet<i64> = comments
        .comments
        .iter()
        .map(|view| view.comment.id)
        .collect();
    let orphans = comments
        .comments
        .iter()
        .filter(|view| {
            let segments: Vec<&str> = view.comment.path.split('.').collect();
            if segments.len() < 3 {
                return false;
            }
            let parent = segments[segments.len() - 2];
            parent != "0" && !ids.contains(&parent.parse::<i64>().unwrap_or(-1))
        })
        .count();
    assert!(
        !threads.is_empty(),
        "a non-empty comment batch must produce at least one root"
    );

    println!(
        "thread on post {post_id}: {} comments, {} roots, max depth {max_depth}, {orphans} promoted orphans",
        comments.comments.len(),
        threads.len()
    );
}

/// Download a URL, as the image loader would, returning the bytes and the
/// content type the server sent with them.
fn fetch(url: &str) -> Result<(Vec<u8>, String), String> {
    let response = reqwest::blocking::Client::builder()
        .user_agent(limonada::api::transport::USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !status.is_success() {
        return Err(format!("HTTP {status} ({content_type})"));
    }
    response
        .bytes()
        .map(|b| (b.to_vec(), content_type))
        .map_err(|e| e.to_string())
}

/// The pixel size of the image a URL actually serves, decoded the way the
/// application decodes it.
///
/// AVIF goes through `limonada::avif`: the `image` crate cannot read it in this
/// build, and only a real instance can show whether that matters. Everything else
/// goes through the `image` features. Decoding rather than reading headers keeps
/// a header that parses while the body does not from passing as a pass.
fn served_size(url: &str) -> (u32, u32) {
    let (bytes, mime) = fetch(url).unwrap_or_else(|e| panic!("{url} did not load: {e}"));

    if limonada::avif::is_avif(url) {
        let image = limonada::avif::decode(&bytes)
            .unwrap_or_else(|e| panic!("{url} ({mime}) is AVIF and did not decode: {e}"));
        return (image.size[0] as u32, image.size[1] as u32);
    }

    let decoded = image::load_from_memory(&bytes)
        .unwrap_or_else(|e| panic!("{url} ({mime}) did not decode: {e}"));
    (decoded.width(), decoded.height())
}

/// The picture the viewer opens must be the higher-resolution one.
///
/// `best_image_url` is the shipped decision, so this drives it over live posts
/// rather than over a fixture: choosing a thumbnail when the post's own image
/// is available is the bug this guards, and only a real instance can say which
/// of the two is larger. `lemmy.ml` serves many federated thumbnails at their
/// original size, while `lemmy.world` resizes them — scanning both is what
/// makes the comparison land on posts where the two really differ.
#[test]
#[ignore = "requires network access to live Lemmy instances"]
fn the_picture_the_viewer_opens_is_the_larger_one() {
    /// Measure at most this many thumbnail/original pairs; each is two
    /// downloads, and a handful is enough to show the rule holding.
    const MAX_PAIRS: usize = 6;

    let mut posts_scanned = 0usize;
    let mut links_seen = 0usize;
    let mut undecodable = 0usize;
    let mut pairs = 0usize;
    let mut decoded_once = false;
    let mut gained = 0usize;

    for instance in ["https://lemmy.world", "https://lemmy.ml"] {
        let client = LemmyClient::new(Arc::new(
            HttpTransport::new(instance).expect("instance URL should be usable"),
        ));

        for sort in [Sort::Hot, Sort::New] {
            if pairs >= MAX_PAIRS {
                break;
            }
            let page = client
                .list_posts(sort, ListingType::All, None, 50)
                .unwrap_or_else(|e| panic!("GET {instance}/api/v3/post/list failed: {e}"));

            for view in &page.posts {
                posts_scanned += 1;
                let post = &view.post;
                let thumbnail = inline_image_url(view);

                // A post whose own URL is not an image must fall back to its
                // thumbnail: handing a web page to the picture viewer would show
                // the user nothing, or worse, an error.
                let links_a_page = post
                    .url
                    .as_deref()
                    .is_some_and(|url| !looks_like_image(url, post.url_content_type.as_deref()));
                if links_a_page {
                    links_seen += 1;
                    assert_eq!(
                        best_image_url(view),
                        thumbnail,
                        "post {} links a non-image, so its thumbnail is the only picture to open",
                        post.id
                    );
                    continue;
                }

                let Some(chosen) = best_image_url(view) else {
                    continue;
                };

                // The shipped constructor must be the thing under test, not
                // only the URL helper it is built on.
                let viewer = Viewer::for_post(view)
                    .unwrap_or_else(|| panic!("post {} should have an openable picture", post.id));
                assert_eq!(
                    viewer.url, chosen,
                    "the viewer must open what `best_image_url` chose"
                );

                // Instances do serve formats this build has no decoder for.
                // Those are the one honest exception, and they must be both real
                // and announced: a format the viewer claims it cannot decode had
                // better not be one it actually can.
                if let Some(note) = viewer.undecodable_note() {
                    let (bytes, served_type) = fetch(chosen)
                        .unwrap_or_else(|e| panic!("the chosen image {chosen} did not load: {e}"));
                    assert!(!bytes.is_empty(), "{chosen} served no bytes");
                    assert!(
                        served_type.starts_with("image/"),
                        "the viewer refused {chosen} as undecodable, but it is not an image: {served_type}"
                    );
                    assert!(
                        !decodes(chosen, Some(&served_type)),
                        "the viewer called {chosen} undecodable, but it serves {served_type}, \
                         which this build does decode"
                    );
                    println!("post {}: {note}", post.id);
                    undecodable += 1;
                    continue;
                }

                // Whatever was chosen has to decode, through the decoder the
                // application would use for it.
                let size = served_size(chosen);
                if !decoded_once {
                    decoded_once = true;
                    println!("decoded {chosen} end to end at {}x{}", size.0, size.1);
                }

                // ...and when the choice leaves the thumbnail behind, the larger
                // image must be the one opened. This is the assertion the
                // feature exists for.
                if thumbnail == Some(chosen) || pairs >= MAX_PAIRS {
                    continue;
                }

                let small = served_size(thumbnail.expect("a thumbnail here by construction"));
                pairs += 1;

                assert!(
                    size.0 >= small.0 && size.1 >= small.1,
                    "post {} opened {size:?}, which is smaller than the thumbnail it replaced {small:?}",
                    post.id
                );
                if size.0 > small.0 || size.1 > small.1 {
                    gained += 1;
                }
                let kind = if limonada::avif::is_avif(chosen) {
                    " (AVIF)"
                } else {
                    ""
                };
                println!(
                    "post {} ({instance}): opening {}x{}{kind} instead of the thumbnail's {}x{}",
                    post.id, size.0, size.1, small.0, small.1
                );
            }
        }
    }

    println!(
        "scanned {posts_scanned} posts, {links_seen} linking a web page, {undecodable} in an \
         undecodable format, {pairs} thumbnail/original pairs measured"
    );
    assert!(posts_scanned > 0, "the sample should not be empty");
    assert!(
        links_seen > 0,
        "the sample contained no post linking a non-image, so the fallback rule was never exercised"
    );
    assert!(
        gained > 0,
        "none of the {pairs} measured posts served a larger image than its thumbnail. The \
         viewer would add no resolution anywhere in this sample; widen the scan or re-run"
    );
    println!("posts where opening the picture adds resolution: {gained}");
}

/// AVIF, which instances do serve and the `image` crate cannot decode here.
///
/// The decoder is pure Rust (`zenavif`) because the alternative needs the C
/// library libdav1d, which would make a system package a build requirement. That
/// decision is only as good as its behaviour on real files, so this drives the
/// shipping decoder over AVIF pictures from live instances rather than trusting
/// the one captured fixture.
///
/// Which listing to scan decides whether AVIF turns up at all, and not in an
/// obvious way. Scanning `New` and `Hot` alone found **0** AVIF in 300 posts
/// across three instances on two consecutive runs, which looks exactly like a
/// broken detector. It is not. AVIF posts cluster on the top-of-day and
/// top-of-week listings, where picture-heavy posts have had time to rise, so an
/// empty sample was a property of the *listing*, not of the code: a direct scan
/// of six instances found AVIF under `TopDay` (2-3 per instance) and `TopWeek`
/// (3 per instance) on all six, and under `New` and `Hot` on none. The fix for
/// an empty sample is therefore to widen the sorts, not to distrust the code or
/// soften the assertion below.
#[test]
#[ignore = "requires network access to live Lemmy instances"]
fn live_avif_pictures_decode_through_the_clients_own_decoder() {
    /// Enough to show it is not a one-off, without decoding the whole feed.
    const WANTED: usize = 3;

    /// Both the general listings and the ones AVIF actually appears in.
    const SORTS: [Sort; 4] = [Sort::New, Sort::Hot, Sort::TopDay, Sort::TopWeek];

    let mut scanned = 0usize;
    let mut found = 0usize;
    let mut decoded = 0usize;

    for instance in ["https://lemmy.world", "https://lemmy.ml", "https://lemmy.zip"] {
        if decoded >= WANTED {
            break;
        }
        let client = LemmyClient::new(Arc::new(
            HttpTransport::new(instance).expect("instance URL should be usable"),
        ));

        for sort in SORTS {
            if decoded >= WANTED {
                break;
            }
            let page = client
                .list_posts(sort, ListingType::All, None, 50)
                .unwrap_or_else(|e| panic!("GET {instance}/api/v3/post/list failed: {e}"));

            for view in &page.posts {
                scanned += 1;
                let Some(url) = best_image_url(view) else {
                    continue;
                };
                if !limonada::avif::is_avif(url) {
                    continue;
                }
                found += 1;

                // The viewer must treat it as an ordinary picture: no format
                // explanation, and the URL it picked is the one that decodes.
                let viewer = Viewer::for_post(view)
                    .unwrap_or_else(|| panic!("post {} should have an openable picture", view.post.id));
                assert_eq!(viewer.url, url);
                assert!(
                    viewer.decodable,
                    "{url} is AVIF, which this build decodes through crate::avif"
                );
                assert_eq!(viewer.undecodable_note(), None);

                let (bytes, mime) = fetch(url)
                    .unwrap_or_else(|e| panic!("the AVIF at {url} did not load: {e}"));
                let started = std::time::Instant::now();
                let image = limonada::avif::decode(&bytes).unwrap_or_else(|e| {
                    panic!("a live AVIF did not decode ({url}, {mime}, {} bytes): {e}", bytes.len())
                });
                let elapsed = started.elapsed();

                // A real picture rather than a blank buffer, which is what a
                // decoder failing quietly would hand back.
                let colours: std::collections::HashSet<[u8; 4]> = image
                    .pixels
                    .iter()
                    .step_by(7)
                    .map(|pixel| pixel.to_array())
                    .collect();
                assert!(
                    colours.len() >= 8,
                    "{url} decoded to {}x{} but only {} distinct colours",
                    image.size[0],
                    image.size[1],
                    colours.len()
                );

                println!(
                    "AVIF {url}\n  {} bytes ({mime}) -> {}x{} in {elapsed:?}, {} distinct colours",
                    bytes.len(),
                    image.size[0],
                    image.size[1],
                    colours.len()
                );
                decoded += 1;
            }
        }
    }

    println!(
        "scanned {scanned} posts over {} listings: {found} AVIF pictures found, {decoded} decoded",
        SORTS.len()
    );
    assert!(
        found > 0,
        "no AVIF picture turned up in {scanned} posts across three instances and the listings \
         {SORTS:?}. AVIF really is served (a captured example is in fixtures/pictures), so \
         this is a sampling miss: re-run, or widen the scan to more listings"
    );
}

#[test]
#[ignore = "requires network access to a live Lemmy instance"]
fn an_unreachable_instance_reports_an_error_rather_than_panicking() {
    // A port that should refuse connections. The point is that the failure is a
    // typed, displayable error, not a panic or a hang.
    let client = LemmyClient::new(Arc::new(
        HttpTransport::new("http://127.0.0.1:9").expect("URL is well-formed"),
    ));
    let err = client.site().expect_err("connecting to a closed port must fail");
    assert!(
        err.is_retryable(),
        "a connection failure should be retryable, got {err:?}"
    );
    assert!(
        !err.to_string().is_empty(),
        "the error must have a user-facing message"
    );
    println!("unreachable instance reported: {err}");
}

/// Search against a real instance.
///
/// Read-only, so unlike the write test below it needs no account: it can be run
/// on its own.
#[test]
#[ignore = "requires network access to a live Lemmy instance"]
fn searches_a_live_instance() {
    let client = live_client();

    // Each specific type fills its own array and leaves the other three empty —
    // verified here rather than assumed, because a search that quietly answered
    // for the wrong type would still return rows.
    for search_type in [
        SearchType::Posts,
        SearchType::Communities,
        SearchType::Users,
        SearchType::Comments,
    ] {
        let response = client
            .search("lemmy", search_type, Sort::TopAll, ListingType::All, None, 5)
            .unwrap_or_else(|e| panic!("search {search_type:?} failed: {e}"));

        assert_eq!(
            response.type_.as_deref(),
            Some(search_type.as_query()),
            "the response should echo the requested type"
        );

        let populated = [
            response.posts.len(),
            response.comments.len(),
            response.communities.len(),
            response.users.len(),
        ]
        .into_iter()
        .filter(|count| *count > 0)
        .count();
        assert!(
            populated <= 1,
            "type_={} filled {populated} arrays; only its own should be filled",
            search_type.as_query()
        );
        println!(
            "search type_={}: {} posts, {} comments, {} communities, {} users",
            search_type.as_query(),
            response.posts.len(),
            response.comments.len(),
            response.communities.len(),
            response.users.len()
        );
    }

    // `All` fills every kind at once.
    let all = client
        .search("lemmy", SearchType::All, Sort::TopAll, ListingType::All, None, 3)
        .expect("type_=All");
    let total =
        all.posts.len() + all.comments.len() + all.communities.len() + all.users.len();
    assert!(total > 0, "type_=All returned nothing for a common word");
    println!(
        "search type_=All: {} posts, {} comments, {} communities, {} users",
        all.posts.len(),
        all.comments.len(),
        all.communities.len(),
        all.users.len()
    );

    // No match is an empty result, not an error.
    let none = client
        .search(
            "zzqqxx-no-instance-contains-this-phrase-zzqqxx",
            SearchType::All,
            Sort::TopAll,
            ListingType::All,
            None,
            5,
        )
        .expect("a search with no matches must still succeed");
    assert_eq!(
        none.posts.len() + none.comments.len() + none.communities.len() + none.users.len(),
        0,
        "a nonsense phrase should match nothing"
    );
    println!("a query with no matches returned an empty, successful response");
}

/// The write path against a real instance.
///
/// Ignored, and gated on credentials supplied through the environment, so no
/// account is committed anywhere and the test cannot run by accident:
///
/// ```text
/// LIMONADA_TEST_USER=you LIMONADA_TEST_PASSWORD=... \
///   cargo test --test live_smoke -- --ignored --nocapture signs_in
/// ```
///
/// It signs in, puts a vote on a post and takes it back off, then posts a
/// comment and confirms it appears in the thread. It deliberately does not vote
/// anything down, because an instance may have downvotes disabled, and it never
/// prints the session token.
#[test]
#[ignore = "requires network access and LIMONADA_TEST_USER / LIMONADA_TEST_PASSWORD"]
fn signs_in_votes_and_comments_on_a_live_instance() {
    let (Ok(user), Ok(password)) = (
        std::env::var("LIMONADA_TEST_USER"),
        std::env::var("LIMONADA_TEST_PASSWORD"),
    ) else {
        eprintln!(
            "skipping: set LIMONADA_TEST_USER and LIMONADA_TEST_PASSWORD to run the write smoke test"
        );
        return;
    };

    let client = live_client();
    let login = client
        .login(&user, &password, None)
        .expect("sign in with the supplied credentials");
    let token = login.jwt.expect("a successful sign-in issues a token");
    client.set_token(Some(token));
    client
        .validate_auth()
        .expect("the token just issued should validate");

    // --- vote a post, then take the vote back ----------------------------
    let page = client
        .list_posts(Sort::Hot, ListingType::All, None, 5)
        .expect("feed");
    let post = page.posts.first().expect("a post to vote on").post.clone();
    let before = page.posts[0].counts.score;

    let up = client.like_post(post.id, 1).expect("upvote");
    assert_eq!(
        up.post_view.my_vote,
        Some(1),
        "the instance should record our upvote"
    );

    let withdrawn = client.like_post(post.id, 0).expect("take the vote back");
    assert_eq!(
        withdrawn.post_view.my_vote, None,
        "score 0 must remove the vote, not record a neutral one"
    );
    println!(
        "post {}: score {before} -> {} -> {}",
        post.id, up.post_view.counts.score, withdrawn.post_view.counts.score
    );

    // --- post a comment, then find it in the thread ----------------------
    let body = "Limonada live smoke test — please ignore.";
    let posted = client
        .create_comment(body, post.id, None)
        .expect("post a comment");
    let comment_id = posted.comment_view.comment.id;

    let thread = client
        .list_comments(post.id, CommentSort::New, 50)
        .expect("thread");
    assert!(
        thread
            .comments
            .iter()
            .any(|view| view.comment.id == comment_id),
        "the comment just posted should appear in the thread"
    );
    println!(
        "comment {comment_id} posted on post {} and found in a {} comment thread",
        post.id,
        thread.comments.len()
    );
}
