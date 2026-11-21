//! An open post: its body, its link, and its comment thread.
//!
//! Voting the post and its comments, and writing a comment, all report back
//! through [`Actions`] and are applied after the frame — the same rule the rest
//! of the interface follows, so a draw pass never mutates state mid-draw.

use std::collections::HashSet;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::api::CommentSort;
use crate::app::{Limonada, Status};
use crate::state::{CommunityScope, Phase};
use crate::tree::ThreadNode;
use crate::ui::widgets;
use crate::ui::widgets::VoteAction;

/// Clicks and choices collected during the render pass, applied afterwards so
/// that drawing never mutates application state.
#[derive(Default)]
struct Actions {
    back: bool,
    reload: bool,
    set_comment_sort: Option<CommentSort>,
    toggle_collapse: Vec<i64>,
    open_image: bool,
    /// A vote on the post itself.
    post_vote: Option<i16>,
    /// A vote on one comment: its id and the score to send.
    comment_vote: Option<(i64, i16)>,
    /// A vote was asked for with no session.
    sign_in_needed: bool,
    /// Scope the feed to the post's community.
    scope: Option<CommunityScope>,
    /// Open the editor for a new top-level comment (`None`) or a reply.
    start_reply: Option<Option<i64>>,
    cancel_reply: bool,
    submit_reply: bool,
}

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let now = app.now;
    let signed_in = app.signed_in();
    let mut actions = Actions::default();

    {
        // `detail` and `markdown` are disjoint fields, so both borrows coexist —
        // and `detail` is mutable because the reply editor edits its draft.
        let Some(detail) = app.detail.as_mut() else {
            widgets::empty(ui, "No post open", Some("Choose a post from the feed."));
            return;
        };
        let cache = &mut app.markdown;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if ui.button("← Back").clicked() {
                        actions.back = true;
                    }
                    if ui.button("⟳ Reload").clicked() {
                        actions.reload = true;
                    }
                    if signed_in {
                        if ui
                            .button("✎ Add a comment")
                            .on_hover_text("Write a new comment on this post")
                            .clicked()
                        {
                            actions.start_reply = Some(None);
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("Sign in to comment or vote")
                                .weak()
                                .small(),
                        );
                    }
                });

                ui.add_space(6.0);
                render_header(ui, detail, cache, now, signed_in, &mut actions);
                ui.add_space(10.0);

                render_thread_section(ui, detail, cache, now, signed_in, &mut actions);
                ui.add_space(12.0);
            });
    }

    if actions.back {
        app.close_post(&ctx);
        return;
    }
    if actions.reload
        && let Some(detail) = &mut app.detail
    {
        detail.reload(&ctx, &app.client);
    }
    if let Some(sort) = actions.set_comment_sort {
        app.settings.comment_sort = sort;
        if let Some(detail) = &mut app.detail {
            detail.comment_sort = sort;
            detail.reload(&ctx, &app.client);
        }
    }
    if !actions.toggle_collapse.is_empty()
        && let Some(detail) = &mut app.detail
    {
        for id in actions.toggle_collapse {
            if !detail.collapsed.remove(&id) {
                detail.collapsed.insert(id);
            }
        }
    }
    if actions.sign_in_needed {
        app.status = Some(Status::info("Sign in to vote."));
    }
    if let Some(scope) = actions.scope {
        app.set_scope(&ctx, Some(scope));
        return;
    }
    let post_id = app.detail.as_ref().map(|detail| detail.post_id);
    if let (Some(score), Some(post_id)) = (actions.post_vote, post_id) {
        app.cast_post_vote(&ctx, post_id, score);
    }
    if let Some((comment_id, score)) = actions.comment_vote {
        app.cast_comment_vote(&ctx, comment_id, score);
    }
    if let Some(parent) = actions.start_reply
        && let Some(detail) = &mut app.detail
    {
        detail.start_reply(parent);
    }
    if actions.cancel_reply
        && let Some(detail) = &mut app.detail
    {
        detail.cancel_reply();
    }
    if actions.submit_reply
        && let Some(detail) = &mut app.detail
    {
        detail.submit_reply(&ctx, &app.client);
    }
    if actions.open_image && !app.show_open_post_image() {
        app.status = Some(crate::app::Status::info(
            "This post has no picture to open.",
        ));
    }
}

fn render_header(
    ui: &mut egui::Ui,
    detail: &mut crate::state::Detail,
    cache: &mut CommonMarkCache,
    now: i64,
    signed_in: bool,
    actions: &mut Actions,
) {
    let Some(post) = detail.post.as_ref() else {
        match &detail.post_phase {
            Phase::Failed(message) => {
                let message = message.clone();
                if widgets::error_block(ui, "Could not load this post", &message, Some("Retry"), None)
                {
                    actions.reload = true;
                }
            }
            _ => widgets::loading(ui, "Loading post…"),
        }
        return;
    };

    ui.horizontal(|ui| {
        if ui
            .add(egui::Link::new(
                egui::RichText::new(format!("!{}", post.community.name)).strong(),
            ))
            .on_hover_text("Show posts from this community")
            .clicked()
        {
            actions.scope = Some(CommunityScope {
                id: post.community.id,
                name: post.community.name.clone(),
            });
        }
        ui.label(egui::RichText::new("·").weak());
        ui.label(post.creator.label());
        ui.label(egui::RichText::new("·").weak());
        ui.label(widgets::age(post.post.published.as_deref(), now));
        widgets::nsfw_badge(ui, post.post.nsfw);
    });

    ui.add_space(4.0);
    ui.label(egui::RichText::new(&post.post.name).heading());
    ui.add_space(2.0);

    ui.horizontal(|ui| {
        match widgets::vote_buttons(ui, post.counts.score, post.my_vote, signed_in) {
            VoteAction::Nothing => {}
            VoteAction::Set(score) => actions.post_vote = Some(score),
            VoteAction::SignInRequired => actions.sign_in_needed = true,
        }
        // The server's own count, which can differ from what the thread shows.
        widgets::comment_count(ui, post.counts.comments);
        if let Some(url) = post.post.url.as_deref()
            && let Some(host) = widgets::host_of(url)
        {
            ui.hyperlink_to(host, url);
        }
    });

    // The picture on the page is a thumbnail, so clicking it is how the user
    // asks for the original: it opens over the post rather than replacing it, so
    // the thread stays where it was.
    if let Some(thumb) = crate::state::inline_image_url(post) {
        ui.add_space(6.0);
        let mut image = egui::Image::new(thumb)
            .max_width(ui.available_width().min(560.0))
            .max_height(420.0)
            .sense(egui::Sense::click());
        if let Some(alt) = post
            .post
            .alt_text
            .as_deref()
            .map(str::trim)
            .filter(|alt| !alt.is_empty())
        {
            image = image.alt_text(alt);
        }
        let response = ui
            .add(image)
            .on_hover_cursor(egui::CursorIcon::ZoomIn)
            .on_hover_text("Click to view at full resolution");
        if response.clicked() {
            actions.open_image = true;
        }
    }

    if let Some(body) = post
        .post
        .body
        .as_deref()
        .filter(|body| !body.trim().is_empty())
    {
        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            CommonMarkViewer::new().show(ui, cache, body);
        });
    }
}

fn render_thread_section(
    ui: &mut egui::Ui,
    detail: &mut crate::state::Detail,
    cache: &mut CommonMarkCache,
    now: i64,
    signed_in: bool,
    actions: &mut Actions,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Comments").strong());

        // `selectable_value` needs a mutable target, so the choice is made in a
        // local and reported through `actions` when it actually changes.
        let mut chosen = detail.comment_sort;
        egui::ComboBox::from_id_salt("limonada-comment-sort")
            .selected_text(chosen.label())
            .show_ui(ui, |ui| {
                for sort in CommentSort::ALL {
                    ui.selectable_value(&mut chosen, sort, sort.label());
                }
            });
        if chosen != detail.comment_sort {
            actions.set_comment_sort = Some(chosen);
        }

        ui.label(
            egui::RichText::new(format!(
                "· {} shown · instance reports {}",
                detail.comment_count(),
                detail
                    .post
                    .as_ref()
                    .map(|post| post.counts.comments)
                    .unwrap_or(0)
            ))
            .weak()
            .small(),
        );
    });

    ui.add_space(6.0);

    // The reply editor sits here rather than under the comment it answers, so
    // the thread it is written against stays fully readable while writing.
    if detail.reply_to.is_some() {
        render_reply_editor(ui, detail, actions);
        ui.add_space(6.0);
    }

    match &detail.thread_phase {
        Phase::Loading if detail.comments.is_empty() => {
            widgets::loading(ui, "Loading comments…");
        }
        Phase::Failed(message) => {
            let message = message.clone();
            if widgets::error_block(ui, "Could not load comments", &message, Some("Retry"), None) {
                actions.reload = true;
            }
        }
        _ if detail.comments.is_empty() => {
            widgets::empty(
                ui,
                "No comments",
                Some(
                    "This instance returned an empty thread. A post can report comments that its \
                     instance cannot serve, so this is not necessarily an error.",
                ),
            );
        }
        _ => {
            render_nodes(
                ui,
                &detail.comments,
                &detail.collapsed,
                cache,
                now,
                signed_in,
                actions,
            );
        }
    }
}

/// The comment box, drawn at whichever target the editor is open for.
fn render_reply_editor(ui: &mut egui::Ui, detail: &mut crate::state::Detail, actions: &mut Actions) {
    let heading = match detail.reply_to {
        Some(Some(id)) => match detail.comment_author(id) {
            Some(author) => format!("Replying to {author}"),
            None => "Replying to a comment".to_string(),
        },
        _ => "New comment on this post".to_string(),
    };
    let busy = detail.reply_is_busy();

    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(heading).strong().small());

        ui.add(
            egui::TextEdit::multiline(&mut detail.reply_draft)
                .hint_text("Write a comment…")
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can_post = !busy && !detail.reply_draft.trim().is_empty();
            let post = ui.add_enabled(can_post, egui::Button::new("Post comment"));
            if post.clicked() {
                actions.submit_reply = true;
            }
            if ui.button("Cancel").clicked() {
                actions.cancel_reply = true;
            }
            if busy {
                ui.spinner();
                ui.label(egui::RichText::new("posting…").weak().small());
            }
        });

        if let Some(message) = detail.reply_phase.error() {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(message)
                    .small()
                    .color(egui::Color32::from_rgb(220, 80, 80)),
            );
        }
    });
}

fn render_nodes(
    ui: &mut egui::Ui,
    nodes: &[ThreadNode],
    collapsed: &HashSet<i64>,
    cache: &mut CommonMarkCache,
    now: i64,
    signed_in: bool,
    actions: &mut Actions,
) {
    for node in nodes {
        render_node(ui, node, collapsed, cache, now, signed_in, actions);
    }
}

fn render_node(
    ui: &mut egui::Ui,
    node: &ThreadNode,
    collapsed: &HashSet<i64>,
    cache: &mut CommonMarkCache,
    now: i64,
    signed_in: bool,
    actions: &mut Actions,
) {
    let id = node.comment.comment.id;
    let is_collapsed = collapsed.contains(&id);

    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            let arrow = if is_collapsed { "▸" } else { "▾" };
            if ui
                .small_button(arrow)
                .on_hover_text(if is_collapsed {
                    "Expand replies"
                } else {
                    "Collapse replies"
                })
                .clicked()
            {
                actions.toggle_collapse.push(id);
            }

            ui.label(egui::RichText::new(node.comment.creator.label()).strong());
            ui.label(
                egui::RichText::new(widgets::age(
                    node.comment.comment.published.as_deref(),
                    now,
                ))
                .weak()
                .small(),
            );
            match widgets::vote_buttons(
                ui,
                node.comment.counts.score,
                node.comment.my_vote,
                signed_in,
            ) {
                VoteAction::Nothing => {}
                VoteAction::Set(score) => actions.comment_vote = Some((id, score)),
                VoteAction::SignInRequired => actions.sign_in_needed = true,
            }
            if node.comment.creator_is_moderator {
                ui.label(egui::RichText::new("mod").small().weak());
            }
            if node.comment.creator_is_admin {
                ui.label(egui::RichText::new("admin").small().weak());
            }
            if signed_in && ui.small_button("↩ Reply").clicked() {
                actions.start_reply = Some(Some(id));
            }
        });

        if is_collapsed {
            let hidden = node.descendant_count();
            if hidden > 0 {
                ui.label(
                    egui::RichText::new(format!("{hidden} replies hidden"))
                        .weak()
                        .small(),
                );
            }
        } else {
            if node.comment.comment.content.trim().is_empty() {
                ui.label(egui::RichText::new("[no content]").weak());
            } else {
                CommonMarkViewer::new().show(ui, cache, &node.comment.comment.content);
            }

            if !node.children.is_empty() {
                ui.indent(("limonada-thread", id), |ui| {
                    render_nodes(ui, &node.children, collapsed, cache, now, signed_in, actions);
                });
            }
        }
    });
}
