//! Typed Lemmy v3 client.
//!
//! Every method is blocking and returns a decoded model. The UI never calls
//! these directly on its own thread — see [`super::job::spawn`].
//!
//! Reads are anonymous; the four writes this client exposes — `login`,
//! `like_post`, `like_comment` and `create_comment` — all require a session, so
//! the read path and the write path stay visibly separate. `tests/api_shapes.rs`
//! asserts that browsing issues no POST at all, and that each write puts the
//! documented body on the wire.

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::ApiError;
use super::params::{CommentSort, ListingType, SearchType, Sort};
use super::transport::Transport;
use super::types::{
    CommentResponse, GetPostResponse, GetSiteResponse, ListCommentsResponse,
    ListCommunitiesResponse, ListPostsResponse, LoginResponse, PostResponse, SearchResponse,
    SuccessResponse,
};

/// API paths, defined once so tests can assert on them.
pub mod endpoints {
    pub const SITE: &str = "/api/v3/site";
    pub const POST_LIST: &str = "/api/v3/post/list";
    pub const POST: &str = "/api/v3/post";
    pub const COMMENT_LIST: &str = "/api/v3/comment/list";
    pub const COMMUNITY_LIST: &str = "/api/v3/community/list";
    pub const SEARCH: &str = "/api/v3/search";
    pub const USER_LOGIN: &str = "/api/v3/user/login";
    pub const USER_VALIDATE_AUTH: &str = "/api/v3/user/validate_auth";
    pub const POST_LIKE: &str = "/api/v3/post/like";
    pub const COMMENT_LIKE: &str = "/api/v3/comment/like";
    /// Also the create-a-comment endpoint; v3 spells creation as a bare POST.
    pub const COMMENT: &str = "/api/v3/comment";
}

/// Default number of items per page.
pub const DEFAULT_PAGE_SIZE: u32 = 20;

/// The largest `limit` a Lemmy instance accepts.
///
/// Verified against a live lemmy.ml 0.19.20: `comment/list?limit=50` returns
/// 200, while `limit=51` and `limit=100` both return HTTP 400
/// `{"error":"couldnt_get_comments"}`. A larger page size does not fetch a
/// larger page — it fetches nothing — so every request is clamped to this.
pub const MAX_PAGE_SIZE: u32 = 50;

/// Clamp a requested page size into the range the server accepts.
///
/// This turns an out-of-range constant into a smaller page rather than a
/// request that fails, so the worst case is a bounded inefficiency instead of a
/// feature that silently stops working.
fn page_size(limit: u32) -> u32 {
    limit.clamp(1, MAX_PAGE_SIZE)
}

/// The three vote values Lemmy's `score` field accepts.
///
/// The server's own comment on that field is "-1, 0, or 1", where `0` removes an
/// existing vote rather than recording a neutral one. Clamping here makes an
/// out-of-range value a no-op vote instead of an HTTP 400.
pub fn clamp_score(score: i16) -> i16 {
    score.clamp(-1, 1)
}

#[derive(Clone)]
pub struct LemmyClient {
    transport: Arc<dyn Transport>,
}

impl LemmyClient {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }

    /// The transport in use, for status text and capability checks.
    pub fn transport(&self) -> &dyn Transport {
        self.transport.as_ref()
    }

    /// Install (or clear) the session token on the transport.
    ///
    /// Sent as `Authorization: Bearer <token>`: the 0.19.x server reads the
    /// token from that header (verified against lemmy.ml 0.19.20, where an
    /// unauthenticated write answers HTTP 401 `{"error":"incorrect_login"}` and
    /// the same request with a token does not).
    pub fn set_token(&self, token: Option<String>) {
        self.transport.set_auth(token);
    }

    /// Whether a session token is currently installed.
    pub fn has_token(&self) -> bool {
        self.transport.auth().is_some_and(|t| !t.is_empty())
    }

    /// Instance identity and version.
    pub fn site(&self) -> Result<GetSiteResponse, ApiError> {
        let value = self.transport.get(endpoints::SITE, &[])?;
        decode(endpoints::SITE, value)
    }

    /// A page of posts. `cursor` is the opaque `next_page` value from a previous
    /// page; `None` requests the first page.
    ///
    /// The cursor is sent as `page_cursor`, not `page`: `page` is a numeric page
    /// number, and passing the opaque value there fails with HTTP 400
    /// "Query deserialize error: invalid digit found in string". Verified
    /// against a live lemmy.ml 0.19.20, where `page_cursor=<next_page>` returns
    /// the next page and its own fresh cursor, while a `cursor=` parameter is
    /// silently ignored. `tests/api_shapes.rs` pins this.
    pub fn list_posts(
        &self,
        sort: Sort,
        listing: ListingType,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ListPostsResponse, ApiError> {
        self.post_list(sort, listing, cursor, limit, None)
    }

    /// A page of posts restricted to one community.
    ///
    /// `post/list` accepts a `community_id` filter (confirmed against a live
    /// instance), which is what makes the community sidebar navigable rather
    /// than decorative.
    pub fn list_posts_in_community(
        &self,
        sort: Sort,
        community_id: i64,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ListPostsResponse, ApiError> {
        self.post_list(sort, ListingType::All, cursor, limit, Some(community_id))
    }

    fn post_list(
        &self,
        sort: Sort,
        listing: ListingType,
        cursor: Option<&str>,
        limit: u32,
        community_id: Option<i64>,
    ) -> Result<ListPostsResponse, ApiError> {
        let mut query = post_list_query(sort, listing, limit);
        if let Some(id) = community_id {
            query.push(("community_id", id.to_string()));
        }
        if let Some(cursor) = cursor.filter(|c| !c.is_empty()) {
            query.push(("page_cursor", cursor.to_string()));
        }
        let value = self.transport.get(endpoints::POST_LIST, &query)?;
        decode(endpoints::POST_LIST, value)
    }

    /// Communities known to the instance, in the order `sort` asks for.
    ///
    /// The `sort` parameter is not optional in effect. A live lemmy.ml 0.19.20
    /// answers `community/list` without it by ordering on **creation date**, so
    /// the fifty entries a sidebar loads were the fifty newest communities —
    /// measured at 31 with zero posts and 41 with two or fewer — and widely
    /// known communities such as `!technology` and `!linux` were absent
    /// entirely. `sort=TopAll` returned fifty communities with none of them
    /// empty and the well-known ones first, which is the directory the sidebar
    /// is supposed to be.
    pub fn list_communities(
        &self,
        listing: ListingType,
        sort: Sort,
        limit: u32,
    ) -> Result<ListCommunitiesResponse, ApiError> {
        let query = vec![
            ("type_", listing.as_query().to_string()),
            ("sort", sort.as_query().to_string()),
            ("limit", page_size(limit).to_string()),
        ];
        let value = self.transport.get(endpoints::COMMUNITY_LIST, &query)?;
        decode(endpoints::COMMUNITY_LIST, value)
    }

    /// A single post, with its community.
    pub fn get_post(&self, id: i64) -> Result<GetPostResponse, ApiError> {
        let query = vec![("id", id.to_string())];
        let value = self.transport.get(endpoints::POST, &query)?;
        decode(endpoints::POST, value)
    }

    /// The full comment thread for a post, as a flat list.
    ///
    /// Takes a [`CommentSort`], not a [`Sort`]: `comment/list` accepts a
    /// different and smaller sort vocabulary than `post/list`, and rejects the
    /// post values with HTTP 400.
    ///
    /// Note the server's `max_depth` default means very deep threads may be
    /// truncated; [`crate::tree::build_threads`] promotes any comment whose
    /// parent is missing rather than dropping it.
    ///
    /// `type_` is passed even though the endpoint would accept the request
    /// without it, because it is **not** optional in effect: `comment/list`
    /// defaults it to `Local`, which hides every comment on a federated post.
    /// Verified against a live lemmy.ml 0.19.20 — on federated posts, omitting
    /// `type_` returned 0 comments where `type_=All` returned 1, 2, 7 and 8 for
    /// the same posts, while local posts were unaffected either way. Since the
    /// default feed is `All`, omitting it would make most threads look empty.
    /// A thread is always requested with `All` regardless of the session: the
    /// comments of one post are not a listing the user picks a scope for.
    pub fn list_comments(
        &self,
        post_id: i64,
        sort: CommentSort,
        limit: u32,
    ) -> Result<ListCommentsResponse, ApiError> {
        let query = vec![
            ("post_id", post_id.to_string()),
            ("sort", sort.as_query().to_string()),
            ("type_", ListingType::All.as_query().to_string()),
            ("limit", page_size(limit).to_string()),
        ];
        let value = self.transport.get(endpoints::COMMENT_LIST, &query)?;
        decode(endpoints::COMMENT_LIST, value)
    }

    /// Search the instance.
    ///
    /// `type_` selects which of the four result arrays the server fills; only
    /// that one is populated unless it is `All`. `listing_type` is a separate
    /// parameter from `type_` here — a live lemmy.ml 0.19.20 accepts
    /// `listing_type=All` alongside `type_=Posts`, and they mean different
    /// things. `limit` goes through the same cap as every other request.
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        query: &str,
        search_type: SearchType,
        sort: Sort,
        listing: ListingType,
        community_id: Option<i64>,
        limit: u32,
    ) -> Result<SearchResponse, ApiError> {
        let mut params = vec![
            ("q", query.to_string()),
            ("type_", search_type.as_query().to_string()),
            ("sort", sort.as_query().to_string()),
            ("listing_type", listing.as_query().to_string()),
            ("limit", page_size(limit).to_string()),
        ];
        if let Some(id) = community_id {
            params.push(("community_id", id.to_string()));
        }
        let value = self.transport.get(endpoints::SEARCH, &params)?;
        decode(endpoints::SEARCH, value)
    }

    // -----------------------------------------------------------------------
    // Writes — every one of these needs a session
    // -----------------------------------------------------------------------

    /// Sign in and obtain a session token.
    ///
    /// `totp_2fa_token` is only sent when non-empty, so an account without
    /// 2FA is not asked for a code it does not have. A failed sign-in is an
    /// HTTP 401, which the transport reports as [`ApiError::Unauthorized`].
    pub fn login(
        &self,
        username_or_email: &str,
        password: &str,
        totp_2fa_token: Option<&str>,
    ) -> Result<LoginResponse, ApiError> {
        let mut body = serde_json::json!({
            "username_or_email": username_or_email,
            "password": password,
        });
        if let Some(token) = totp_2fa_token.filter(|token| !token.trim().is_empty()) {
            body["totp_2fa_token"] = Value::String(token.trim().to_string());
        }
        let value = self.transport.post(endpoints::USER_LOGIN, &body)?;
        decode(endpoints::USER_LOGIN, value)
    }

    /// Check that the installed token is still valid.
    ///
    /// Answers `{"success": true}` on success, and HTTP 400 with
    /// `{"error":"not_logged_in"}` when there is no usable token — verified
    /// against a live lemmy.ml 0.19.20, where an unauthorised call to this
    /// endpoint returns that error with 400 rather than the 401 the write
    /// endpoints use.
    pub fn validate_auth(&self) -> Result<SuccessResponse, ApiError> {
        let value = self.transport.get(endpoints::USER_VALIDATE_AUTH, &[])?;
        decode(endpoints::USER_VALIDATE_AUTH, value)
    }

    /// Upvote, downvote or un-vote a post.
    pub fn like_post(&self, post_id: i64, score: i16) -> Result<PostResponse, ApiError> {
        let body = serde_json::json!({
            "post_id": post_id,
            "score": clamp_score(score),
        });
        let value = self.transport.post(endpoints::POST_LIKE, &body)?;
        decode(endpoints::POST_LIKE, value)
    }

    /// Upvote, downvote or un-vote a comment.
    pub fn like_comment(&self, comment_id: i64, score: i16) -> Result<CommentResponse, ApiError> {
        let body = serde_json::json!({
            "comment_id": comment_id,
            "score": clamp_score(score),
        });
        let value = self.transport.post(endpoints::COMMENT_LIKE, &body)?;
        decode(endpoints::COMMENT_LIKE, value)
    }

    /// Post a comment, either at the top of a thread or as a reply.
    pub fn create_comment(
        &self,
        content: &str,
        post_id: i64,
        parent_id: Option<i64>,
    ) -> Result<CommentResponse, ApiError> {
        let mut body = serde_json::json!({
            "content": content,
            "post_id": post_id,
        });
        if let Some(parent) = parent_id {
            body["parent_id"] = Value::Number(parent.into());
        }
        let value = self.transport.post(endpoints::COMMENT, &body)?;
        decode(endpoints::COMMENT, value)
    }
}

fn post_list_query(sort: Sort, listing: ListingType, limit: u32) -> Vec<(&'static str, String)> {
    // `type_` really is spelled with a trailing underscore in the v3 API.
    vec![
        ("sort", sort.as_query().to_string()),
        ("type_", listing.as_query().to_string()),
        ("limit", page_size(limit).to_string()),
    ]
}

fn decode<T: DeserializeOwned>(path: &str, value: Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(|e| ApiError::Decode {
        context: format!("decoding the response from {path}"),
        detail: e.to_string(),
    })
}
