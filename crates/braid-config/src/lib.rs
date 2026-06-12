//! Lean config, docid, and viewer registry for braid.
//!
//! No heavy runtime dependencies (no tokio, axum, samod, or rust-embed).
//! Used by both the `braid` CLI and the `braid-viewer` Tauri app.

pub mod config;
pub mod docid;
pub mod ui_config;
pub mod viewer;
