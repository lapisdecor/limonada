//! The user interface.
//!
//! Each submodule owns one region of the window. Every entry point has the same
//! shape — `show(app: &mut Limonada, ui: &mut egui::Ui)` — and either renders
//! from state or records an action for [`crate::app::Limonada`] to apply after
//! the frame.

pub mod auth;
pub mod central;
pub mod communities;
pub mod detail;
pub mod feed;
pub mod image_viewer;
pub mod search;
pub mod settings;
pub mod statusbar;
pub mod topbar;
pub mod widgets;
