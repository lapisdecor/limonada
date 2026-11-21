//! Comment threads.
//!
//! Lemmy returns a comment thread as a **flat list**, where each comment
//! carries a materialised ancestor path:
//!
//! ```text
//! "0.28327851.28327929.28334001"
//!  │  │        │        └── this comment's id
//!  │  │        └─────────── its parent
//!  │  └──────────────────── its grandparent
//!  └─────────────────────── the sentinel root (0 = "no parent")
//! ```
//!
//! So the whole tree is one grouping pass over a single response — no
//! per-thread requests. This module is pure and has no network or UI
//! dependencies, which is why it can be tested exhaustively.

use std::collections::HashMap;

use crate::api::types::CommentView;

/// Hard ceiling on nesting.
///
/// Real threads are shallow (a sampled live thread was 4 levels deep), so this
/// is purely defensive: a malformed or hostile `path` chain must not be able to
/// drive unbounded work or recursion.
pub const MAX_DEPTH: usize = 64;

/// A comment and its replies, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadNode {
    pub comment: CommentView,
    /// Nesting level within this batch, starting at 0 for roots.
    ///
    /// Derived from the built tree rather than from `comment.path`, so a
    /// comment promoted to a root (because its parent was truncated) still
    /// renders at a sensible indent instead of floating at its path depth.
    pub depth: usize,
    pub children: Vec<ThreadNode>,
}

impl ThreadNode {
    /// Every descendant, excluding this node.
    pub fn descendant_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| 1 + child.descendant_count())
            .sum()
    }

    /// Find a comment's view somewhere in this subtree.
    pub fn find(&self, id: i64) -> Option<&CommentView> {
        if self.comment.comment.id == id {
            return Some(&self.comment);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    /// Replace the stored view for a comment somewhere in this subtree.
    ///
    /// Used after a vote or an edit, so the tree shows the server's own updated
    /// copy. Returns false when the comment is not in this subtree, which is not
    /// an error — the thread may have been reloaded since the request left.
    pub fn replace(&mut self, view: &CommentView) -> bool {
        if self.comment.comment.id == view.comment.id {
            self.comment = view.clone();
            return true;
        }
        self.children.iter_mut().any(|child| child.replace(view))
    }

    /// Depth-first walk in render order, calling `visit` for each node.
    ///
    /// Returning `false` from `visit` skips that node's subtree, which is how
    /// the UI implements collapsing without a second traversal.
    pub fn walk<F>(&self, visit: &mut F)
    where
        F: FnMut(usize, &CommentView) -> bool,
    {
        fn recurse<F>(node: &ThreadNode, visit: &mut F)
        where
            F: FnMut(usize, &CommentView) -> bool,
        {
            if visit(node.depth, &node.comment) {
                for child in &node.children {
                    recurse(child, visit);
                }
            }
        }
        recurse(self, visit);
    }
}

/// Build the thread forest for a batch of comments.
///
/// Guarantees:
///
/// * Sibling order matches the order the server returned, so a `Top`-sorted
///   thread keeps its ranking.
/// * A comment whose parent is **not in this batch** becomes a root instead of
///   being dropped. The server truncates deep threads, so this is the normal
///   case for large threads, not an edge case.
/// * A comment whose `path` is empty, malformed, self-referential, or part of a
///   parent cycle also becomes a root. Nothing is ever lost or looped.
pub fn build_threads(comments: &[CommentView]) -> Vec<ThreadNode> {
    let count = comments.len();
    if count == 0 {
        return Vec::new();
    }

    let index: HashMap<i64, usize> = comments
        .iter()
        .enumerate()
        .map(|(i, view)| (view.comment.id, i))
        .collect();

    // Parent edge for each comment, when that parent is present in the batch.
    let mut parent_of: Vec<Option<usize>> = vec![None; count];
    for (i, view) in comments.iter().enumerate() {
        let Some(parent_id) = parent_id(&view.comment.path, view.comment.id) else {
            continue; // root, or unusable path
        };
        if let Some(&parent_index) = index.get(&parent_id)
            && parent_index != i
        {
            parent_of[i] = Some(parent_index);
        }
    }

    // Cut any edge that would close a cycle, so construction below cannot
    // recurse forever even on contradictory input.
    //
    // Detection runs against a snapshot of the graph as read from the paths,
    // which makes the outcome independent of iteration order: every edge that
    // closes a cycle in the input is cut, no matter which comment is visited
    // first.
    let original = parent_of.clone();
    for (index, slot) in parent_of.iter_mut().enumerate() {
        let mut cursor = *slot;
        let mut steps = 0usize;
        while let Some(parent) = cursor {
            if parent == index || steps > MAX_DEPTH {
                *slot = None;
                break;
            }
            steps += 1;
            cursor = original[parent];
        }
    }

    // Absolute depth along the root chain, used to enforce MAX_DEPTH.
    let depth_of: Vec<usize> = parent_of
        .iter()
        .map(|first| {
            let mut depth = 0usize;
            let mut cursor = *first;
            while let Some(parent) = cursor {
                depth += 1;
                if depth > MAX_DEPTH {
                    break;
                }
                cursor = parent_of[parent];
            }
            depth
        })
        .collect();

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut roots: Vec<usize> = Vec::new();
    for (index, parent) in parent_of.iter().enumerate() {
        match *parent {
            // Past the ceiling: promote to a root rather than nest further.
            Some(parent) if depth_of[parent] < MAX_DEPTH => children[parent].push(index),
            _ => roots.push(index),
        }
    }

    roots
        .into_iter()
        .map(|i| build_node(i, 0, &children, comments))
        .collect()
}

fn build_node(
    index: usize,
    depth: usize,
    children: &[Vec<usize>],
    comments: &[CommentView],
) -> ThreadNode {
    ThreadNode {
        comment: comments[index].clone(),
        depth,
        children: children[index]
            .iter()
            .map(|&child| build_node(child, depth + 1, children, comments))
            .collect(),
    }
}

/// Extract the parent id from a materialised path.
///
/// Returns `None` when the comment is a root, when the path is unusable, or
/// when it claims the comment is its own parent.
fn parent_id(path: &str, own_id: i64) -> Option<i64> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let segments: Vec<&str> = trimmed.split('.').collect();
    // "0.<id>" is the minimum shape, so anything shorter cannot describe a
    // parent/child relationship.
    if segments.len() < 2 {
        return None;
    }

    // Every segment must be a non-negative integer; otherwise the path is not
    // something we can trust.
    let mut parsed: Vec<i64> = Vec::with_capacity(segments.len());
    for segment in &segments {
        match segment.parse::<i64>() {
            Ok(value) if value >= 0 => parsed.push(value),
            _ => return None,
        }
    }

    let parent = parsed[parsed.len() - 2];
    if parent == 0 {
        return None; // sentinel root
    }
    if parent == own_id {
        return None; // self-reference
    }
    Some(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{Comment, CommentAggregates, CommentView, Person};

    /// Build a `CommentView` with just the fields the tree logic reads.
    fn comment(id: i64, path: &str) -> CommentView {
        CommentView {
            comment: Comment {
                id,
                content: format!("comment {id}"),
                path: path.to_string(),
                post_id: 1,
                creator_id: 1,
                language_id: 0,
                ap_id: None,
                published: None,
                local: true,
                distinguished: false,
                removed: false,
                deleted: false,
            },
            creator: Person {
                id: 1,
                name: "tester".into(),
                display_name: None,
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
            },
            post: None,
            community: None,
            counts: CommentAggregates {
                comment_id: id,
                ..Default::default()
            },
            subscribed: None,
            saved: false,
            creator_is_moderator: false,
            creator_is_admin: false,
            banned_from_community: false,
            creator_banned_from_community: false,
            creator_blocked: false,
            my_vote: None,
        }
    }

    /// Collect (id, depth) pairs in render order.
    fn shape(comments: &[CommentView]) -> Vec<(i64, usize)> {
        let mut out = Vec::new();
        for root in build_threads(comments) {
            root.walk(&mut |depth, view| {
                out.push((view.comment.id, depth));
                true
            });
        }
        out
    }

    #[test]
    fn nests_children_under_parents() {
        let comments = vec![
            comment(1, "0.1"),
            comment(2, "0.1.2"),
            comment(3, "0.1.2.3"),
            comment(4, "0.4"),
        ];
        assert_eq!(shape(&comments), vec![(1, 0), (2, 1), (3, 2), (4, 0)]);

        let roots = build_threads(&comments);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].comment.comment.id, 1);
        assert_eq!(roots[0].children.len(), 1);
        assert_eq!(roots[0].children[0].children.len(), 1);
        assert_eq!(roots[0].descendant_count(), 2);
    }

    #[test]
    fn preserves_server_order_within_siblings() {
        // Deliberately not ascending: a Top-sorted thread arrives ranked, and
        // we must not re-sort it.
        let comments = vec![
            comment(10, "0.10"),
            comment(30, "0.10.30"),
            comment(20, "0.10.20"),
            comment(5, "0.5"),
        ];
        assert_eq!(
            shape(&comments),
            vec![(10, 0), (30, 1), (20, 1), (5, 0)],
            "sibling order must follow the server, not the id"
        );
    }

    #[test]
    fn promotes_a_comment_whose_parent_is_missing() {
        // The server truncated the thread: comment 3's parent (2) is absent.
        let comments = vec![comment(1, "0.1"), comment(3, "0.1.2.3")];
        assert_eq!(
            shape(&comments),
            vec![(1, 0), (3, 0)],
            "an orphaned comment must be kept as a root, never dropped"
        );
    }

    #[test]
    fn promotes_a_deep_orphan_to_the_top_level_not_its_path_depth() {
        let comments = vec![comment(9, "0.2.4.6.8.9")];
        assert_eq!(shape(&comments), vec![(9, 0)]);
    }

    #[test]
    fn treats_unusable_paths_as_roots() {
        let comments = vec![
            comment(1, ""),
            comment(2, "garbage"),
            comment(3, "0"),
            comment(4, "-1.4"),
            comment(5, "0.abc.5"),
        ];
        assert_eq!(
            shape(&comments),
            vec![(1, 0), (2, 0), (3, 0), (4, 0), (5, 0)]
        );
    }

    #[test]
    fn treats_a_self_referencing_path_as_a_root() {
        assert_eq!(shape(&[comment(7, "0.7.7")]), vec![(7, 0)]);
    }

    #[test]
    fn survives_a_two_node_parent_cycle() {
        // Each claims the other as parent, which is contradictory input.
        // The contract is only that building terminates and nothing is lost;
        // which of the two edges survives is an arbitrary tie-break, so this
        // asserts the invariant rather than a specific shape.
        let comments = vec![comment(1, "0.2.1"), comment(2, "0.1.2")];
        let shape = shape(&comments);

        assert_eq!(shape.len(), 2, "both comments survive a cycle");
        let ids: Vec<i64> = shape.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1) && ids.contains(&2));
        assert!(
            shape.iter().all(|(_, depth)| *depth <= 1),
            "cutting one edge of a two-node cycle leaves at most one level: {shape:?}"
        );
        assert!(
            shape.iter().any(|(_, depth)| *depth == 0),
            "at least one comment must remain a root: {shape:?}"
        );
    }

    #[test]
    fn survives_a_longer_cycle() {
        let comments = vec![
            comment(1, "0.3.1"),
            comment(2, "0.1.2"),
            comment(3, "0.2.3"),
        ];
        assert_eq!(shape(&comments).len(), 3);
    }

    #[test]
    fn caps_nesting_at_max_depth() {
        // A legitimate chain deeper than MAX_DEPTH: the tail is promoted to
        // roots instead of nesting without bound.
        let deep: Vec<CommentView> = (1..=(MAX_DEPTH as i64 + 10))
            .map(|id| {
                let mut path = String::from("0");
                for ancestor in 1..id {
                    path.push('.');
                    path.push_str(&ancestor.to_string());
                }
                path.push('.');
                path.push_str(&id.to_string());
                comment(id, &path)
            })
            .collect();

        let roots = build_threads(&deep);
        let mut max_depth = 0;
        for root in &roots {
            root.walk(&mut |depth, _| {
                max_depth = max_depth.max(depth);
                true
            });
        }
        assert!(
            max_depth <= MAX_DEPTH,
            "nesting must stay within MAX_DEPTH, got {max_depth}"
        );

        let total: usize = roots
            .iter()
            .map(|root| 1 + root.descendant_count())
            .sum();
        assert_eq!(total, deep.len(), "every comment must still be present");
    }

    #[test]
    fn an_empty_batch_yields_no_threads() {
        assert!(build_threads(&[]).is_empty());
    }

    #[test]
    fn walk_can_skip_a_subtree() {
        let comments = vec![
            comment(1, "0.1"),
            comment(2, "0.1.2"),
            comment(3, "0.1.2.3"),
        ];
        let roots = build_threads(&comments);
        let mut seen = Vec::new();
        roots[0].walk(&mut |depth, view| {
            seen.push((view.comment.id, depth));
            view.comment.id != 2 // skip 2 and everything under it
        });
        assert_eq!(seen, vec![(1, 0), (2, 1)]);
    }
}
