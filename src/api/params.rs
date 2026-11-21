//! Query parameters shared by the API client and the UI.
//!
//! Kept separate from the API models so the UI can hold a `Sort` in its state
//! without depending on the wire format, and so the query strings have exactly
//! one definition.

/// Lemmy v3 `sort` query values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Sort {
    Active,
    #[default]
    Hot,
    New,
    Old,
    TopDay,
    TopWeek,
    TopMonth,
    TopYear,
    TopAll,
    MostComments,
    NewComments,
    Controversial,
    Scaled,
}

impl Sort {
    /// Every sort, in the order the UI offers them.
    pub const ALL: [Self; 13] = [
        Self::Hot,
        Self::Active,
        Self::New,
        Self::Old,
        Self::MostComments,
        Self::NewComments,
        Self::Controversial,
        Self::Scaled,
        Self::TopDay,
        Self::TopWeek,
        Self::TopMonth,
        Self::TopYear,
        Self::TopAll,
    ];

    /// The exact string Lemmy expects in the `sort` query parameter.
    pub fn as_query(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Hot => "Hot",
            Self::New => "New",
            Self::Old => "Old",
            Self::TopDay => "TopDay",
            Self::TopWeek => "TopWeek",
            Self::TopMonth => "TopMonth",
            Self::TopYear => "TopYear",
            Self::TopAll => "TopAll",
            Self::MostComments => "MostComments",
            Self::NewComments => "NewComments",
            Self::Controversial => "Controversial",
            Self::Scaled => "Scaled",
        }
    }

    /// Human-facing label for the sort picker.
    pub fn label(self) -> &'static str {
        match self {
            Self::TopDay => "Top today",
            Self::TopWeek => "Top this week",
            Self::TopMonth => "Top this month",
            Self::TopYear => "Top this year",
            Self::TopAll => "Top all time",
            Self::MostComments => "Most comments",
            Self::NewComments => "New comments",
            Self::Controversial => "Controversial",
            Self::Scaled => "Scaled",
            Self::Active => "Active",
            Self::Hot => "Hot",
            Self::New => "New",
            Self::Old => "Old",
        }
    }
}

/// Comment sorts, accepted by `comment/list`.
///
/// Comments have their own, smaller sort vocabulary than posts — this is not a
/// subset of [`Sort`]. Verified against a live lemmy.ml 0.19.20: `comment/list`
/// accepts exactly `Hot`, `Top`, `New`, `Old` and `Controversial`, and rejects
/// every other post sort with HTTP 400 (`unknown variant`). Note `Top` here,
/// where posts use `TopAll`.
///
/// Keeping this as a separate type makes an invalid comment sort
/// unrepresentable rather than a runtime 400.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CommentSort {
    Hot,
    #[default]
    Top,
    New,
    Old,
    Controversial,
}

impl CommentSort {
    /// Every comment sort, in the order the UI offers them.
    pub const ALL: [Self; 5] = [
        Self::Top,
        Self::Hot,
        Self::New,
        Self::Old,
        Self::Controversial,
    ];

    /// The exact string `comment/list` expects in `sort`.
    pub fn as_query(self) -> &'static str {
        match self {
            Self::Hot => "Hot",
            Self::Top => "Top",
            Self::New => "New",
            Self::Old => "Old",
            Self::Controversial => "Controversial",
        }
    }

    /// Human-facing label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hot => "Hot",
            Self::Top => "Top",
            Self::New => "New",
            Self::Old => "Old",
            Self::Controversial => "Controversial",
        }
    }
}

/// Lemmy v3 `type_` listing scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ListingType {
    #[default]
    All,
    Local,
    Subscribed,
}

impl ListingType {
    pub const ALL: [Self; 3] = [Self::All, Self::Local, Self::Subscribed];

    pub fn as_query(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Local => "Local",
            Self::Subscribed => "Subscribed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Local => "Local",
            Self::Subscribed => "Subscribed",
        }
    }
}

/// Lemmy v3 `search` result filter.
///
/// Verified against a live lemmy.ml 0.19.20: `type_=All`, `Posts`, `Comments`,
/// `Communities` and `Users` each answer 200 and only populate their own array
/// (`type_=Posts` returned 2 posts and zero of everything else), and the
/// response echoes the value back in its own `type_` field. `Url` is the
/// server's sixth value — it resolves an exact permalink rather than searching
/// — and is offered here too, since Lemmy's own web search has it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SearchType {
    #[default]
    All,
    Posts,
    Comments,
    Communities,
    Users,
    Url,
}

impl SearchType {
    /// Every search type, in the order the UI offers them.
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Posts,
        Self::Comments,
        Self::Communities,
        Self::Users,
        Self::Url,
    ];

    /// The exact string `search` expects in `type_`.
    pub fn as_query(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Posts => "Posts",
            Self::Comments => "Comments",
            Self::Communities => "Communities",
            Self::Users => "Users",
            Self::Url => "Url",
        }
    }

    /// Human-facing label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Posts => "Posts",
            Self::Comments => "Comments",
            Self::Communities => "Communities",
            Self::Users => "Users",
            Self::Url => "URL",
        }
    }

    /// A hint for the search box, since a URL search wants a different kind of
    /// input from the rest.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Url => "Paste a post or comment link…",
            _ => "Search posts, comments, communities…",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_match_lemmy_v3_spelling() {
        assert_eq!(Sort::TopDay.as_query(), "TopDay");
        assert_eq!(Sort::MostComments.as_query(), "MostComments");
        assert_eq!(ListingType::All.as_query(), "All");
        // `type_` is spelled with the underscore; the query key is asserted in
        // tests/api_shapes.rs where the request is actually built.
    }

    /// Pins the live-verified comment vocabulary. If someone adds a value here
    /// that `comment/list` rejects, this fails instead of the app 400-ing.
    #[test]
    fn comment_sorts_match_the_endpoints_accepted_vocabulary() {
        let accepted = ["Hot", "Top", "New", "Old", "Controversial"];
        for sort in CommentSort::ALL {
            assert!(
                accepted.contains(&sort.as_query()),
                "{:?} ({}) is not accepted by comment/list",
                sort,
                sort.as_query()
            );
        }
        assert_eq!(
            CommentSort::ALL.len(),
            accepted.len(),
            "the accepted list should be covered exactly"
        );
        assert_eq!(CommentSort::default().as_query(), "Top");
    }

    /// The two endpoints really do disagree, which is why they get two types.
    #[test]
    fn post_and_comment_sorts_are_distinct_vocabularies() {
        assert!(
            Sort::ALL.iter().any(|s| s.as_query() == "TopAll"),
            "posts use TopAll"
        );
        assert!(
            !Sort::ALL.iter().any(|s| s.as_query() == "Top"),
            "post/list rejects a bare Top"
        );
        assert!(
            !CommentSort::ALL.iter().any(|s| s.as_query() == "TopAll"),
            "comment/list rejects TopAll"
        );
    }

    /// Pins the live-verified search vocabulary, and that each value is
    /// spelled the way `search` expects — `type_=Posts`, not `post`.
    #[test]
    fn search_types_match_the_endpoints_accepted_vocabulary() {
        let accepted = ["All", "Posts", "Comments", "Communities", "Users", "Url"];
        for search_type in SearchType::ALL {
            assert!(
                accepted.contains(&search_type.as_query()),
                "{:?} ({}) is not accepted by search",
                search_type,
                search_type.as_query()
            );
        }
        assert_eq!(
            SearchType::ALL.len(),
            accepted.len(),
            "the vocabulary differs"
        );
        assert_eq!(SearchType::default().as_query(), "All");

        let mut labels: Vec<&str> = SearchType::ALL.iter().map(|s| s.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), SearchType::ALL.len(), "duplicate labels");
    }

    #[test]
    fn every_sort_has_a_distinct_label_and_query() {
        let mut labels: Vec<&str> = Sort::ALL.iter().map(|s| s.label()).collect();
        labels.sort_unstable();
        let mut queries: Vec<&str> = Sort::ALL.iter().map(|s| s.as_query()).collect();
        queries.sort_unstable();
        let unique_labels = {
            let mut l = labels.clone();
            l.dedup();
            l.len()
        };
        let unique_queries = {
            let mut q = queries.clone();
            q.dedup();
            q.len()
        };
        assert_eq!(unique_labels, Sort::ALL.len(), "duplicate sort labels");
        assert_eq!(unique_queries, Sort::ALL.len(), "duplicate sort queries");
    }
}
