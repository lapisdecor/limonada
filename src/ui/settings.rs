//! Settings: instance, demo mode, theme, defaults, and diagnostics.
//!
//! Rendered as a right-hand panel rather than a floating window, which keeps it
//! non-modal and lets the user watch the feed reload as they change things.

use eframe::egui;

use crate::api::{CommentSort, ListingType, Sort};
use crate::app::{Limonada, Status};
use crate::state::Theme;

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let mut close = false;
    let mut apply = false;

    egui::Panel::right("limonada-settings")
        .resizable(true)
        .default_size(340.0)
        .min_size(290.0)
        .max_size(560.0)
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Settings").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Close").clicked() {
                        close = true;
                    }
                });
            });
            ui.separator();
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // --- Instance -------------------------------------------
                    ui.label(egui::RichText::new("Instance").strong());
                    ui.add_space(2.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut app.settings.instance_url)
                            .hint_text("https://lemmy.ml")
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(4.0);
                    ui.checkbox(&mut app.settings.demo, "Demo mode")
                        .on_hover_text(
                            "Serve captured responses from fixtures/ instead of the network",
                        );
                    if app.settings.demo {
                        ui.label(
                            egui::RichText::new(format!(
                                "Reading {}",
                                crate::app::demo_fixture_dir().display()
                            ))
                            .weak()
                            .small(),
                        );
                    }
                    ui.add_space(4.0);
                    if ui.button("Apply and reload").clicked() {
                        apply = true;
                    }
                    ui.label(
                        egui::RichText::new(
                            "Changing the instance or demo mode reloads the feed, \
                             communities and instance info.",
                        )
                        .weak()
                        .small(),
                    );

                    ui.add_space(10.0);
                    ui.separator();

                    // --- Appearance -----------------------------------------
                    ui.label(egui::RichText::new("Appearance").strong());
                    ui.add_space(2.0);
                    for theme in Theme::ALL {
                        if ui
                            .radio_value(&mut app.settings.theme, theme, theme.label())
                            .clicked()
                        {
                            theme.apply(&ctx);
                        }
                    }
                    if app.settings.theme != Theme::System {
                        // Re-assert each frame so a theme chosen here is not
                        // overridden by a later system change.
                        app.settings.theme.apply(&ctx);
                    }

                    ui.add_space(10.0);
                    ui.separator();

                    // --- Defaults -------------------------------------------
                    ui.label(egui::RichText::new("Defaults for new loads").strong());
                    ui.add_space(2.0);

                    ui.horizontal(|ui| {
                        ui.label("Sort");
                        egui::ComboBox::from_id_salt("limonada-settings-sort")
                            .selected_text(app.settings.sort.label())
                            .show_ui(ui, |ui| {
                                for sort in Sort::ALL {
                                    ui.selectable_value(
                                        &mut app.settings.sort,
                                        sort,
                                        sort.label(),
                                    );
                                }
                            });
                    });

                    ui.horizontal(|ui| {
                        ui.label("Listing");
                        egui::ComboBox::from_id_salt("limonada-settings-listing")
                            .selected_text(app.settings.listing.label())
                            .show_ui(ui, |ui| {
                                for listing in ListingType::ALL {
                                    ui.selectable_value(
                                        &mut app.settings.listing,
                                        listing,
                                        listing.label(),
                                    );
                                }
                            });
                    });

                    ui.horizontal(|ui| {
                        ui.label("Comments");
                        egui::ComboBox::from_id_salt("limonada-settings-comment-sort")
                            .selected_text(app.settings.comment_sort.label())
                            .show_ui(ui, |ui| {
                                for sort in CommentSort::ALL {
                                    ui.selectable_value(
                                        &mut app.settings.comment_sort,
                                        sort,
                                        sort.label(),
                                    );
                                }
                            });
                    });
                    ui.label(
                        egui::RichText::new(
                            "These apply the next time the feed or a thread loads.",
                        )
                        .weak()
                        .small(),
                    );

                    ui.add_space(10.0);
                    ui.separator();

                    // --- Diagnostics ----------------------------------------
                    ui.label(egui::RichText::new("About").strong());
                    ui.add_space(2.0);
                    ui.label(format!("Limonada {}", env!("CARGO_PKG_VERSION")));
                    ui.label(
                        egui::RichText::new(crate::api::transport::USER_AGENT)
                            .weak()
                            .small(),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(format!("Source: {}", app.source_label()))
                            .weak()
                            .small(),
                    );
                    if let Some(info) = app.site.info.as_ref() {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} · lemmy {} · {} users",
                                info.site_view.site.name, info.version, info.site_view.counts.users
                            ))
                            .weak()
                            .small(),
                        );
                    }
                    if let Some(path) = app.settings_path_display() {
                        ui.label(
                            egui::RichText::new(format!("Settings file: {path}"))
                                .weak()
                                .small(),
                        );
                    }
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "Account: {}",
                            app.signed_in_as().unwrap_or("not signed in")
                        ))
                        .weak()
                        .small(),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(
                            "This version browses, searches, votes and comments. Creating posts, \
                             the inbox, notifications and multiple accounts are not \
                             implemented.",
                        )
                        .weak()
                        .small(),
                    );
                });
        });

    if close {
        app.settings_open = false;
    }
    if apply {
        app.reload_all(&ctx);
        app.status = Some(Status::info("Reloaded from the configured source."));
    }
}
