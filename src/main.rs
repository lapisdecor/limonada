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

use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;
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

/// Limits to request from the GPU, trimmed to what the adapter reports.
///
/// egui-wgpu 0.36.2 builds its device request from `wgpu::Limits::default()`
/// (and from `wgpu::Limits::downlevel_webgl2_defaults()` on the GL backend,
/// which inherits the same value), and both ask for
/// `max_color_attachments: 8`. OpenGL ES 3.0 only guarantees 4, and the driver
/// on a Raspberry Pi reports 4, so startup fails before the first frame with
///
/// ```text
/// Error: Wgpu(RequestDeviceError(RequestDeviceError { inner: Core(
/// LimitsExceeded(FailedLimit { name: "max_color_attachments",
/// requested: 8, allowed: 4 })) }))
/// ```
///
/// That is wgpu's own overshoot (gfx-rs/wgpu issues #6986 and #8189), not
/// something egui needs: it draws into a single colour attachment, so the
/// default buys nothing here. Taking the adapter's own limits instead means every requested
/// value is one the driver has already said it supports, on x86 and ARM alike.
/// `max_texture_dimension_2d` keeps egui's 8192 ceiling where the adapter can
/// reach it — egui-wgpu raises it that high so 4K surfaces fit — and is the
/// adapter's own maximum where it cannot.
fn wgpu_device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let supported = adapter.limits();
    wgpu::DeviceDescriptor {
        label: Some("limonada wgpu device"),
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: supported.max_texture_dimension_2d.min(8192),
            ..supported
        },
        ..Default::default()
    }
}

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        renderer: renderer_from_env(),
        // `CreateNew` is eframe's default setup; starting from
        // `without_display_handle()` keeps its backend/power-preference
        // environment handling (`WGPU_BACKEND`, `WGPU_POWER_PREF`) while giving
        // this app a say in the device request. eframe fills the display handle
        // in itself when one is needed (Wayland with GLES).
        wgpu_options: eframe::WgpuConfiguration {
            wgpu_setup: WgpuSetup::CreateNew(WgpuSetupCreateNew {
                device_descriptor: Arc::new(wgpu_device_descriptor),
                ..WgpuSetupCreateNew::without_display_handle()
            }),
            ..Default::default()
        },
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
