//! The top toolbar: identity, feed controls and view toggles.

use eframe::egui;

use crate::api::{ListingType, Sort};
use crate::app::{Limonada, View};
use crate::state::Theme;
use crate::ui::widgets;

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let mut refresh = false;
    let mut sort_or_listing_changed = false;

    // egui 0.36 replaced TopBottomPanel/SidePanel with a unified `Panel`.
    egui::Panel::top("limonada-topbar").show(ui, |ui| {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🍋 Limonada").heading().strong());
            ui.separator();

            let host = widgets::host_of(&app.settings.normalised_instance())
                .unwrap_or_else(|| app.source_label());
            ui.label(egui::RichText::new(host).weak());

            if app.client.transport().is_offline() {
                ui.label(
                    egui::RichText::new("DEMO")
                        .small()
                        .strong()
                        .color(egui::Color32::from_rgb(230, 170, 60)),
                )
                .on_hover_text("Serving captured responses from fixtures/, not the network");
            }

            if app.view == View::Feed {
                ui.separator();

                let before_sort = app.feed.sort;
                egui::ComboBox::from_id_salt("limonada-feed-sort")
                    .selected_text(app.feed.sort.label())
                    .show_ui(ui, |ui| {
                        for sort in Sort::ALL {
                            ui.selectable_value(&mut app.feed.sort, sort, sort.label());
                        }
                    });
                if app.feed.sort != before_sort {
                    sort_or_listing_changed = true;
                }

                let before_listing = app.feed.listing;
                egui::ComboBox::from_id_salt("limonada-feed-listing")
                    .selected_text(app.feed.listing.label())
                    .show_ui(ui, |ui| {
                        for listing in ListingType::ALL {
                            ui.selectable_value(&mut app.feed.listing, listing, listing.label());
                        }
                    });
                if app.feed.listing != before_listing {
                    sort_or_listing_changed = true;
                }
            } else if app.view == View::Post {
                ui.separator();
                if ui.button("← Back to feed").clicked() {
                    app.close_post(&ctx);
                }
            } else {
                ui.separator();
                ui.label(egui::RichText::new("Searching").weak());
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    app.settings_open = true;
                }

                // Who is signed in, or the way to sign in. The label is built
                // before the click is handled, so mutating `app` below does not
                // fight the borrow that produced it.
                let account = match app.signed_in_as() {
                    Some(username) => format!("👤 {username}"),
                    None => "👤 Sign in".to_string(),
                };
                if ui
                    .button(account)
                    .on_hover_text(match app.signed_in_as() {
                        Some(_) => "Account",
                        None => "Sign in to vote and comment",
                    })
                    .clicked()
                {
                    app.auth_open = !app.auth_open;
                }

                if app.view != View::Search && ui.button("🔍 Search").clicked() {
                    app.open_search();
                }

                let theme_label = match app.settings.theme {
                    Theme::System => "◐",
                    Theme::Light => "☀",
                    Theme::Dark => "☾",
                };
                if ui
                    .button(theme_label)
                    .on_hover_text(format!("Theme: {}", app.settings.theme.label()))
                    .clicked()
                {
                    let current = Theme::ALL
                        .iter()
                        .position(|t| *t == app.settings.theme)
                        .unwrap_or(0);
                    app.settings.theme = Theme::ALL[(current + 1) % Theme::ALL.len()];
                    app.settings.theme.apply(&ctx);
                }

                let sidebar_label = if app.sidebar_open {
                    "◀ Communities"
                } else {
                    "▶ Communities"
                };
                if ui.button(sidebar_label).clicked() {
                    app.sidebar_open = !app.sidebar_open;
                }

                if ui.button("⟳ Refresh").clicked() {
                    refresh = true;
                }
            });
        });
        ui.add_space(5.0);
    });

    if sort_or_listing_changed {
        // Keep the persisted preferences in step with the UI.
        app.settings.sort = app.feed.sort;
        app.settings.listing = app.feed.listing;
        app.feed.refresh(&ctx, &app.client);
    }
    if refresh {
        app.refresh_current(&ctx);
    }
}
