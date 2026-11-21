//! The central region: either the feed or an open post.

use eframe::egui;

use crate::app::{Limonada, View};

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| match app.view {
        View::Feed => super::feed::show(app, ui),
        View::Post => super::detail::show(app, ui),
        View::Search => super::search::show(app, ui),
    });
}
