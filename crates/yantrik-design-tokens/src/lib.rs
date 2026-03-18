// Design tokens are pure .slint files — no Rust code needed.
// Consuming crates access them via the build-time DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH env var,
// which is set by this crate's build.rs via the `links` mechanism.
