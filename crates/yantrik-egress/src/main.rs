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
//! | `EGRESS_LISTEN` | `127.0.0.1:7450` | the endpoint door, where the mind's `HTTPS_PROXY` points: `lan` rules apply |
//! | `EGRESS_PUBLIC_LISTEN` | `127.0.0.1:7451` | the public door, for anything fetched for someone else: the internet only (`door`). Pinned: any other value is refused ([`PUBLIC_DOOR`]) |
//! | `EGRESS_STATE` | `/var/lib/yantrik-egress` | the policy, the ledger, Private mode |
//! | `EGRESS_CONTROL` | `/run/yantrik-egress/control` | the desktop's socket |
//! | `EGRESS_SERVE_UID` | the uid of `yantrik-mind` | the one account served |
//! | `EGRESS_OWNER_UID` | — | the desktop's owner, who may use the control socket beside root |
//!
//! `yantrik-egress direct [STATE_DIR]` prints the direct set instead and exits: where the kernel
//! lets the mind account connect without this proxy (`direct`, `yantrik-update mind-egress`).
//! `yantrik-egress seed-plan` reads `<source> <url>` lines and prints which become seeded `lan`
//! rules (`seed`, `yantrik-update`'s seeding). `yantrik-egress capabilities` prints what this
//! build promises, one word a line (`policy::CAPABILITIES`). `yantrik-egress lan-hosts
//! [STATE_DIR]` prints every host a `lan` rule names, with its ports (`direct`). `yantrik-egress
//! snapshot [STATE_DIR]` prints both, `direct`'s lines then `lan-hosts`', from one read of the
//! policy (what `yantrik-update mind-egress` asks).

mod control;
mod direct;
mod door;
#[cfg(test)]
mod door_tests;
mod ledger;
mod local;
mod netlink;
mod peer;
mod policy;
mod proxy;
mod ranges;
mod request;
mod seed;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the ledger is written when it changed.
const FLUSH: Duration = Duration::from_secs(30);

/// The public door's one address. Pinned, not configured: the kernel's table lets the mind reach
/// this port (`MIND_LOOPBACK_PORTS` in `yantrik-update`) and the Mind's status file names it
/// (`public_proxy`), so a drop-in that moved it would send untrusted fetches to a port this proxy
/// does not serve, and anything else that bound it would get them.
const PUBLIC_DOOR: &str = "127.0.0.1:7451";

/// The public door from `EGRESS_PUBLIC_LISTEN`: unset, empty or [`PUBLIC_DOOR`] itself; anything
/// else is refused and the proxy does not start.
fn public_door(value: Option<&str>) -> Result<SocketAddr, String> {
    let pinned: SocketAddr = PUBLIC_DOOR.parse().map_err(|e| format!("{PUBLIC_DOOR}: {e}"))?;
    match value.filter(|v| !v.is_empty()) {
        None => Ok(pinned),
        Some(v) if v.parse::<SocketAddr>().ok() == Some(pinned) => Ok(pinned),
        Some(v) => Err(format!(
            "EGRESS_PUBLIC_LISTEN is {v}, but the public door is pinned to {PUBLIC_DOOR}: the kernel's table and the Mind's status file name that port"
        )),
    }
}

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("direct") if args.len() <= 2 => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env("EGRESS_STATE", "/var/lib/yantrik-egress")));
            match direct::export(&dir) {
                Ok(text) => print!("{text}"),
                Err(e) => {
                    eprintln!("yantrik-egress direct: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Some("lan-hosts") if args.len() <= 2 => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env("EGRESS_STATE", "/var/lib/yantrik-egress")));
            match direct::export_lan_hosts(&dir) {
                Ok(text) => print!("{text}"),
                Err(e) => {
                    eprintln!("yantrik-egress lan-hosts: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Some("snapshot") if args.len() <= 2 => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env("EGRESS_STATE", "/var/lib/yantrik-egress")));
            match direct::export_snapshot(&dir) {
                Ok(text) => print!("{text}"),
                Err(e) => {
                    eprintln!("yantrik-egress snapshot: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Some("seed-plan") if args.len() == 1 => {
            use std::io::Read;
            let mut input = String::new();
            if std::io::stdin().take(64 * 1024).read_to_string(&mut input).is_err() {
                eprintln!("yantrik-egress seed-plan: stdin is not text");
                std::process::exit(1);
            }
            print!("{}", seed::plan_text(&input));
            return;
        }
        Some("capabilities") if args.len() == 1 => {
            for c in policy::CAPABILITIES {
                println!("{c}");
            }
            return;
        }
        Some(_) => {
            eprintln!("usage: yantrik-egress            the proxy, configured by its unit's environment\n       yantrik-egress direct [DIR]   print the direct set from the policy in DIR\n       yantrik-egress lan-hosts [DIR] print every host a lan rule names, with its ports\n       yantrik-egress snapshot [DIR]  both of those, from one read of the policy\n       yantrik-egress seed-plan      <source> <url> lines on stdin: which become seeded lan rules\n       yantrik-egress capabilities   print what this build promises, one word a line");
            std::process::exit(2);
        }
    }
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let runtime = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "yantrik-egress could not start its runtime");
            std::process::exit(1);
        }
    };
    if let Err(e) = runtime.block_on(run()) {
        tracing::error!(error = %e, "yantrik-egress stopped");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let listen = loopback("EGRESS_LISTEN", "127.0.0.1:7450")?;
    let public_listen = public_door(std::env::var("EGRESS_PUBLIC_LISTEN").ok().as_deref())?;
    if public_listen == listen {
        return Err("EGRESS_PUBLIC_LISTEN must not be EGRESS_LISTEN: the two doors are two ports".into());
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
        tracing::info!(mode = ?s.policy.mode, rules = s.policy.rules.len(), private = s.private, serve_uid, %listen, %public_listen, "yantrik-egress starting");
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

    // Both doors are bound before either serves: a proxy with one door is not started.
    let open = Arc::new(tokio::sync::Semaphore::new(proxy::MOST_OPEN));
    let net = Arc::new(local::Watch::new(local::addresses, local::Watch::FRESH));
    let mut doors = Vec::new();
    for (addr, door) in [(listen, door::Door::Endpoint), (public_listen, door::Door::Public)] {
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("{addr}: {e}"))?;
        let local = listener.local_addr().map_err(|e| e.to_string())?;
        doors.push((listener, Arc::new(proxy::Proxy { state: state.clone(), serve_uid, local, door, net: net.clone(), world: proxy::World::REAL })));
    }
    let (public, endpoint) = (doors.pop().ok_or("no public door")?, doors.pop().ok_or("no endpoint door")?);
    tokio::join!(proxy::serve(endpoint.0, endpoint.1, open.clone()), proxy::serve(public.0, public.1, open));
    Ok(())
}

/// A listening address from `name`, which must be on loopback: this proxy is for this machine's
/// mind only.
fn loopback(name: &str, default: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = env(name, default).parse().map_err(|e| format!("{name}: {e}"))?;
    if !addr.ip().is_loopback() {
        return Err(format!("{name} must be a loopback address: this proxy is for this machine's mind only"));
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_door_is_pinned() {
        let pinned: SocketAddr = PUBLIC_DOOR.parse().unwrap();
        for v in [None, Some(""), Some("127.0.0.1:7451")] {
            assert_eq!(public_door(v), Ok(pinned), "{v:?}");
        }
        for v in ["127.0.0.1:7452", "127.0.0.2:7451", "[::1]:7451", "0.0.0.0:7451", "localhost:7451", "7451"] {
            assert!(public_door(Some(v)).is_err(), "{v}");
        }
    }
}
