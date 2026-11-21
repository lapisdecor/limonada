//! The Lemmy API layer.
//!
//! Layering, outermost first:
//!
//! * [`client::LemmyClient`] — typed methods (`site`, `list_posts`, …).
//! * [`transport::Transport`] — a synchronous source of JSON, either
//!   [`transport::HttpTransport`] (live) or [`transport::FixtureTransport`]
//!   (offline demo and tests).
//! * [`job`] — runs a blocking request on a worker thread and reports back over
//!   a channel, so the immediate-mode UI never blocks.
//! * [`types`] — serde models for the wire shapes.
//! * [`params`] — `Sort` / `ListingType` query values.

pub mod client;
pub mod error;
pub mod job;
pub mod params;
pub mod transport;
pub mod types;

pub use client::{DEFAULT_PAGE_SIZE, LemmyClient, MAX_PAGE_SIZE, clamp_score};
pub use error::ApiError;
pub use params::{CommentSort, ListingType, SearchType, Sort};
pub use transport::{FixtureTransport, HttpTransport, RefusingTransport, Transport};
