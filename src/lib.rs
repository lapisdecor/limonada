//! Limonada — a desktop client for Lemmy.
//!
//! The crate is split into a library (this file) and a thin binary
//! (`src/main.rs`) so that the API layer, the comment-tree logic and the
//! settings store can be exercised by integration tests in `tests/` without
//! launching a GUI.

#![forbid(unsafe_code)]

pub mod api;
pub mod app;
pub mod avif;
pub mod state;
pub mod time;
pub mod tree;
pub mod ui;
