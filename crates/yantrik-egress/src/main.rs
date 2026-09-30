//! yantrik-egress — the one way out for the mind account.
//!
//! A forward proxy on loopback that serves one account, `yantrik-mind`, and lets it reach only
//! what the person's policy allows. Tunnels only (`CONNECT`), plus plain HTTP where a rule says so;
//! no TLS is ever opened here, so it sees where the mind connects and never what it says. See
//! design/mind-egress-2026-09-29.md.
//!
//! Configured by its unit's environment:
//!
//! | variable | default | |
//! |---|---|---|
//! | `EGRESS_LISTEN` | `127.0.0.1:7450` | where the mind's `HTTPS_PROXY` points |
//! | `EGRESS_STATE` | `/var/lib/yantrik-egress` | the policy, the ledger, Private mode |
//! | `EGRESS_CONTROL` | `/run/yantrik-egress/control` | the desktop's socket |
//! | `EGRESS_SERVE_UID` | the uid of `yantrik-mind` | the one account served |
//! | `EGRESS_OWNER_UID` | — | the desktop's owner, who may use the control socket beside root |

mod control;
mod ledger;
mod local;
mod peer;
mod policy;
mod proxy;
mod request;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the ledger is written when it changed.
const FLUSH: Duration = Duration::from_secs(30);

fn env(name: &str, default: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

#[cfg(unix)]
fn uid_of_user(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    // SAFETY: getpwnam with a NUL-terminated name; the result is read at once, before any other
    // call that could reuse its buffer.
    let pw = unsafe { libc::getpwnam(c.as_ptr()) };
    (!pw.is_null()).then(|| unsafe { (*pw).pw_uid })
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    if let Err(e) = run().await {
        tracing::error!(error = %e, "yantrik-egress stopped");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let listen: SocketAddr = env("EGRESS_LISTEN", "127.0.0.1:7450").parse().map_err(|e| format!("EGRESS_LISTEN: {e}"))?;
    if !listen.ip().is_loopback() {
        return Err("EGRESS_LISTEN must be a loopback address: this proxy is for this machine's mind only".into());
    }
    let dir = PathBuf::from(env("EGRESS_STATE", "/var/lib/yantrik-egress"));
    let control_path = PathBuf::from(env("EGRESS_CONTROL", "/run/yantrik-egress/control"));
    let serve_uid = match std::env::var("EGRESS_SERVE_UID").ok().and_then(|v| v.parse().ok()) {
        Some(u) => u,
        None => uid_of_user("yantrik-mind").ok_or("no yantrik-mind account, and EGRESS_SERVE_UID is not set")?,
    };
    if serve_uid == 0 {
        return Err("the served account cannot be root".into());
    }
    let owner: Option<u32> = std::env::var("EGRESS_OWNER_UID").ok().and_then(|v| v.parse().ok()).filter(|u| *u != serve_uid);

    let state = Arc::new(Mutex::new(state::State::load(&dir)));
    {
        let s = state.lock().map_err(|_| "state poisoned")?;
        tracing::info!(mode = ?s.policy.mode, rules = s.policy.rules.len(), private = s.private, serve_uid, %listen, "yantrik-egress starting");
    }

    let _ = std::fs::remove_file(&control_path);
    let control = tokio::net::UnixListener::bind(&control_path).map_err(|e| format!("control socket {}: {e}", control_path.display()))?;
    // Anyone may connect; the kernel's word on who they are decides whether they are answered.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&control_path, std::fs::Permissions::from_mode(0o666)).map_err(|e| e.to_string())?;
    }
    tokio::spawn(control::serve(control, state.clone(), owner));

    {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(FLUSH).await;
                if let Ok(mut s) = state.lock() {
                    if let Err(e) = s.save_ledger() {
                        tracing::warn!(error = %e, "the ledger could not be written");
                    }
                }
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|e| format!("{listen}: {e}"))?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    proxy::serve(listener, Arc::new(proxy::Proxy { state, serve_uid, local })).await;
    Ok(())
}
