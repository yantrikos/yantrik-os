//! Yantrik OS cognition engine — the LLM-free "autonomous brain loop".
//!
//! These four modules were added to Yantrik OS in commit 5dc0448 ("Autonomous
//! brain loop: DB-native cognition engine without LLM") and lived inside the
//! vendored copy of yantrikdb-core, which made them look like database code.
//! They are not: they touch no YantrikDB type at all, only a plain
//! `rusqlite::Connection`. Keeping them here lets Yantrik OS depend on the
//! upstream yantrikdb release instead of maintaining a fork of it.
//!
//! - [`brain`]                — homeostasis, candidate generation, signal typing
//! - [`detectors`]            — signal detectors over recorded events
//! - [`curiosity`]            — curiosity sources and scoring
//! - [`brain_consolidation`]  — idle-time consolidation pass

pub mod brain;
pub mod brain_consolidation;
pub mod curiosity;
pub mod detectors;
