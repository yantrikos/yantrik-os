//! What the panel's buttons do: sign in, install, add an account, use one.
//!
//! Signing in and installing open a terminal — `foot`, which the image ships — running the
//! vendor's own command from `vendors::VENDORS`, with the person watching and answering it. It
//! stays open when the command ends (`--hold`), so what the vendor said is there to read.
//!
//! "Use" points programs started from now on at an account: the vendor's variable
//! (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) goes into the environment of every app the shell launches
//! ([`launch_env`], read by `wire::dock::session_env`) and of the person's user services
//! (`systemctl --user set-environment`), where the harnesses run. A program already running keeps
//! the account it started with; the first account is the vendor's default, so using it unsets
//! the variable rather than naming the directory.

use std::path::Path;
use std::process::Command;
use std::sync::RwLock;

use super::store::{self, Store, PRIMARY};
use super::vendors::{SignIn, Vendor, VENDORS};

/// The variables every launch gets: one per vendor whose active account is not its first.
static LAUNCH_ENV: RwLock<Vec<(&'static str, String)>> = RwLock::new(Vec::new());

/// What `session_env` adds to a launch.
pub fn launch_env() -> Vec<(&'static str, String)> {
    LAUNCH_ENV.read().map(|v| v.clone()).unwrap_or_default()
}

/// The variables the store asks for: `(variable, Some(dir))` to set, `(variable, None)` to unset.
pub fn env_of(home: &Path, store: &Store) -> Vec<(&'static str, Option<String>)> {
    VENDORS
        .iter()
        .filter_map(|v| {
            let var = v.home_env?;
            let label = store.active(v.id);
            let dir = (label != PRIMARY).then(|| store::dir_of(home, v, label).to_string_lossy().into_owned());
            Some((var, dir))
        })
        .collect()
}

/// Make the store's choices true for everything started from now on.
pub fn apply(home: &Path, store: &Store) {
    // An account whose directory is not the person's own all the way down is not exported: the
    // vendor's program would load its settings and hooks from there. It answers with its first
    // account instead, and says so.
    let env: Vec<_> = env_of(home, store)
        .into_iter()
        .map(|(var, dir)| match dir {
            Some(d) => match store::own_dir(home, Path::new(&d), false) {
                Ok(()) => (var, Some(d)),
                Err(e) => {
                    tracing::error!(var, dir = %d, error = %e, "an account's directory is not the person's own; not using it");
                    (var, None)
                }
            },
            None => (var, None),
        })
        .collect();
    if let Ok(mut held) = LAUNCH_ENV.write() {
        *held = env.iter().filter_map(|(k, v)| v.clone().map(|v| (*k, v))).collect();
    }
    for (var, dir) in env {
        let mut cmd = Command::new("systemctl");
        cmd.arg("--user");
        match &dir {
            Some(d) => cmd.arg("set-environment").arg(format!("{var}={d}")),
            None => cmd.arg("unset-environment").arg(var),
        };
        match cmd.output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => tracing::warn!(var, status = %o.status, "the user manager did not take the account's variable"),
            Err(e) => tracing::warn!(var, error = %e, "systemctl could not be run for the account's variable"),
        }
    }
}

/// Make `id` the account its vendor answers with.
pub fn use_account(home: &Path, id: &str) -> Result<(), String> {
    let (vendor, label) = super::parse_id(id).ok_or("not an account id")?;
    let path = store::path_in(home);
    let mut s = Store::load(&path);
    s.use_label(vendor.id, label)?;
    s.save(&path).map_err(|e| format!("accounts.json could not be written: {e}"))?;
    apply(home, &s);
    Ok(())
}

/// What pressing an account's own button, or a "+" choice, turned into.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// A terminal, running the vendor's command.
    Terminal,
    /// Keys live in Settings → AI; the caller goes there.
    Settings,
}

/// Sign `id` in: its vendor's command, in a terminal, pointed at its directory.
pub fn sign_in(home: &Path, id: &str) -> Result<Opened, String> {
    let (vendor, label) = super::parse_id(id).ok_or("not an account id")?;
    let SignIn::Program { login } = vendor.sign_in else {
        return Ok(Opened::Settings);
    };
    if label != PRIMARY && !Store::load(&store::path_in(home)).labels(vendor.id).iter().any(|l| l == label) {
        return Err(format!("{} has no account {label}", vendor.name));
    }
    let dir = store::dir_of(home, vendor, label);
    if label != PRIMARY {
        store::own_dir(home, &dir, true).map_err(|e| format!("the account's directory could not be made: {e}"))?;
    }
    terminal(&format!("Sign in to {}", vendor.name), login, vendor, (label != PRIMARY).then_some(dir.as_path()))?;
    Ok(Opened::Terminal)
}

/// Install `vendor`'s program, in a terminal.
pub fn install(vendor: &Vendor) -> Result<Opened, String> {
    if vendor.install.is_empty() {
        return Ok(Opened::Settings);
    }
    terminal(&format!("Install {}", vendor.name), vendor.install, vendor, None)?;
    Ok(Opened::Terminal)
}

/// The "+" list's choice for `vendor`: the one thing `super::choices` offered for it.
pub fn add(home: &Path, vendor_id: &str, facts: &super::Facts) -> Result<Opened, String> {
    let choice = super::choices(facts)
        .into_iter()
        .find(|c| c.vendor.id == vendor_id)
        .ok_or("there is nothing to add for that vendor")?;
    match choice.what {
        "Set up a key" => Ok(Opened::Settings),
        "Install" => install(choice.vendor),
        "Sign in" => sign_in(home, &super::account_id(vendor_id, PRIMARY)),
        "Add account" => {
            let path = store::path_in(home);
            let mut s = Store::load(&path);
            let label = s.add(choice.vendor)?;
            s.save(&path).map_err(|e| format!("accounts.json could not be written: {e}"))?;
            sign_in(home, &super::account_id(vendor_id, &label))
        }
        other => Err(format!("unknown choice {other}")),
    }
}

/// The vendor's command in a terminal, with the account's directory in the vendor's variable (or
/// the variable removed, for the vendor's first account). `command` is always a constant from
/// `VENDORS`.
fn terminal(title: &str, command: &str, vendor: &Vendor, dir: Option<&Path>) -> Result<(), String> {
    let env: Vec<crate::terminal_window::EnvChange<'_>> = match vendor.home_env {
        Some(var) => vec![(var, dir.map(Path::as_os_str))],
        None => Vec::new(),
    };
    crate::terminal_window::open(title, command, &env)
}

#[cfg(test)]
mod tests {
    use super::super::vendors::by_id;
    use super::*;

    #[test]
    fn the_environment_names_extra_accounts_and_unsets_the_first() {
        let home = Path::new("/home/p");
        let mut s = Store::default();
        s.add(by_id("codex").unwrap()).unwrap();
        s.use_label("codex", "account-2").unwrap();
        let env = env_of(home, &s);
        assert!(env.contains(&("CODEX_HOME", Some("/home/p/.local/share/yantrik/accounts/codex/account-2".into()))));
        assert!(env.contains(&("CLAUDE_CONFIG_DIR", None)), "the first account is the default");
        assert_eq!(env.len(), 2, "only vendors that can hold more than one account have a variable");
    }

    #[test]
    fn a_made_up_id_signs_nothing_in() {
        let home = std::env::temp_dir().join(format!("yantrik-act-{}", std::process::id()));
        assert!(sign_in(&home, "claude:account-4").is_err(), "an account the store does not have");
        assert!(sign_in(&home, "claude:/etc").is_err());
        assert_eq!(sign_in(&home, "xai:primary"), Ok(Opened::Settings));
    }
}
