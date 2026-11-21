//! Search: a query, a type filter, and the four kinds of result.
//!
//! Results are drawn from a snapshot of the query's outcome and every click is
//! recorded for [`Limonada`] to apply after the frame, like every other region.
//!
//! Only one array is populated per search unless the type is `All` — a live
//! lemmy.ml 0.19.20 answers `type_=Posts` with posts and three empty arrays — so
//! each section is drawn only when it has something, and an empty section is not
//! reported as "no results".

use eframe::egui;

use crate::api::types::PostView;
use crate::api::{SearchType, Sort};
use crate::app::Limonada;
use crate::state::CommunityScope;
use crate::ui::widgets;

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let now = app.now;
    let mut submit = false;
    let mut retry = false;
    let mut close = false;
    let mut open_post: Option<(i64, Option<PostView>)> = None;
    let mut scope_community: Option<CommunityScope> = None;
    // `Some(true)` narrows the search to the feed's community, `Some(false)`
    // searches the whole instance. Applied after the frame, like every other
    // action here.
    let mut narrow: Option<bool> = None;
    let narrowed = app
        .search
        .community_id
        .and_then(|_| app.feed.scope.as_ref().map(|scope| scope.name.clone()));

    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Search").strong());
        if ui.button("← Feed").clicked() {
            close = true;
        }
        if app.search.is_busy() {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.search.attempted && !app.search.is_busy() {
                let summary = match &narrowed {
                    Some(name) => format!("{} results in !{name}", app.search.total()),
                    None => format!("{} results", app.search.total()),
                };
                ui.label(egui::RichText::new(summary).weak().small());
            }
        });
    });

    ui.add_space(4.0);

    // The query row. Enter in the box submits, as it would in a browser.
    let mut submitted_by_enter = false;
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.search.query)
                .hint_text(app.search.type_.hint())
                .desired_width(ui.available_width().min(460.0)),
        );
        submitted_by_enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

        egui::ComboBox::from_id_salt("limonada-search-type")
            .selected_text(app.search.type_.label())
            .show_ui(ui, |ui| {
                for search_type in SearchType::ALL {
                    ui.selectable_value(&mut app.search.type_, search_type, search_type.label());
                }
            });

        if ui.button("Search").clicked() {
            submit = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Sorted by").weak().small());
        egui::ComboBox::from_id_salt("limonada-search-sort")
            .selected_text(app.search.sort.label())
            .show_ui(ui, |ui| {
                for sort in Sort::ALL {
                    ui.selectable_value(&mut app.search.sort, sort, sort.label());
                }
            });

        // Narrowing is opt-in and visible. It used to be copied from the feed's
        // scope when this screen opened, which made an `All` search return
        // nothing at all after clicking any community — the instance answers
        // `type_=All` with empty arrays when `community_id` is not the one
        // searched for.
        match app.feed.scope.as_ref().map(|scope| (scope.id, scope.name.clone())) {
            Some((id, name)) => {
                let mut only = app.search.community_id == Some(id);
                if ui
                    .checkbox(&mut only, format!("Only in !{name}"))
                    .on_hover_text("Limit this search to the community the feed is showing")
                    .changed()
                {
                    narrow = Some(only);
                }
            }
            None => {
                ui.label(
                    egui::RichText::new("· all instances this one knows")
                        .weak()
                        .small(),
                );
            }
        }
    });

    ui.add_space(6.0);
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);

            if app.search.phase.is_loading() && app.search.total() == 0 {
                widgets::loading(ui, "Searching…");
                return;
            }

            if let Some(message) = app.search.phase.error() {
                retry = widgets::error_block(ui, "Search failed", message, Some("Retry"), None);
                return;
            }

            if !app.search.attempted {
                widgets::empty(
                    ui,
                    "Nothing searched yet",
                    Some("Type a query above, choose what to look for, and press Search."),
                );
                return;
            }

            if app.search.total() == 0 {
                if narrowed.is_some() {
                    widgets::empty(
                        ui,
                        "No matches in this community",
                        Some("The search was limited to the community the feed is showing."),
                    );
                    ui.add_space(6.0);
                    ui.vertical_centered(|ui| {
                        if ui.button("Search the whole instance").clicked() {
                            narrow = Some(false);
                        }
                    });
                } else {
                    widgets::empty(
                        ui,
                        "No results",
                        Some("The instance returned nothing for this query and type."),
                    );
                }
                return;
            }

            if !app.search.posts.is_empty() {
                section(ui, &format!("Posts ({})", app.search.posts.len()));
                for post in &app.search.posts {
                    if post_result(ui, post, now).clicked() {
                        open_post = Some((post.post.id, Some(post.clone())));
                    }
                }
                ui.add_space(8.0);
            }

            if !app.search.communities.is_empty() {
                section(
                    ui,
                    &format!("Communities ({})", app.search.communities.len()),
                );
                for view in &app.search.communities {
                    let name = view.community.name.clone();
                    if community_result(ui, view).clicked() {
                        scope_community = Some(CommunityScope {
                            id: view.community.id,
                            name,
                        });
                    }
                }
                ui.add_space(8.0);
            }

            if !app.search.comments.is_empty() {
                section(ui, &format!("Comments ({})", app.search.comments.len()));
                for view in &app.search.comments {
                    let excerpt = comment_excerpt(&view.comment.content);
                    let author = view.creator.label().to_string();
                    let source = view
                        .community
                        .as_ref()
                        .map(|community| format!("!{}", community.name))
                        .unwrap_or_else(|| format!("post {}", view.comment.post_id));
                    if comment_result(ui, &excerpt, &author, &source).clicked() {
                        open_post = Some((view.comment.post_id, None));
                    }
                }
                ui.add_space(8.0);
            }

            if !app.search.users.is_empty() {
                section(ui, &format!("Users ({})", app.search.users.len()));
                for view in &app.search.users {
                    user_result(ui, view);
                }
            }

            ui.add_space(8.0);
        });

    if close {
        app.close_search();
        return;
    }
    if let Some(only) = narrow {
        app.search.community_id = if only {
            app.feed.scope.as_ref().map(|scope| scope.id)
        } else {
            None
        };
        // Resubmits even mid-flight: the answer to the old narrowing must not
        // land after the toggle has changed it.
        app.search.resubmit(&ctx, &app.client);
    }
    if submit || submitted_by_enter || retry {
        app.search.submit(&ctx, &app.client);
    }
    if let Some(scope) = scope_community {
        let name = scope.name.clone();
        app.set_scope(&ctx, Some(scope));
        app.status = Some(crate::app::Status::info(format!(
            "Scoped the feed to !{name}."
        )));
    }
    if let Some((post_id, seed)) = open_post {
        app.open_post(&ctx, post_id, seed);
    }
}

fn section(ui: &mut egui::Ui, heading: &str) {
    ui.label(egui::RichText::new(heading).strong());
    ui.add_space(4.0);
}

fn post_result(ui: &mut egui::Ui, view: &PostView, now: i64) -> egui::Response {
    let id = egui::Id::new(("limonada-search-post", view.post.id));
    let inner = egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("!{}", view.community.name)).strong());
            ui.label(egui::RichText::new("·").weak());
            ui.label(view.creator.label());
            ui.label(egui::RichText::new("·").weak());
            ui.label(
                egui::RichText::new(widgets::age(view.post.published.as_deref(), now))
                    .weak()
                    .small(),
            );
            widgets::nsfw_badge(ui, view.post.nsfw);
        });
        ui.add_space(2.0);
        ui.label(egui::RichText::new(&view.post.name).heading());
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            // Shown without vote controls: a search result opens the post, where
            // voting belongs, so this stays a score chip here.
            widgets::score(ui, view.counts.score);
            widgets::comment_count(ui, view.counts.comments);
        });
    });
    clickable_card(ui, inner, id)
}

fn community_result(ui: &mut egui::Ui, view: &crate::api::types::CommunityView) -> egui::Response {
    let id = egui::Id::new(("limonada-search-community", view.community.id));
    let inner = egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            // Qualified with its instance: a community search spans the whole
            // federation, where `!technology` exists on a dozen instances.
            ui.label(egui::RichText::new(widgets::community_handle(&view.community)).strong());
            if view.community.nsfw {
                widgets::nsfw_badge(ui, true);
            }
            ui.label(
                egui::RichText::new(format!(
                    "{} subscribers",
                    widgets::group_digits(view.counts.subscribers)
                ))
                .weak()
                .small(),
            );
        });
        if let Some(title) = view
            .community
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
        {
            ui.label(egui::RichText::new(title).weak().small());
        }
    });
    clickable_card(ui, inner, id)
}

fn comment_result(ui: &mut egui::Ui, excerpt: &str, author: &str, source: &str) -> egui::Response {
    // Keyed by content, because a comment result is only identified by the
    // comment it is; `excerpt` plus `author` is stable for one result.
    let id = egui::Id::new(("limonada-search-comment", author, excerpt));
    let inner = egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(author).strong());
            ui.label(egui::RichText::new("·").weak());
            ui.label(egui::RichText::new(source).weak().small());
        });
        ui.label(egui::RichText::new(excerpt).weak());
    });
    let response = ui.interact(inner.response.rect, id, egui::Sense::click());
    if response.hovered() {
        ui.output_mut(|out| out.cursor_icon = egui::CursorIcon::PointingHand);
    }
    response
}

fn user_result(ui: &mut egui::Ui, view: &crate::api::types::PersonView) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(view.person.label()).strong());
            ui.label(
                egui::RichText::new(format!("@{}", view.person.name))
                    .weak()
                    .small(),
            );
            if view.is_admin {
                ui.label(egui::RichText::new("admin").small().weak());
            }
            if view.person.bot_account {
                ui.label(egui::RichText::new("bot").small().weak());
            }
        });
        ui.label(
            egui::RichText::new(format!(
                "{} posts · {} comments",
                widgets::group_digits(view.counts.post_count),
                widgets::group_digits(view.counts.comment_count)
            ))
            .weak()
            .small(),
        );
    });
}

/// Make a card clickable as a whole, keyed by `id`.
fn clickable_card(
    ui: &mut egui::Ui,
    inner: egui::InnerResponse<()>,
    id: egui::Id,
) -> egui::Response {
    let response = ui.interact(inner.response.rect, id, egui::Sense::click());
    if response.hovered() {
        ui.output_mut(|out| out.cursor_icon = egui::CursorIcon::PointingHand);
    }
    response
}

/// A one-line, whitespace-collapsed excerpt of a comment body.
fn comment_excerpt(content: &str) -> String {
    widgets::truncate(
        &content.split_whitespace().collect::<Vec<_>>().join(" "),
        140,
    )
}

#[cfg(test)]
mod tests {
    use super::comment_excerpt;

    #[test]
    fn an_excerpt_collapses_whitespace_and_truncates() {
        assert_eq!(comment_excerpt("a\n\nb   c"), "a b c");
        assert_eq!(comment_excerpt("   "), "");
        let long = "x".repeat(300);
        let excerpt = comment_excerpt(&long);
        assert!(
            excerpt.chars().count() <= 140,
            "got {}",
            excerpt.chars().count()
        );
        assert!(excerpt.ends_with('…'));
    }
}
