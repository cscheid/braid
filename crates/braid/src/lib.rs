//! braid: an automerge-centric issue tracker for LLM agents.
//!
//! The binary's logic lives in this library so integration tests can call
//! it directly; `main.rs` is a thin clap dispatcher.

// config and docid live in braid-config; re-export so callers keep stable paths.
pub use braid_config::config;
pub use braid_config::docid;

pub mod cache;
pub mod commands;
pub mod import;
pub mod mcp;
pub mod ops;
pub mod skein;
pub mod sync;
pub mod ui;
pub mod ws;
