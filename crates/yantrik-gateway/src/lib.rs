//! The local model gateway (#673): one OpenAI-compatible endpoint, owned by the OS, that every
//! mind is pointed at once and that forwards to whichever of the person's AI accounts a request
//! names. Keys never reach a harness: each is given a token for this gateway, and the gateway adds
//! the account's key as it forwards.
//!
//! ```text
//! GET  /v1/models              → the models this harness may use, `<account>/<model>`
//! POST /v1/chat/completions    → forwarded to the account; `stream: true` streams back
//!      Authorization: Bearer ygw-<64 hex>        (the harness's own token)
//!      {"model": "<account>/<model>", "effort": "easy|medium|high|xhigh", …}
//! ```
//!
//! `"model": "picked"` ([`PICKED`]) is whichever model the person picked for that mind in the
//! ask bar, resolved at each call.
//!
//! On 127.0.0.1:[`PORT`] only, and nothing else ([`server::bind`] refuses any other address, and
//! a connection from anywhere but loopback is closed unread). What it keeps:
//!
//! - **One token per harness** ([`tokens`]), minted by the OS and written into that harness's own
//!   config through `provider_handoff`, kept here as a SHA-256 only. A wrong token is a 401.
//! - **Private context stays where the person allowed it** ([`policy`]): a harness marked as
//!   sending the person's private context may use only the accounts the person allowed it for,
//!   and in Private mode only accounts on this machine or network.
//! - **Effort in the provider's own words** ([`body`]), with `yantrik_ml::model_caps`.
//! - **Every call logged without its content** ([`log`]): harness, account, model, effort, status,
//!   tokens and time, for the honest status and the audit view.

pub mod body;
pub mod http;
pub mod log;
pub mod policy;
pub mod route;
pub mod server;
pub mod tokens;
pub mod upstream;

#[cfg(test)]
mod tests;

/// The gateway's port, fixed and documented (docs/harness.md, "Yantrik models"): harness configs
/// name it, and the Mind's kernel table opens exactly it (deploy/yantrik-os/yantrik-update,
/// MIND_LOOPBACK_PORTS).
pub const PORT: u16 = 7460;

/// Where it listens.
pub const ADDR: &str = "127.0.0.1:7460";

/// The model id that means "the model the person picked for this mind in the ask bar": what a
/// harness that never reads a turn's options is configured with, so a pick takes effect on its
/// next call without its config being written again.
pub const PICKED: &str = "picked";

/// What a harness puts in `base_url`.
pub fn base_url() -> String {
    format!("http://{ADDR}/v1")
}

pub use policy::{Refusal, Target};
pub use server::{Accounts, Gateway, ModelInfo};
pub use tokens::{Grant, Tokens};
