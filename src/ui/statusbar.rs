//! The bottom status bar: where the data comes from, and any transient message.

use eframe::egui;

use crate::app::{Limonada, Status};

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    egui::Panel::bottom("limonada-status").show(ui, |ui| {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            match app.status.clone() {
                Some(Status { text, is_error: true }) => {
                    ui.label(
                        egui::RichText::new(format!("⚠ {text}"))
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                    if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                        app.status = None;
                    }
                }
                Some(Status {
                    text,
                    is_error: false,
                }) => {
                    ui.label(text);
                    if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                        app.status = None;
                    }
                }
                None => {
                    ui.label(egui::RichText::new(activity(app)).weak().small());
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(username) = app.signed_in_as() {
                    ui.label(
                        egui::RichText::new(format!("👤 {username}")).weak().small(),
                    )
                    .on_hover_text("Signed in");
                }
                let version = app
                    .site
                    .info
                    .as_ref()
                    .map(|info| info.version.clone())
                    .filter(|version| !version.is_empty());
                if let Some(version) = version {
                    ui.label(egui::RichText::new(format!("lemmy {version}")).weak().small())
                        .on_hover_text("Version reported by the instance");
                }
                ui.label(
                    egui::RichText::new(app.source_label())
                        .weak()
                        .small(),
                )
                .on_hover_text("Where responses are being loaded from");
            });
        });
        ui.add_space(3.0);
    });
}

/// What is currently happening, for the idle case.
fn activity(app: &Limonada) -> String {
    let mut parts: Vec<String> = Vec::new();
    if app.feed.is_busy() {
        parts.push("loading posts".into());
    }
    if app.communities.is_busy() {
        parts.push("loading communities".into());
    }
    if app.search.is_busy() {
        parts.push("searching".into());
    }
    if app.votes.is_busy() {
        parts.push("sending a vote".into());
    }
    if let Some(detail) = &app.detail {
        if detail.post_phase.is_loading() {
            parts.push("loading post".into());
        }
        if detail.thread_phase.is_loading() {
            parts.push("loading comments".into());
        }
    }
    if parts.is_empty() {
        let posts = app.feed.items.len();
        let comments = app
            .detail
            .as_ref()
            .map(|detail| detail.comment_count())
            .unwrap_or(0);
        return match app.view {
            crate::app::View::Feed => format!("{posts} posts loaded"),
            crate::app::View::Post => format!("{comments} comments in thread"),
            crate::app::View::Search => {
                let scope = app
                    .feed
                    .scope
                    .as_ref()
                    .map(|scope| format!(" in !{}", scope.name))
                    .unwrap_or_default();
                format!("{} results{scope}", app.search.total())
            }
        };
    }
    parts.join(", ")
}
