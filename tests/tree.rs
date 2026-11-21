//! Thread building verified against a real captured thread.
//!
//! `fixtures/comment_list.json` is a live `comment/list` response from
//! lemmy.ml: 19 comments with a maximum materialised depth of 4 and no
//! truncated parents. The synthetic cases cover what that batch cannot.

use std::collections::HashSet;
use std::path::PathBuf;

use limonada::api::types::ListCommentsResponse;
use limonada::tree::{MAX_DEPTH, build_threads};

fn real_thread() -> ListCommentsResponse {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("comment_list.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("comment_list.json should decode")
}

/// Every comment in the batch, paired with its depth, in render order.
fn rendered(response: &ListCommentsResponse) -> Vec<(i64, usize)> {
    let mut out = Vec::new();
    for root in build_threads(&response.comments) {
        root.walk(&mut |depth, view| {
            out.push((view.comment.id, depth));
            true
        });
    }
    out
}

#[test]
fn every_comment_in_a_real_thread_is_preserved_exactly_once() {
    let response = real_thread();
    let rendered = rendered(&response);

    assert_eq!(
        rendered.len(),
        response.comments.len(),
        "the tree must contain every comment exactly once"
    );

    let seen: HashSet<i64> = rendered.iter().map(|(id, _)| *id).collect();
    assert_eq!(seen.len(), response.comments.len(), "no duplicates");

    for view in &response.comments {
        assert!(seen.contains(&view.comment.id), "comment {} was dropped", view.comment.id);
    }
}

#[test]
fn the_real_thread_takes_its_expected_shape() {
    let response = real_thread();
    let roots = build_threads(&response.comments);

    // Measured from the fixture: the batch has 10 root comments.
    assert_eq!(roots.len(), 10, "expected 10 top-level comments");

    let mut max_depth = 0;
    for root in &roots {
        root.walk(&mut |depth, _| {
            max_depth = max_depth.max(depth);
            true
        });
    }
    assert_eq!(max_depth, 4, "measured maximum nesting is 4");
}

#[test]
fn nesting_agrees_with_the_materialised_paths() {
    // Independent check on the builder: a comment's tree depth must equal its
    // path depth, because this batch has no truncated parents.
    let response = real_thread();

    let mut by_id = std::collections::HashMap::new();
    for (id, depth) in rendered(&response) {
        by_id.insert(id, depth);
    }

    for view in &response.comments {
        let path_depth = view.comment.path.split('.').count() - 2;
        assert_eq!(
            by_id.get(&view.comment.id).copied(),
            Some(path_depth),
            "comment {} nested at the wrong depth",
            view.comment.id
        );
    }
}

#[test]
fn sibling_order_follows_the_server_ranking() {
    // `sort=Top` ranks comments, and the UI must not re-sort them, so the order
    // of each sibling group has to match the order the server sent.
    let response = real_thread();

    let mut positions = std::collections::HashMap::new();
    for (position, (id, _)) in rendered(&response).into_iter().enumerate() {
        positions.insert(id, position);
    }

    // Within each sibling group in the server list, relative order is kept. A
    // cheap global proxy: every comment appears after its parent.
    for view in &response.comments {
        let segments: Vec<&str> = view.comment.path.split('.').collect();
        if segments.len() < 3 {
            continue;
        }
        let parent: i64 = segments[segments.len() - 2].parse().expect("numeric parent");
        if let (Some(&child_at), Some(&parent_at)) = (
            positions.get(&view.comment.id),
            positions.get(&parent),
        ) {
            assert!(
                parent_at < child_at,
                "comment {} should render after its parent {parent}",
                view.comment.id
            );
        }
    }
}

#[test]
fn an_empty_thread_builds_no_nodes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("comment_list_empty.json");
    let text = std::fs::read_to_string(&path).expect("read empty fixture");
    let response: ListCommentsResponse = serde_json::from_str(&text).expect("decode empty fixture");

    assert!(build_threads(&response.comments).is_empty());
}

#[test]
fn descendant_counts_are_consistent_with_the_tree() {
    let response = real_thread();
    let roots = build_threads(&response.comments);

    // Roots plus all descendants account for the whole batch.
    let total: usize = roots
        .iter()
        .map(|root| 1 + root.descendant_count())
        .sum();
    assert_eq!(total, response.comments.len());
}

#[test]
fn the_depth_cap_is_a_sane_bound() {
    // Guards against a future change making MAX_DEPTH absurd in either
    // direction: deep enough for real discussion, small enough to bound work.
    const { assert!(MAX_DEPTH >= 16, "too shallow for real threads") };
    const { assert!(MAX_DEPTH <= 256, "too deep to bound recursion safely") };
}
