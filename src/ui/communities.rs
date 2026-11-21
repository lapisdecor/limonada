//! The community sidebar.
//!
//! Clicking a community scopes the feed to it, which works without a session
//! because `post/list` supports a `community_id` filter.
//!
//! The page it lists is asked for with `sort=TopAll`, so it is the instance's
//! active communities rather than its newest ones — without that sort a live
//! lemmy.ml 0.19.20 returned fifty brand-new communities, most with no posts at
//! all, and clicking one showed an empty feed. Because one page cannot hold
//! every community, the filter also asks the instance: matches from that lookup
//! are drawn under "Elsewhere on this instance".

use eframe::egui;

use crate::app::Limonada;
use crate::state::{CommunityScope, Phase};
use crate::ui::widgets;

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let mut selected: Option<CommunityScope> = None;
    let mut clear_scope = false;
    let mut reload = false;
    let mut retry_find = false;

    egui::Panel::left("limonada-communities")
        .resizable(true)
        .default_size(250.0)
        .min_size(180.0)
        .max_size(420.0)
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Communities").strong());
                if app.communities.is_busy() {
                    ui.spinner();
                }
                if ui
                    .small_button("⟳")
                    .on_hover_text("Reload the community list")
                    .clicked()
                {
                    reload = true;
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                // Leave room for the spinner so typing never shifts the box.
                let width = (ui.available_width() - 26.0).max(80.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.communities.filter)
                        .hint_text("Search communities…")
                        .desired_width(width),
                );
                // The instance-wide lookup runs behind the filter, so say when
                // it is still working rather than leaving the list looking
                // complete.
                if app.communities.is_finding() {
                    ui.spinner();
                }
            });

            if app.feed.scope.is_some() {
                ui.add_space(4.0);
                let label = format!("✕ Showing only !{}", app.feed.scope_label());
                if ui.button(widgets::truncate(&label, 34)).clicked() {
                    clear_scope = true;
                }
            }

            ui.add_space(6.0);
            ui.separator();

            match &app.communities.phase {
                Phase::Loading if app.communities.items.is_empty() => {
                    widgets::loading(ui, "Loading communities…");
                }
                Phase::Failed(message) => {
                    let message = message.clone();
                    let retry = widgets::error_block(
                        ui,
                        "Could not load communities",
                        &message,
                        Some("Retry"),
                        None,
                    );
                    if retry {
                        reload = true;
                    }
                }
                _ => {
                    // Copy out what the rows need so the list below can be
                    // iterated while `app` stays borrowed only for reads.
                    let active_scope = app.feed.scope.as_ref().map(|scope| scope.id);
                    let rows: Vec<(i64, String, String, i64, bool, String)> = app
                        .communities
                        .filtered()
                        .into_iter()
                        .map(|view| {
                            (
                                view.community.id,
                                view.community.name.clone(),
                                view.community
                                    .title
                                    .clone()
                                    .unwrap_or_else(|| view.community.name.clone()),
                                view.counts.subscribers,
                                active_scope == Some(view.community.id),
                                widgets::community_handle(&view.community),
                            )
                        })
                        .collect();

                    // Matches the page does not already show, so the two lists
                    // never repeat a community.
                    let shown: std::collections::HashSet<i64> =
                        rows.iter().map(|row| row.0).collect();
                    let matches: Vec<(i64, String, String, i64, String)> = app
                        .communities
                        .found
                        .iter()
                        .filter(|view| !shown.contains(&view.community.id))
                        .map(|view| {
                            (
                                view.community.id,
                                view.community.name.clone(),
                                view.community
                                    .title
                                    .clone()
                                    .unwrap_or_else(|| view.community.name.clone()),
                                view.counts.subscribers,
                                widgets::community_handle(&view.community),
                            )
                        })
                        .collect();

                    let lookup = app.communities.lookup_text();
                    let filter_text = app.communities.filter.trim().to_string();
                    let find_phase = app.communities.find_phase.clone();

                    if rows.is_empty() && matches.is_empty() {
                        // Nothing to list: report why, distinguishing "still
                        // asking" from "the instance has no such community"
                        // and from "nothing is loaded at all".
                        if let Some(message) = find_phase.error() {
                            retry_find =
                                widgets::error_block(ui, "Could not search communities", message, Some("Retry"), None);
                        } else if find_phase.is_loading() {
                            widgets::loading(ui, "Searching this instance…");
                        } else if !lookup.is_empty() {
                            let detail = format!("Nothing on this instance matches “{lookup}”.");
                            widgets::empty(ui, "No match", Some(detail.as_str()));
                        } else if !filter_text.is_empty() {
                            let detail = format!(
                                "Nothing in the loaded page matches “{filter_text}”, and a \
                                 lookup needs at least two characters."
                            );
                            widgets::empty(ui, "No match", Some(detail.as_str()));
                        } else {
                            widgets::empty(ui, "No communities", None);
                        }
                    } else {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (id, name, title, subscribers, is_active, handle) in &rows {
                                    let text = widgets::truncate(handle, 30);
                                    let response = ui
                                        .selectable_label(*is_active, text)
                                        .on_hover_text(format!(
                                            "{title}\n{} subscribers",
                                            widgets::group_digits(*subscribers)
                                        ));
                                    if response.clicked() {
                                        selected = Some(CommunityScope {
                                            id: *id,
                                            name: name.clone(),
                                        });
                                    }
                                }

                                if !matches.is_empty() {
                                    ui.add_space(8.0);
                                    ui.separator();
                                    ui.label(
                                        egui::RichText::new("Elsewhere on this instance")
                                            .weak()
                                            .small(),
                                    );
                                    ui.add_space(2.0);
                                    for (id, name, title, subscribers, handle) in &matches {
                                        let text = widgets::truncate(handle, 30);
                                        let response = ui
                                            .selectable_label(false, text)
                                            .on_hover_text(format!(
                                                "{title}\n{} subscribers",
                                                widgets::group_digits(*subscribers)
                                            ));
                                        if response.clicked() {
                                            selected = Some(CommunityScope {
                                                id: *id,
                                                name: name.clone(),
                                            });
                                        }
                                    }
                                }
                            });
                    }
                }
            }
        });

    if reload {
        app.communities
            .refresh(&ctx, &app.client, crate::api::ListingType::All);
    }
    if retry_find {
        let query = app.communities.lookup_text();
        app.communities.find(&ctx, &app.client, &query);
    }
    if clear_scope {
        app.set_scope(&ctx, None);
    } else if let Some(scope) = selected {
        app.set_scope(&ctx, Some(scope));
    }
}
