// Nothing in this crate needs `unsafe`, and the binary is the crate most likely
// to grow platform glue, so the guarantee is enforced rather than assumed.
#![forbid(unsafe_code)]
// A release build on Windows is a GUI program, not a console one: without this
// the linker marks the executable CUI, and launching it from Explorer opens a
// console window behind the app that stays there for as long as it runs.
// Debug builds keep the console so the startup logging below is visible.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use eframe::egui;
use limonada::app::Limonada;

/// Which rendering backend to use.
///
/// `wgpu` is the default. `glow` is selectable via `LIMONADA_RENDERER=glow`,
/// which is needed for eframe's `EFRAME_SCREENSHOT_TO` render-proof path: in
/// eframe 0.36.2 that path panics with "not yet implemented for wgpu backend",
/// so a reproducible screenshot requires glow.
fn renderer_from_env() -> eframe::Renderer {
    match std::env::var("LIMONADA_RENDERER").as_deref() {
        Ok("glow") => eframe::Renderer::Glow,
        Ok("wgpu") | Ok("") | Err(_) => eframe::Renderer::Wgpu,
        Ok(other) => {
            eprintln!("limonada: unknown LIMONADA_RENDERER={other:?}, using wgpu");
            eframe::Renderer::Wgpu
        }
    }
}

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        renderer: renderer_from_env(),
        viewport: egui::ViewportBuilder::default()
            .with_title("Limonada")
            .with_app_id("com.gatochalupa.limonada")
            .with_inner_size([1200.0, 820.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "limonada",
        native_options,
        Box::new(|cc| Ok(Box::new(Limonada::new(cc)))),
    )
}
