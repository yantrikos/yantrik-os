//! The policy half of the proxy, as a library: what a rule is and whether one lets a destination
//! through. The desktop reads it to know, before asking the person, whether a rule is missing
//! (Settings → Network → Web search). The proxy itself is `main.rs`; the control socket is the
//! only way anything changes the policy.

pub mod policy;
