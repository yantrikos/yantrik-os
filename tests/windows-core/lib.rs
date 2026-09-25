// The taskbar's own two modules used to sit here as `#[path]` mirrors of their yantrik-ui
// sources. `windows.rs` has since grown references into the shell it lives in —
// `crate::agents::WINDOW_TITLE_PREFIX`, and `crate::wire::dock` and `crate::apps` inside its
// name-and-surface tests — so it cannot compile outside yantrik-ui any more, and this crate
// has not since 22877dd wrote it: #85 found a mirror that no CI and no laptop had built.
// Those tests run in CI from yantrik-ui's own test binary. `running.rs` is still self-
// contained, and the fast lane for the taskbar state it holds — #76's underline fix among
// them — is what this crate is for.
#[path="../../crates/yantrik-ui/src/running.rs"]
pub mod running;
