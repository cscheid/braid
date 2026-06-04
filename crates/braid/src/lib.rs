//! braid: an automerge-centric issue tracker for LLM agents.
//!
//! The binary's logic lives in this library so integration tests can call
//! it directly; `main.rs` is a thin clap dispatcher.

pub mod cache;
pub mod commands;
pub mod config;
pub mod docid;
pub mod import;
pub mod sync;
pub mod skein;
