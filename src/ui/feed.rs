//! The post feed.
//!
//! Rows are rendered from an immutable borrow of the feed, and any click is
//! recorded and applied afterwards — so rendering never mutates state mid-draw.

use eframe::egui;

use crate::api::ListingType;
use crate::api::types::PostView;
use crate::app::Limonada;
use crate::state::CommunityScope;
use crate::ui::widgets;

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let mut clicked: Option<PostView> = None;
    let mut wants_more = false;
    let mut retry = false;
    let mut vote: Option<(i64, i16)> = None;
    let mut sign_in_needed = false;
    let mut clear_scope = false;
    let mut scope_to: Option<CommunityScope> = None;
    let signed_in = app.signed_in();

    {
        let feed = &app.feed;
        let now = app.now;

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(feed.scope_label()).strong());
            ui.label(
                egui::RichText::new(format!("· sorted by {}", feed.sort.label()))
                    .weak()
                    .small(),
            );
            if feed.is_busy() {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !feed.items.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("{} loaded", feed.items.len()))
                            .weak()
                            .small(),
                    );
                }
            });
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Loading with nothing to show yet.
                if feed.items.is_empty() && feed.phase.is_loading() {
                    widgets::loading(ui, "Loading posts…");
                    return;
                }

                // Failure with nothing to show.
                if let Some(message) = feed.phase.error()
                    && feed.items.is_empty()
                {
                    let hint = subscribed_hint(feed.listing, signed_in);
                    retry = widgets::error_block(
                        ui,
                        "Could not load posts",
                        message,
                        Some("Retry"),
                        hint,
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!("Source: {}", app.client.transport().description()))
                            .weak()
                            .small(),
                    );
                    return;
                }

                if feed.items.is_empty() {
                    // An empty community is a normal state, not a failure, and
                    // saying so is the difference between "this community has
                    // nothing in it" and "the app is broken".
                    match feed.scope.as_ref() {
                        Some(scope) => {
                            let heading = format!("!{} has no posts yet", scope.name);
                            widgets::empty(
                                ui,
                                &heading,
                                Some(
                                    "Nothing has been posted here, or this instance has not \
                                     fetched the community's posts yet.",
                                ),
                            );
                            ui.add_space(6.0);
                            ui.vertical_centered(|ui| {
                                if ui.button("Show all posts").clicked() {
                                    clear_scope = true;
                                }
                            });
                        }
                        None => widgets::empty(
                            ui,
                            "No posts",
                            Some("The instance returned no posts for this listing and sort."),
                        ),
                    }
                    return;
                }

                for view in &feed.items {
                    let (response, action, community) = post_row(ui, view, now, signed_in);
                    if response.clicked() {
                        clicked = Some(view.clone());
                    }
                    if community.is_some() {
                        scope_to = community;
                    }
                    match action {
                        widgets::VoteAction::Nothing => {}
                        widgets::VoteAction::Set(score) => vote = Some((view.post.id, score)),
                        widgets::VoteAction::SignInRequired => sign_in_needed = true,
                    }
                }

                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    if feed.can_load_more() {
                        if ui.button("Load more").clicked() {
                            wants_more = true;
                        }
                    } else if feed.next_page.is_none() && !feed.is_busy() {
                        ui.label(egui::RichText::new("End of feed").weak().small());
                    }
                });
                ui.add_space(8.0);
            });
    }

    if clear_scope {
        app.set_scope(&ctx, None);
    }
    if retry {
        app.feed.refresh(&ctx, &app.client);
    }
    if wants_more {
        // A failed "load more" keeps the posts already on screen; `Feed::poll`
        // returns that error as a transient status message instead of blanking
        // the list.
        app.feed.load_more(&ctx, &app.client);
    }
    if sign_in_needed {
        app.status = Some(crate::app::Status::info("Sign in to vote."));
    }
    if let Some((post_id, score)) = vote {
        app.cast_post_vote(&ctx, post_id, score);
    }
    if let Some(scope) = scope_to {
        app.set_scope(&ctx, Some(scope));
    } else if let Some(post) = clicked {
        app.open_post(&ctx, post.post.id, Some(post));
    }
}

/// A hint explaining the one listing scope that needs a session.
///
/// `Subscribed` works once signed in — `post/list` filters by the token's own
/// subscriptions — so the hint is only for the signed-out case.
fn subscribed_hint(listing: ListingType, signed_in: bool) -> Option<&'static str> {
    match listing {
        ListingType::Subscribed if !signed_in => Some(
            "A subscribed feed needs a signed-in session. Sign in from the \
             top bar, or try All or Local.",
        ),
        _ => None,
    }
}

/// A single post row: a clickable card, with the vote controls on their own
/// strip below it, and a community chip that scopes the feed instead of opening
/// the post.
///
/// The votes deliberately sit *outside* the card's rectangle. The card is one
/// big click target — that is what makes a page of posts pleasant — and egui
/// resolves overlapping clicks in favour of the widget registered last, which
/// is the whole-row target. A vote button drawn inside it would therefore never
/// receive a click; drawn below it, both work.
///
/// The community chip cannot move out of the card, so it uses the same rule from
/// the other side: its own click target is registered *after* the card's, which
/// makes the chip win inside its rectangle and the card win everywhere else.
fn post_row(
    ui: &mut egui::Ui,
    view: &PostView,
    now: i64,
    signed_in: bool,
) -> (egui::Response, widgets::VoteAction, Option<CommunityScope>) {
    let mut community_rect = None;
    let inner = egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            // A link rather than a styled label: it reads as clickable, and
            // egui exposes it to the accessibility tree under its own text,
            // which is what lets a test click it without hardcoded pixels.
            let chip = ui
                .add(egui::Link::new(
                    egui::RichText::new(format!("!{}", view.community.name)).strong(),
                ))
                .on_hover_text("Show posts from this community");
            community_rect = Some(chip.rect);
            ui.label(egui::RichText::new("·").weak());
            ui.label(view.creator.label());
            ui.label(egui::RichText::new("·").weak());
            ui.label(
                egui::RichText::new(widgets::age(view.post.published.as_deref(), now))
                    .weak()
                    .small(),
            );
            widgets::nsfw_badge(ui, view.post.nsfw);
            if view.post.locked {
                ui.label(egui::RichText::new("🔒").small());
            }
        });

        ui.add_space(3.0);

        ui.horizontal(|ui| {
            if let Some(thumb) = view
                .post
                .thumbnail_url
                .as_deref()
                .filter(|thumb| !thumb.trim().is_empty())
            {
                // Images are fetched by egui_extras' HTTP loader.
                ui.add(egui::Image::new(thumb).max_height(68.0));
            }

            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(&view.post.name).heading())
                        .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);

                if let Some(url) = view.post.url.as_deref() {
                    if let Some(host) = widgets::host_of(url) {
                        ui.add(
                            egui::Label::new(egui::RichText::new(host).weak().small())
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(url);
                    }
                } else {
                    ui.label(egui::RichText::new("text post").weak().small());
                }

                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    widgets::comment_count(ui, view.counts.comments);
                    if view.post.url.is_none() {
                        ui.label(
                            egui::RichText::new(
                                view.post
                                    .body
                                    .as_deref()
                                    .map(|body| widgets::truncate(&body.replace('\n', " "), 90))
                                    .unwrap_or_default(),
                            )
                            .weak()
                            .small(),
                        );
                    }
                });
            });
        });
    });

    // The vote strip, below the card so it keeps its own clicks.
    let action = ui
        .horizontal(|ui| {
            ui.add_space(4.0);
            let action = widgets::vote_buttons(ui, view.counts.score, view.my_vote, signed_in);
            if view.my_vote.is_some() {
                ui.label(egui::RichText::new("your vote").weak().small());
            }
            action
        })
        .inner;
    ui.add_space(4.0);

    // Make the entire card clickable, not just the title.
    let response = ui.interact(
        inner.response.rect,
        ui.make_persistent_id(("limonada-post-row", view.post.id)),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.output_mut(|out| out.cursor_icon = egui::CursorIcon::PointingHand);
    }

    // Registered after the card on purpose: egui gives a click to the widget
    // registered last, so the chip is clickable without giving up the card.
    let mut scope = None;
    if let Some(rect) = community_rect {
        let chip = ui.interact(
            rect,
            ui.make_persistent_id(("limonada-post-community", view.post.id)),
            egui::Sense::click(),
        );
        if chip.hovered() {
            ui.output_mut(|out| out.cursor_icon = egui::CursorIcon::PointingHand);
        }
        if chip.clicked() {
            scope = Some(CommunityScope {
                id: view.community.id,
                name: view.community.name.clone(),
            });
        }
    }
    (response, action, scope)
}
