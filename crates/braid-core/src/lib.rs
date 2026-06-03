//! braid-core: schema types, hydrate/reconcile, and domain logic for braid.
//!
//! This crate performs no I/O. It defines the skein document schema
//! (see `claude-notes/plans/2026/06/03/braid-design-kickoff.md`), converts
//! between automerge documents and plain Rust values (hydrate/reconcile),
//! and implements domain logic (id generation, ready/blocked computation).

pub mod amdoc;
pub mod domain;
pub mod id;
pub mod schema;
pub mod time;
