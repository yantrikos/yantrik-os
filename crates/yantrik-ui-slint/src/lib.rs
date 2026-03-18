//! Yantrik OS — Slint UI types.
//!
//! This crate owns the Slint compilation step. It compiles all `.slint` files
//! and re-exports the generated types (`App`, data structs, callbacks).
//!
//! Separated from yantrik-ui so that Rust-only changes (wire/*.rs) don't
//! trigger Slint recompilation (~10 min saved per incremental build).

slint::include_modules!();
