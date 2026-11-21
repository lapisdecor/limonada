//! Full-resolution picture viewing.
//!
//! A post's picture is fetched as its thumbnail and drawn small, which is what
//! makes a page of posts cheap — and also why the same picture can look soft or
//! unreadable there. Clicking it opens this overlay, which loads the best image
//! the post offers ([`crate::state::best_image_url`]) and draws it as large as
//! the window allows, with a 1:1 mode for images no screen can show at once.
//!
//! The overlay owns nothing: it reads [`Viewer`] and reports close, zoom and
//! open-in-browser back to [`Limonada`], which applies them after the frame.

use eframe::egui;

use crate::app::Limonada;
use crate::state::{Viewer, Zoom};
use crate::ui::widgets;

/// Space kept free inside the overlay for the caption above and the alt text
/// and controls below, so a picture as tall as the window cannot push its own
/// controls off the screen.
const CONTROL_SPACE: f32 = 124.0;

/// The box a picture may occupy in a window of `screen` size.
///
/// Never returns a degenerate box: the minimum is what a small window can still
/// show, and a window too small even for that scrolls rather than collapsing.
fn fit_budget(screen: egui::Vec2) -> egui::Vec2 {
    egui::vec2(
        (screen.x - 40.0).max(200.0),
        (screen.y - CONTROL_SPACE).max(120.0),
    )
}

pub fn show(app: &mut Limonada, ctx: &egui::Context) {
    let budget = fit_budget(ctx.content_rect().size());
    let mut close = false;
    let mut toggle_zoom = false;
    let mut open_in_browser = false;
    let mut served: Option<egui::Vec2> = None;

    if let Some(viewer) = app.image.as_ref() {
        let modal = egui::Modal::new(egui::Id::new("limonada-picture"))
            // Darker than egui's default backdrop: a picture needs to be read
            // against the interface being dimmed away, not next to it.
            .backdrop_color(egui::Color32::from_black_alpha(190))
            .show(ctx, |ui| {
                caption(ui, viewer);
                ui.add_space(6.0);
                let loaded = draw_picture(ui, viewer, budget);
                if let Some(alt) = viewer.alt.as_deref() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(alt).weak().small());
                }
                ui.add_space(6.0);
                controls(
                    ui,
                    viewer,
                    loaded,
                    &mut toggle_zoom,
                    &mut open_in_browser,
                );
                served = loaded;
            });

        // egui's own dismissal: a click on the backdrop outside the picture,
        // `Esc` while this is the topmost layer, or any widget inside that asked
        // to close with `ui.close()` — which is how the ✕ in `caption` reports.
        // Using egui's signal rather than checking keys by hand keeps `Esc` from
        // being swallowed by whatever else may be open.
        //
        // This is the *only* place `close` is written for the overlay, and it
        // must stay that way: `should_close` is an assignment, so a close
        // reported by the button into a flag of our own would be overwritten
        // here and the button would appear dead. It did exactly that.
        close = modal.should_close();
    }

    report_loaded(app, served);

    if close {
        app.close_image(ctx);
        return;
    }
    if toggle_zoom
        && let Some(viewer) = app.image.as_mut()
    {
        viewer.zoom = viewer.zoom.toggled();
    }
    if open_in_browser
        && let Some(viewer) = app.image.as_ref()
    {
        // The system browser is the only place a huge image, an animation or an
        // SVG can be inspected at its own pace.
        ctx.open_url(egui::OpenUrl::new_tab(viewer.url.clone()));
    }
}

/// Say once, under `LIMONADA_LOG=1`, that the picture arrived — and at what
/// size. This is what distinguishes "the overlay opened" from "the bytes were
/// fetched, decoded and uploaded", which is the part no headless test can
/// reach: those install no image loaders, so nothing in them touches the
/// network on purpose.
fn report_loaded(app: &mut Limonada, served: Option<egui::Vec2>) {
    let Some(size) = served else {
        return;
    };
    let Some(viewer) = app.image.as_mut() else {
        return;
    };
    if viewer.reported_size {
        return;
    }
    viewer.reported_size = true;
    if crate::state::diagnostics_enabled() {
        eprintln!(
            "limonada: picture loaded at {}x{} px ({})",
            size.x.round() as i64,
            size.y.round() as i64,
            viewer.zoom.label(),
        );
    }
}

/// Where the picture comes from, and how much of it this is.
fn caption(ui: &mut egui::Ui, viewer: &Viewer) {
    ui.horizontal(|ui| {
        let host = widgets::host_of(&viewer.url).unwrap_or_else(|| "picture".to_string());
        ui.label(egui::RichText::new(host).strong());
        ui.label(
            egui::RichText::new(viewer.resolution_note())
                .weak()
                .small(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("✕")
                .on_hover_text("Close (Esc)")
                .clicked()
            {
                // egui's own close signal: the enclosing modal's area turns a
                // `ui.close()` from anywhere inside it into its own close
                // response, which `show` reads. Reporting through a flag of our
                // own instead is what made this button do nothing.
                ui.close();
            }
        });
    });

    // A picture this build cannot decode draws egui's "could not load" glyph
    // and nothing else. Saying which format it is turns a broken image into a
    // problem the user can act on, and the button below is the action.
    if let Some(note) = viewer.undecodable_note() {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(note)
                .small()
                .color(egui::Color32::from_rgb(230, 170, 60)),
        );
    }
}

/// Draw the picture, and report the pixel size it was served at when the loader
/// knows it. The size comes from the texture rather than from the post's
/// metadata, which describes the thumbnail and would misreport a larger image.
fn draw_picture(ui: &mut egui::Ui, viewer: &Viewer, budget: egui::Vec2) -> Option<egui::Vec2> {
    let loaded = egui::Image::new(&viewer.url)
        .load_for_size(ui.ctx(), budget)
        .ok()
        .and_then(|poll| poll.size());

    match viewer.zoom {
        Zoom::Fit => {
            // `fit_to_original_size(1.0)` bounded by `max_size` means "as large
            // as this box allows, never larger than the picture itself" — a
            // small image is shown at its true size rather than blown up.
            ui.add(
                egui::Image::new(&viewer.url)
                    .fit_to_original_size(1.0)
                    .max_size(budget),
            );
        }
        Zoom::Actual => {
            // Deliberately unscaled and unbounded: overflowing the box is the
            // point, and the scroll area is what makes the overflow reachable.
            egui::ScrollArea::both()
                .max_width(budget.x)
                .max_height(budget.y)
                .show(ui, |ui| {
                    ui.add(egui::Image::new(&viewer.url).fit_to_original_size(1.0));
                });
        }
    }

    loaded
}

/// The zoom toggle, the escape hatch to a browser, and where the user is told
/// how to leave.
fn controls(
    ui: &mut egui::Ui,
    viewer: &Viewer,
    served: Option<egui::Vec2>,
    toggle_zoom: &mut bool,
    open_in_browser: &mut bool,
) {
    ui.horizontal(|ui| {
        let next = viewer.zoom.toggled();
        if ui
            .button(next.label())
            .on_hover_text(match next {
                Zoom::Fit => "Shrink the picture so the whole of it fits the window",
                Zoom::Actual => "One image pixel per screen pixel, scrolling as needed",
            })
            .clicked()
        {
            *toggle_zoom = true;
        }
        if ui
            .button("Open in browser")
            .on_hover_text("Hand the image URL to your system browser")
            .clicked()
        {
            *open_in_browser = true;
        }
        if let Some(size) = served {
            ui.label(
                egui::RichText::new(format!(
                    "{} × {} px",
                    size.x.round() as i64,
                    size.y.round() as i64
                ))
                .weak()
                .small(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new("Esc closes").weak().small());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::fit_budget;

    #[test]
    fn the_picture_box_leaves_room_for_the_controls() {
        let budget = fit_budget(egui::vec2(1280.0, 820.0));
        assert_eq!(budget, egui::vec2(1240.0, 696.0));

        // A window too small for the overlay must still yield a usable box
        // rather than a negative or zero one, which would divide by zero inside
        // egui's scaling.
        let tiny = fit_budget(egui::vec2(0.0, 0.0));
        assert!(tiny.x >= 200.0 && tiny.y >= 120.0, "got {tiny:?}");
    }
}
