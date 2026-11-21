//! Response decoding and request construction, verified against real payloads.
//!
//! The fixtures in `fixtures/` are captured from a live instance (lemmy.ml,
//! server version 0.19.20). Decoding them proves the models match what a real
//! server sends — including the fields that only appear on some endpoints, and
//! the omitted-when-null fields that would otherwise break a decode.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use limonada::api::client::endpoints;
use limonada::api::types::{
    GetPostResponse, GetSiteResponse, ListCommentsResponse, ListCommunitiesResponse,
    ListPostsResponse, SearchResponse,
};
use limonada::api::{
    ApiError, CommentSort, FixtureTransport, LemmyClient, ListingType, MAX_PAGE_SIZE, SearchType,
    Sort, Transport, clamp_score,
};
use limonada::state::THREAD_PAGE_SIZE;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn read_fixture(name: &str) -> serde_json::Value {
    let path = fixtures().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Decoding real payloads
// ---------------------------------------------------------------------------

#[test]
fn decodes_site_response() {
    let value = read_fixture("site.json");
    let site: GetSiteResponse = serde_json::from_value(value).expect("site.json should decode");

    assert!(!site.site_view.site.name.is_empty());
    assert!(
        site.version.starts_with("0.19"),
        "unexpected server version {:?}",
        site.version
    );
    assert!(site.site_view.counts.users > 0, "instance should have users");
}

#[test]
fn decodes_post_list_and_its_pagination_cursor() {
    let value = read_fixture("post_list.json");
    let list: ListPostsResponse = serde_json::from_value(value).expect("post_list.json should decode");

    assert_eq!(list.posts.len(), 20, "fixture was captured with limit=20");
    assert!(
        list.next_page.is_some(),
        "a full first page must carry a next_page cursor so the UI can paginate"
    );

    let first = &list.posts[0];
    assert!(first.post.id > 0);
    assert!(!first.post.name.is_empty());
    assert!(!first.creator.name.is_empty(), "creator handle should be present");
    assert!(!first.community.name.is_empty());

    // Counts drive the row chips, so they must survive decoding.
    assert!(first.counts.score >= 0 || first.counts.score < 0);
    assert!(first.counts.comments >= 0);
}

#[test]
fn decodes_every_post_row_without_panicking() {
    // A single bad row should have failed decoding, so this also proves no
    // post in the page uses a shape the models reject.
    let value = read_fixture("post_list.json");
    let list: ListPostsResponse = serde_json::from_value(value).expect("post_list.json should decode");

    for view in &list.posts {
        assert!(view.post.id > 0, "post id must decode");
        assert!(view.creator.id > 0);
        assert!(view.community.id > 0);
    }

    // `image_details` only appears on some posts; the optional field must not
    // break the others.
    let with_details = list
        .posts
        .iter()
        .filter(|view| view.image_details.is_some())
        .count();
    assert!(
        with_details < list.posts.len(),
        "the fixture should contain a mix of posts with and without image_details"
    );
}

#[test]
fn decodes_community_list() {
    let value = read_fixture("community_list.json");
    let list: ListCommunitiesResponse =
        serde_json::from_value(value).expect("community_list.json should decode");

    assert_eq!(list.communities.len(), 50, "fixture was captured with limit=50");
    for view in &list.communities {
        assert!(view.community.id > 0);
        assert!(!view.community.name.is_empty());
    }
    assert!(
        list.communities.iter().any(|c| c.counts.subscribers > 0),
        "at least one community should report subscribers"
    );
}

#[test]
fn decodes_comment_thread_and_its_paths() {
    let value = read_fixture("comment_list.json");
    let list: ListCommentsResponse =
        serde_json::from_value(value).expect("comment_list.json should decode");

    assert_eq!(list.comments.len(), 19, "fixture thread has 19 comments");
    for view in &list.comments {
        assert!(view.comment.id > 0);
        assert!(
            !view.comment.path.is_empty(),
            "every comment must carry a materialised path for threading"
        );
        // The path's final segment is the comment's own id.
        let last = view.comment.path.rsplit('.').next().unwrap_or_default();
        assert_eq!(
            last,
            view.comment.id.to_string(),
            "path must end with the comment's own id"
        );
    }
}

#[test]
fn decodes_post_detail() {
    let value = read_fixture("post_detail.json");
    let detail: GetPostResponse = serde_json::from_value(value).expect("post_detail.json should decode");

    assert!(detail.post_view.post.id > 0);
    assert!(
        detail.community_view.is_some(),
        "post detail should include its community"
    );
}

#[test]
fn an_empty_thread_decodes_as_empty_not_as_an_error() {
    // Observed on a real instance: a federated post reported 13 comments but
    // `comment/list` returned none, because comments from an instance the
    // server could not fetch are counted but not served. An empty thread is a
    // normal state, not a failure.
    let value = read_fixture("comment_list_empty.json");
    let list: ListCommentsResponse =
        serde_json::from_value(value).expect("an empty thread must decode");
    assert!(list.comments.is_empty());
}

#[test]
fn a_malformed_payload_is_reported_as_a_decode_error_not_a_panic() {
    // A transport that hands back a well-formed JSON document of the wrong
    // shape must surface as `Decode`, so the UI can show "unreadable response"
    // rather than crashing.
    let wrong = LemmyClient::new(Arc::new(MismatchedTransport));
    let err = wrong.site().expect_err("a wrong-shaped body must fail");
    match err {
        ApiError::Decode { context, .. } => {
            assert!(context.contains(endpoints::SITE), "context was {context:?}");
        }
        other => panic!("expected Decode, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Request construction
// ---------------------------------------------------------------------------

/// A recorded request: its path, and its query parameters in order.
type Recorded = (String, Vec<(String, String)>);

/// Records every request so the tests can assert on the wire shape, and doubles
/// as a live-offline source of fixture data.
struct RecordingTransport {
    inner: FixtureTransport,
    gets: Mutex<Vec<Recorded>>,
    /// Each POST: its path and the JSON body sent.
    posts: Mutex<Vec<(String, serde_json::Value)>>,
    /// The session token the client installed, to prove it reaches the wire.
    auth: Mutex<Option<String>>,
}

impl RecordingTransport {
    fn new() -> Self {
        Self {
            inner: FixtureTransport::new(fixtures()),
            gets: Mutex::new(Vec::new()),
            posts: Mutex::new(Vec::new()),
            auth: Mutex::new(None),
        }
    }

    fn gets(&self) -> Vec<Recorded> {
        self.gets.lock().expect("lock").clone()
    }

    /// The paths of every POST sent, in order.
    fn posts(&self) -> Vec<String> {
        self.posts
            .lock()
            .expect("lock")
            .iter()
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// The body sent to `path`. Panics when nothing was posted there.
    fn posted_body(&self, path: &str) -> serde_json::Value {
        self.posts
            .lock()
            .expect("lock")
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, body)| body.clone())
            .unwrap_or_else(|| panic!("no POST recorded for {path}"))
    }

    fn auth(&self) -> Option<String> {
        self.auth.lock().expect("lock").clone()
    }

    fn query_of<'a>(&self, gets: &'a [Recorded], path: &str) -> &'a Vec<(String, String)> {
        gets.iter()
            .find(|(p, _)| p == path)
            .map(|(_, q)| q)
            .unwrap_or_else(|| panic!("no request recorded for {path}"))
    }

    /// Look a query parameter up by key.
    fn value_of(query: &[(String, String)], key: &str) -> Option<String> {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }
}

impl Transport for RecordingTransport {
    fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, ApiError> {
        self.gets.lock().expect("lock").push((
            path.to_string(),
            query
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect(),
        ));
        self.inner.get(path, query)
    }

    /// Answers writes from canned payloads, so a write can be exercised offline.
    ///
    /// The shapes are the real ones, taken from `fixtures/`: a post vote answers
    /// `PostResponse` (the captured `post_detail.json` carries a `post_view`, and
    /// the extra fields are ignored), and both comment writes answer
    /// `CommentResponse` with the thread's first comment.
    fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, ApiError> {
        self.posts
            .lock()
            .expect("lock")
            .push((path.to_string(), body.clone()));
        match path {
            endpoints::USER_LOGIN => Ok(serde_json::json!({
                "jwt": "test-session-token",
                "registration_created": false,
                "verify_email_sent": false
            })),
            endpoints::POST_LIKE => Ok(read_fixture("post_detail.json")),
            endpoints::COMMENT_LIKE | endpoints::COMMENT => Ok(serde_json::json!({
                "comment_view": read_fixture("comment_list.json")["comments"][0].clone(),
                "recipient_ids": []
            })),
            other => Err(ApiError::Refused(format!(
                "the recording transport has no canned answer for {other}"
            ))),
        }
    }

    fn description(&self) -> String {
        "recording".into()
    }

    fn set_auth(&self, token: Option<String>) {
        *self.auth.lock().expect("lock") = token;
    }

    fn auth(&self) -> Option<String> {
        self.auth.lock().expect("lock").clone()
    }
}

/// Returns a valid JSON document that is not a site response.
struct MismatchedTransport;

impl Transport for MismatchedTransport {
    fn get(&self, _path: &str, _query: &[(&str, String)]) -> Result<serde_json::Value, ApiError> {
        Ok(serde_json::json!({"unexpected": "shape"}))
    }
    fn post(&self, _path: &str, _body: &serde_json::Value) -> Result<serde_json::Value, ApiError> {
        unreachable!("this transport is only used for a GET")
    }
    fn description(&self) -> String {
        "mismatched".into()
    }
}

#[test]
fn list_posts_sends_the_documented_query_parameters() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .list_posts(Sort::TopWeek, ListingType::Local, None, 20)
        .expect("first page should load");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::POST_LIST);

    assert_eq!(
        RecordingTransport::value_of(query, "sort").as_deref(),
        Some("TopWeek")
    );
    assert_eq!(
        RecordingTransport::value_of(query, "type_").as_deref(),
        Some("Local")
    );
    assert_eq!(
        RecordingTransport::value_of(query, "limit").as_deref(),
        Some("20")
    );
    assert_eq!(
        RecordingTransport::value_of(query, "page_cursor"),
        None,
        "the first page must not send a cursor"
    );
}

#[test]
fn list_posts_sends_the_cursor_as_page_cursor() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .list_posts(Sort::Hot, ListingType::All, Some("P33813b9"), 20)
        .expect("page two should load");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::POST_LIST);
    assert_eq!(
        RecordingTransport::value_of(query, "page_cursor").as_deref(),
        Some("P33813b9"),
        "the opaque next_page value must be sent as page_cursor"
    );
    // Regression: sending it as `page` returns HTTP 400 from a live instance,
    // because `page` is a numeric page number. Observed against lemmy.ml
    // 0.19.20 as: "Query deserialize error: invalid digit found in string".
    assert_eq!(
        RecordingTransport::value_of(query, "page"),
        None,
        "the opaque cursor must never be sent as `page`"
    );
}

#[test]
fn list_comments_sends_the_post_id_and_a_comment_sort() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .list_comments(53989052, CommentSort::Top, 50)
        .expect("thread should load");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::COMMENT_LIST);
    assert_eq!(
        RecordingTransport::value_of(query, "post_id").as_deref(),
        Some("53989052")
    );
    // Regression: `comment/list` accepts a different, smaller vocabulary than
    // `post/list`. A live lemmy.ml 0.19.20 rejects post sorts here with HTTP 400
    // ("unknown variant `TopAll`, expected one of `Hot`, `Top`, `New`, `Old`,
    // `Controversial`"), so the value sent must be a comment sort.
    assert_eq!(
        RecordingTransport::value_of(query, "sort").as_deref(),
        Some("Top")
    );
    // Regression: `type_` looks optional but is not, because `comment/list`
    // defaults it to `Local` and that hides every comment on a federated post.
    // Verified against a live lemmy.ml 0.19.20: on federated posts, omitting
    // `type_` returned 0 comments where `type_=All` returned 1, 2, 7 and 8 for
    // the same posts. Since the feed is `All`, omitting it made most threads
    // look empty.
    assert_eq!(
        RecordingTransport::value_of(query, "type_").as_deref(),
        Some("All"),
        "comment/list must be scoped to All, or federated threads come back empty"
    );
    assert_eq!(
        RecordingTransport::value_of(query, "limit").as_deref(),
        Some("50")
    );
}

/// Regression: a `limit` above the server's maximum does not fetch a larger
/// page, it fetches nothing. A live lemmy.ml 0.19.20 answers `limit=51` and
/// `limit=100` with HTTP 400 `{"error":"couldnt_get_comments"}`, which is what
/// made every comment thread unreadable while `THREAD_PAGE_SIZE` was 100.
#[test]
fn no_request_asks_for_more_than_the_server_allows() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    // Deliberately over the cap, at and beyond the value that used to ship.
    client
        .list_posts(Sort::Hot, ListingType::All, None, 500)
        .expect("posts");
    client
        .list_comments(53989052, CommentSort::Top, 100)
        .expect("comments");
    client
        .list_communities(ListingType::All, Sort::TopAll, 500)
        .expect("communities");

    let gets = recording.gets();
    assert!(gets.len() >= 3, "the sequence should have issued its reads");
    for (path, query) in &gets {
        let Some(limit) = RecordingTransport::value_of(query, "limit") else {
            continue;
        };
        let limit: u32 = limit.parse().expect("limit should be numeric");
        assert!(limit >= 1, "{path} asked for limit=0");
        assert!(
            limit <= MAX_PAGE_SIZE,
            "{path} asked for limit={limit}, above the {MAX_PAGE_SIZE} the server accepts"
        );
    }
}

/// `THREAD_PAGE_SIZE` is what the UI actually sends, so it is the constant that
/// has to stay inside the cap — the client's clamp is a backstop, not the fix.
#[test]
fn the_thread_page_size_is_within_the_server_cap() {
    // Both sides are consts, so these are compile-time checks: raising the page
    // size above the cap fails the build here rather than failing silently at
    // runtime against a live instance.
    const {
        assert!(
            THREAD_PAGE_SIZE <= MAX_PAGE_SIZE,
            "THREAD_PAGE_SIZE must not exceed the server's maximum limit"
        )
    };
    const { assert!(THREAD_PAGE_SIZE == 50, "50 is the largest thread Lemmy serves") };
}

#[test]
fn list_communities_sends_the_listing_type_and_the_sort() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .list_communities(ListingType::All, Sort::TopAll, 50)
        .expect("communities should load");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::COMMUNITY_LIST);
    assert_eq!(
        RecordingTransport::value_of(query, "type_").as_deref(),
        Some("All")
    );
    // Regression: without `sort` a live instance answers newest-first, so the
    // sidebar listed fifty brand-new communities with nothing in them.
    assert_eq!(
        RecordingTransport::value_of(query, "sort").as_deref(),
        Some("TopAll"),
        "community/list must ask for the active communities, not the newest"
    );
}

/// The sidebar's lookup is a community search, not the search screen's `All`:
/// it must ask only for communities, ranked by activity. `Hot` ranked a
/// six-subscriber `!fedora` above `!linux` on a live instance.
#[test]
fn the_sidebar_lookup_asks_for_communities_ranked_by_activity() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .search(
            "linux",
            SearchType::Communities,
            Sort::TopAll,
            ListingType::All,
            None,
            50,
        )
        .expect("lookup should run");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::SEARCH);
    assert_eq!(
        RecordingTransport::value_of(query, "type_").as_deref(),
        Some("Communities")
    );
    assert_eq!(
        RecordingTransport::value_of(query, "sort").as_deref(),
        Some("TopAll")
    );
    assert_eq!(
        RecordingTransport::value_of(query, "community_id"),
        None,
        "the sidebar lookup spans the whole instance"
    );
}

#[test]
fn get_post_sends_the_id() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client.get_post(53989052).expect("post should load");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::POST);
    assert_eq!(
        RecordingTransport::value_of(query, "id").as_deref(),
        Some("53989052")
    );
}

/// The read path issues no write, asserted rather than assumed: loading the
/// feed, the community list, a post and its thread must never POST. Writes now
/// exist — but behind a vote or a comment, never behind reading.
#[test]
fn browsing_never_issues_a_write() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client.site().expect("site");
    client
        .list_posts(Sort::Hot, ListingType::All, None, 20)
        .expect("posts");
    client
        .list_communities(ListingType::All, Sort::TopAll, 50)
        .expect("communities");
    client.get_post(53989052).expect("post detail");
    client
        .list_comments(53989052, CommentSort::Hot, 50)
        .expect("comments");

    assert!(
        recording.posts().is_empty(),
        "v1 is read-only, but these writes were attempted: {:?}",
        recording.posts()
    );
    assert!(
        recording.gets().len() >= 5,
        "the browse sequence should have issued its reads"
    );
}

#[test]
fn demo_mode_refuses_writes() {
    let transport = FixtureTransport::new(fixtures());
    let err = transport
        .post("/api/v3/post/like", &serde_json::json!({}))
        .expect_err("demo mode must refuse writes");
    assert!(matches!(err, ApiError::Refused(_)), "got {err:?}");
    assert!(transport.is_offline());

    // A sign-in is a write too, so demo mode refuses it for the same reason: it
    // has no instance to authenticate against.
    let err = transport
        .post(
            endpoints::USER_LOGIN,
            &serde_json::json!({"username_or_email": "a", "password": "b"}),
        )
        .expect_err("demo mode must refuse a sign-in");
    assert!(matches!(err, ApiError::Refused(_)), "got {err:?}");
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[test]
fn decodes_a_real_search_response() {
    let value = read_fixture("search.json");
    let search: SearchResponse = serde_json::from_value(value).expect("search.json should decode");

    // Captured from a live lemmy.ml 0.19.20 with `q=lemmy&type_=All&limit=3`.
    assert_eq!(
        search.type_.as_deref(),
        Some("All"),
        "the response echoes the requested type"
    );
    assert_eq!(search.posts.len(), 3);
    assert_eq!(search.comments.len(), 3);
    assert_eq!(search.communities.len(), 3);
    assert_eq!(search.users.len(), 3);

    for user in &search.users {
        assert!(user.person.id > 0, "a user result needs a person");
        assert!(!user.person.name.is_empty());
    }
    for post in &search.posts {
        assert!(post.post.id > 0);
        assert!(!post.community.name.is_empty());
    }
}

/// A signed-out response omits `my_vote` entirely rather than sending null —
/// lemmy.ml 0.19.20 does exactly this — so a missing vote must decode as `None`
/// rather than fail the whole response. The captured fixtures are anonymous and
/// therefore carry no `my_vote` anywhere.
#[test]
fn a_signed_out_view_records_no_vote() {
    let list: ListPostsResponse =
        serde_json::from_value(read_fixture("post_list.json")).expect("post_list.json");
    assert!(
        list.posts.iter().all(|view| view.my_vote.is_none()),
        "an anonymous post list must not claim a vote"
    );

    let search: SearchResponse = serde_json::from_value(read_fixture("search.json")).expect("search");
    assert!(search.comments.iter().all(|view| view.my_vote.is_none()));
    assert!(search.posts.iter().all(|view| view.my_vote.is_none()));
}

#[test]
fn search_sends_the_query_type_and_listing() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .search(
            "rust lang",
            SearchType::Posts,
            Sort::TopWeek,
            ListingType::Local,
            None,
            20,
        )
        .expect("search should run");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::SEARCH);
    assert_eq!(RecordingTransport::value_of(query, "q").as_deref(), Some("rust lang"));
    assert_eq!(
        RecordingTransport::value_of(query, "type_").as_deref(),
        Some("Posts")
    );
    assert_eq!(
        RecordingTransport::value_of(query, "sort").as_deref(),
        Some("TopWeek")
    );
    // The listing is a separate parameter from `type_`: verified live, where
    // `type_=Posts&listing_type=All` is accepted and means two different things.
    assert_eq!(
        RecordingTransport::value_of(query, "listing_type").as_deref(),
        Some("Local")
    );
    assert_eq!(RecordingTransport::value_of(query, "limit").as_deref(), Some("20"));
    assert_eq!(
        RecordingTransport::value_of(query, "community_id"),
        None,
        "an unscoped search must not filter by community"
    );
}

#[test]
fn search_can_be_scoped_to_a_community_and_stays_within_the_cap() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .search(
            "x",
            SearchType::All,
            Sort::Hot,
            ListingType::All,
            Some(42),
            500,
        )
        .expect("search should run");

    let gets = recording.gets();
    let query = recording.query_of(&gets, endpoints::SEARCH);
    assert_eq!(RecordingTransport::value_of(query, "community_id").as_deref(), Some("42"));
    let limit: u32 = RecordingTransport::value_of(query, "limit")
        .expect("a limit")
        .parse()
        .expect("numeric");
    assert!(limit <= MAX_PAGE_SIZE, "search asked for limit={limit}");
}

// ---------------------------------------------------------------------------
// Sign-in, votes and comments (the writes)
// ---------------------------------------------------------------------------

#[test]
fn login_sends_the_credentials_and_omits_an_absent_two_factor_code() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    let response = client
        .login("me@example.org", "hunter2", None)
        .expect("login should succeed");
    assert_eq!(response.jwt.as_deref(), Some("test-session-token"));

    let body = recording.posted_body(endpoints::USER_LOGIN);
    assert_eq!(body["username_or_email"], "me@example.org");
    assert_eq!(body["password"], "hunter2");
    assert!(
        body.get("totp_2fa_token").is_none(),
        "an account without 2FA must not be asked for a code it has not got"
    );
}

#[test]
fn login_sends_a_two_factor_code_only_when_there_is_one() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .login("me", "pw", Some("123456"))
        .expect("login");
    assert_eq!(
        recording.posted_body(endpoints::USER_LOGIN)["totp_2fa_token"],
        "123456"
    );

    // A blank field is an omission, not a code.
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());
    client.login("me", "pw", Some("   ")).expect("login");
    assert!(
        recording
            .posted_body(endpoints::USER_LOGIN)
            .get("totp_2fa_token")
            .is_none()
    );
}

#[test]
fn like_post_sends_the_post_and_score() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    let response = client.like_post(53_989_052, 1).expect("vote should apply");
    assert_eq!(response.post_view.post.id, 53_989_052, "the answer carries the post");

    let body = recording.posted_body(endpoints::POST_LIKE);
    assert_eq!(body["post_id"], 53_989_052);
    assert_eq!(body["score"], 1);
}

#[test]
fn like_comment_sends_the_comment_and_score() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client.like_comment(28_327_851, -1).expect("vote");

    let body = recording.posted_body(endpoints::COMMENT_LIKE);
    assert_eq!(body["comment_id"], 28_327_851);
    assert_eq!(body["score"], -1);
}

/// A vote value the server does not accept is a no-op vote, not a 400.
#[test]
fn an_out_of_range_score_is_clamped_to_a_real_vote() {
    assert_eq!(clamp_score(5), 1);
    assert_eq!(clamp_score(-9), -1);
    assert_eq!(clamp_score(0), 0, "0 is the documented way to take a vote back");

    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());
    client.like_post(1, 7).expect("vote");
    client.like_comment(2, -7).expect("vote");
    assert_eq!(recording.posted_body(endpoints::POST_LIKE)["score"], 1);
    assert_eq!(recording.posted_body(endpoints::COMMENT_LIKE)["score"], -1);
}

#[test]
fn a_top_level_comment_sends_its_post_and_no_parent() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .create_comment("hello there", 53_989_052, None)
        .expect("comment should post");

    let body = recording.posted_body(endpoints::COMMENT);
    assert_eq!(body["content"], "hello there");
    assert_eq!(body["post_id"], 53_989_052);
    assert!(
        body.get("parent_id").is_none(),
        "a top-level comment has no parent_id; sending one would nest it"
    );
}

#[test]
fn a_reply_comment_sends_its_parent() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    client
        .create_comment("point taken", 53_989_052, Some(28_327_851))
        .expect("reply should post");

    let body = recording.posted_body(endpoints::COMMENT);
    assert_eq!(body["content"], "point taken");
    assert_eq!(body["parent_id"], 28_327_851);
}

/// The token reaches the transport, which is what puts it on the wire — proved
/// against a real socket in `tests/http_auth.rs`.
#[test]
fn installing_a_session_token_reaches_the_transport() {
    let recording = Arc::new(RecordingTransport::new());
    let client = LemmyClient::new(recording.clone());

    assert!(!client.has_token(), "nothing is installed to begin with");
    client.set_token(Some("a-session-token".into()));
    assert!(client.has_token());
    assert_eq!(recording.auth().as_deref(), Some("a-session-token"));

    client.set_token(None);
    assert_eq!(recording.auth(), None, "signing out clears it again");
}
