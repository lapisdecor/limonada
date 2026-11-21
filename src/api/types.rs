//! Serde models for the Lemmy v3 REST API.
//!
//! Every shape here was derived from real captured responses in `fixtures/`
//! (decoded by `tests/api_shapes.rs`), not from memory. Conventions:
//!
//! * `id` and `name` are required — a response without them is not a post or
//!   community and should fail loudly rather than render as a blank row.
//! * Every other field carries `#[serde(default)]`, because instances omit
//!   nullable fields (a user with no avatar simply has no `avatar` key) and
//!   newer server versions add fields.
//! * Unknown fields are ignored (serde's default), so an additive server change
//!   degrades to a missing value instead of a failed decode.

use serde::{Deserialize, Serialize};

/// A user account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Person {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub instance_id: Option<i64>,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub bot_account: bool,
    #[serde(default)]
    pub banned: bool,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub published: Option<String>,
}

impl Person {
    /// What to show in the UI: display name when set, handle otherwise.
    pub fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&self.name)
    }
}

/// A community.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Community {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub instance_id: Option<i64>,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub posting_restricted_to_mods: bool,
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
}

/// A post.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Post {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub url_content_type: Option<String>,
    #[serde(default)]
    pub alt_text: Option<String>,
    #[serde(default)]
    pub creator_id: i64,
    #[serde(default)]
    pub community_id: i64,
    #[serde(default)]
    pub language_id: i64,
    #[serde(default)]
    pub ap_id: Option<String>,
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub featured_community: bool,
    #[serde(default)]
    pub featured_local: bool,
}

/// Vote and comment totals for a post.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PostAggregates {
    #[serde(default)]
    pub post_id: i64,
    /// Server-side comment count.
    ///
    /// Verified caveat: this can exceed the number of comments `comment/list`
    /// returns, because a federated instance counts comments it cannot serve
    /// (an observed lemmy.ml post reported 13 here and returned none). The UI
    /// must therefore not treat an empty thread as an error.
    #[serde(default)]
    pub comments: i64,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub upvotes: i64,
    #[serde(default)]
    pub downvotes: i64,
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub newest_comment_time: Option<String>,
}

/// Metadata about a post's linked image, when the instance supplies it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ImageDetails {
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
    #[serde(default)]
    pub content_type: Option<String>,
}

/// A post plus everything needed to render a row for it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PostView {
    pub post: Post,
    pub creator: Person,
    pub community: Community,
    #[serde(default)]
    pub counts: PostAggregates,
    #[serde(default)]
    pub image_details: Option<ImageDetails>,
    /// `"NotSubscribed" | "Subscribed" | "Pending"`. Typed as `Option<String>`
    /// rather than an enum so an unrecognised value cannot fail the decode.
    #[serde(default)]
    pub subscribed: Option<String>,
    #[serde(default)]
    pub saved: bool,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub unread_comments: i64,
    #[serde(default)]
    pub creator_is_moderator: bool,
    #[serde(default)]
    pub creator_is_admin: bool,
    #[serde(default)]
    pub banned_from_community: bool,
    #[serde(default)]
    pub creator_banned_from_community: bool,
    #[serde(default)]
    pub creator_blocked: bool,
    /// The signed-in user's own vote: `1` up, `-1` down, absent for none.
    ///
    /// Confirmed absent from anonymous responses — a live lemmy.ml 0.19.20
    /// simply omits the key when nobody is signed in — so it is defaulted, and
    /// `None` there must not be read as "the user voted neutral".
    #[serde(default)]
    pub my_vote: Option<i16>,
}

/// A comment.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Comment {
    pub id: i64,
    #[serde(default)]
    pub content: String,
    /// Materialised ancestor path, e.g. `"0.100.200.300"`, where the last
    /// segment is this comment's own id. This is what makes the thread tree a
    /// pure client-side grouping over a single response.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub post_id: i64,
    #[serde(default)]
    pub creator_id: i64,
    #[serde(default)]
    pub language_id: i64,
    #[serde(default)]
    pub ap_id: Option<String>,
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub distinguished: bool,
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub deleted: bool,
}

/// Vote and reply totals for a comment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CommentAggregates {
    #[serde(default)]
    pub comment_id: i64,
    /// Number of direct replies known to the server, used for the collapse
    /// badge so the count is available even though the batch may be truncated.
    #[serde(default)]
    pub child_count: i64,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub upvotes: i64,
    #[serde(default)]
    pub downvotes: i64,
    #[serde(default)]
    pub published: Option<String>,
}

/// A comment plus its author, post, community and counts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CommentView {
    pub comment: Comment,
    pub creator: Person,
    #[serde(default)]
    pub post: Option<Post>,
    #[serde(default)]
    pub community: Option<Community>,
    #[serde(default)]
    pub counts: CommentAggregates,
    #[serde(default)]
    pub subscribed: Option<String>,
    #[serde(default)]
    pub saved: bool,
    #[serde(default)]
    pub creator_is_moderator: bool,
    #[serde(default)]
    pub creator_is_admin: bool,
    #[serde(default)]
    pub banned_from_community: bool,
    #[serde(default)]
    pub creator_banned_from_community: bool,
    #[serde(default)]
    pub creator_blocked: bool,
    /// The signed-in user's own vote on this comment. See [`PostView::my_vote`].
    #[serde(default)]
    pub my_vote: Option<i16>,
}

/// Totals for a community.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CommunityAggregates {
    #[serde(default)]
    pub community_id: i64,
    #[serde(default)]
    pub posts: i64,
    #[serde(default)]
    pub comments: i64,
    #[serde(default)]
    pub subscribers: i64,
    #[serde(default)]
    pub subscribers_local: i64,
    #[serde(default)]
    pub users_active_day: i64,
    #[serde(default)]
    pub users_active_week: i64,
    #[serde(default)]
    pub users_active_month: i64,
    #[serde(default)]
    pub users_active_half_year: i64,
}

/// A community plus its counts and the current user's relationship to it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CommunityView {
    pub community: Community,
    #[serde(default)]
    pub counts: CommunityAggregates,
    #[serde(default)]
    pub subscribed: Option<String>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub banned_from_community: bool,
}

/// Instance identity.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Site {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sidebar: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub published: Option<String>,
}

/// Instance totals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SiteAggregates {
    #[serde(default)]
    pub users: i64,
    #[serde(default)]
    pub posts: i64,
    #[serde(default)]
    pub comments: i64,
    #[serde(default)]
    pub communities: i64,
    #[serde(default)]
    pub users_active_day: i64,
    #[serde(default)]
    pub users_active_week: i64,
    #[serde(default)]
    pub users_active_month: i64,
    #[serde(default)]
    pub users_active_half_year: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SiteView {
    pub site: Site,
    #[serde(default)]
    pub counts: SiteAggregates,
}

// ---------------------------------------------------------------------------
// Response envelopes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GetSiteResponse {
    /// Server version string, e.g. `"0.19.20"`. Shown in the status bar so it
    /// is obvious which API generation we are talking to.
    #[serde(default)]
    pub version: String,
    pub site_view: SiteView,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ListPostsResponse {
    #[serde(default)]
    pub posts: Vec<PostView>,
    /// Opaque pagination cursor for the next page (`"P33813b9"`), absent on the
    /// last page.
    #[serde(default)]
    pub next_page: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ListCommunitiesResponse {
    #[serde(default)]
    pub communities: Vec<CommunityView>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ListCommentsResponse {
    #[serde(default)]
    pub comments: Vec<CommentView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GetPostResponse {
    pub post_view: PostView,
    #[serde(default)]
    pub community_view: Option<CommunityView>,
}

/// Totals for a person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PersonAggregates {
    #[serde(default)]
    pub person_id: i64,
    #[serde(default)]
    pub post_count: i64,
    #[serde(default)]
    pub comment_count: i64,
}

/// A person plus their totals — the element type of a user search.
///
/// Captured from a live lemmy.ml 0.19.20 `search?type_=Users`, whose elements
/// carry exactly `person`, `counts` and `is_admin`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PersonView {
    pub person: Person,
    #[serde(default)]
    pub counts: PersonAggregates,
    #[serde(default)]
    pub is_admin: bool,
}

/// A successful sign-in.
///
/// `jwt` is the session token; the two booleans describe the registration flow
/// the same endpoint can also perform (`register_created`), not whether the
/// sign-in succeeded — a failed sign-in is an HTTP 401 instead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct LoginResponse {
    #[serde(default)]
    pub jwt: Option<String>,
    #[serde(default)]
    pub registration_created: bool,
    #[serde(default)]
    pub verify_email_sent: bool,
}

/// Lemmy's generic `{"success": true}` acknowledgement, used by
/// `user/validate_auth`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SuccessResponse {
    #[serde(default)]
    pub success: bool,
}

/// A post plus its updated view — the answer to a vote.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PostResponse {
    pub post_view: PostView,
}

/// A comment plus its updated view — the answer to a vote or a new comment.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CommentResponse {
    pub comment_view: CommentView,
    /// People the server notified. Unused here, but present in every real
    /// response, so it is modelled rather than left to chance.
    #[serde(default)]
    pub recipient_ids: Vec<i64>,
}

/// The answer to a search, with one array per kind of result.
///
/// Only the array matching the requested `type_` is populated — a live
/// lemmy.ml 0.19.20 `type_=Posts` returned posts and three empty arrays — so an
/// empty array means "not this kind", not "nothing found". The exception is
/// `type_=All`, which fills all four.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SearchResponse {
    /// The search type the server answered for, echoed back (e.g. `"Posts"`).
    #[serde(default)]
    pub type_: Option<String>,
    #[serde(default)]
    pub comments: Vec<CommentView>,
    #[serde(default)]
    pub posts: Vec<PostView>,
    #[serde(default)]
    pub communities: Vec<CommunityView>,
    #[serde(default)]
    pub users: Vec<PersonView>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, display_name: Option<&str>) -> Person {
        Person {
            id: 1,
            name: name.to_string(),
            display_name: display_name.map(str::to_string),
            avatar: None,
            banner: None,
            bio: None,
            actor_id: None,
            instance_id: None,
            local: true,
            bot_account: false,
            banned: false,
            deleted: false,
            published: None,
        }
    }

    /// The UI renders `label()`, so this is the contract for how an account is
    /// named on screen.
    #[test]
    fn person_label_prefers_the_display_name() {
        assert_eq!(person("oxysis", Some("Oxysis/Oxy")).label(), "Oxysis/Oxy");
    }

    #[test]
    fn person_label_falls_back_to_the_handle() {
        assert_eq!(person("oxysis", None).label(), "oxysis");
        assert_eq!(
            person("oxysis", Some("   ")).label(),
            "oxysis",
            "a blank display name must not blank the row"
        );
    }
}
