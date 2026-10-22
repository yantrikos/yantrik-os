//! What the rest of the system keeps under the four `yantrik/` roots: the services, the shell,
//! the harnesses and the deploy scripts, by the first name under `<root>/yantrik/`.
//!
//! The framework uses it twice. An app may name an old settings file to bring across
//! (`Settings::FORMER_FILE`), and a name here that is not that app's own is refused, so no app
//! is ever handed another component's file to read (email-service's `email.json` holds the
//! account password). And the tests below hold every app id, those on the framework and those to
//! be ported, clear of every entry: an app's settings, data, state and cache never land on a
//! component's file, except where an entry names that app as sharing it on purpose.
//!
//! An entry covers its own name and every name made from it ([`covers`]): the backups, the
//! markers and the temporaries a component keeps beside its file. From the audit, among others:
//! `deepseek.json.before-yantrik` (an API key), `pi.json.before-yantrik`,
//! `openclaw.env.before-yantrik` (a gateway token), `openclaw.json.before-yantrik`,
//! `companion-models.json.before-yantrik` and their `.absent` markers (harness `backup_once`,
//! shell `backup_path`); `notifications.notices.json`, `notifications.json.unreadable-<ts>`,
//! `notifications.tmp<pid>`, `notices.json.migrated-<ts>` and `notices-archive.<N>.jsonl`
//! (notifications-service); `mind-mode.json.tmp`, `mind-panel.json.new`, `free-ai.json.new`,
//! `.privacy.json.<pid>.<nanos>`, `taint-scopes.json.<pid>` and `last-brief.tmp` (shell).
//!
//! Kept from the audit of 21 Oct 2026 (the pull request that moved the settings to `apps/`). A
//! component that starts keeping a new file under one of the roots adds it here; a test reads
//! the services', the shell's, the harnesses' and the deploy scripts' sources for paths under
//! the four roots and fails on a name no entry covers.

/// Which of the four roots an entry is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    Config,
    Data,
    State,
    Cache,
}

/// One name under `<root>/yantrik/`, and whose it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub root: Root,
    /// The first path component under `<root>/yantrik/`: a file, or a directory.
    pub name: &'static str,
    pub owner: &'static str,
    /// The one app that may treat this entry as its own: its settings from before the
    /// framework, or its data folder shared with its service by design.
    pub app: Option<&'static str>,
}

const fn other(root: Root, name: &'static str, owner: &'static str) -> Entry {
    Entry { root, name, owner, app: None }
}

const fn apps(root: Root, name: &'static str, owner: &'static str, app: &'static str) -> Entry {
    Entry { root, name, owner, app: Some(app) }
}

use Root::{Cache, Config, Data, State};

pub const ENTRIES: &[Entry] = &[
    // config: secrets marked (S) in the audit table.
    other(Config, "email.json", "email-service (accounts, password and OAuth tokens in clear text) (S)"),
    other(Config, "google-oauth.json", "email-service (Google client secret) (S)"),
    other(Config, "providers.yaml", "shell, installer, ISO build (API keys) (S)"),
    other(Config, "deepseek.json", "shell handoff, DeepSeek harness (API key) (S)"),
    other(Config, "openclaw.env", "OpenClaw harness unit (S)"),
    other(Config, "deepseek.env", "DeepSeek harness unit (S)"),
    other(Config, "free-pool.env", "deploy/free-pool (S)"),
    other(Config, "pi.json", "shell handoff, pi harness"),
    other(Config, "openclaw.json", "shell handoff, OpenClaw harness"),
    other(Config, "settings.yaml", "shell; read by every service's gate"),
    other(Config, "theme.yaml", "shell (legacy, migrated into settings.yaml)"),
    other(Config, "theme-override.yaml", "shell"),
    other(Config, "mind-mode.json", "shell; read by the gate"),
    other(Config, "agent-reach.json", "shell; read by ipc-transport"),
    other(Config, "privacy.json", "shell; read by ipc-transport"),
    other(Config, "perception.yaml", "perception-service"),
    other(Config, "vault", "perception-service scope"),
    other(Config, "event_log.db", "shell"),
    other(Config, "skills.db", "shell, companion"),
    other(Config, "entity_graph.db", "shell"),
    other(Config, "app_state.db", "shell"),
    other(Config, "accounts.json", "shell"),
    other(Config, "ai-accounts.json", "shell, installer"),
    other(Config, "picker.json", "shell"),
    other(Config, "first-run.json", "shell, installer"),
    other(Config, "companion-models.json", "shell"),
    other(Config, "mind-panel.json", "shell"),
    other(Config, "free-ai.json", "shell"),
    other(Config, "memory-grants.json", "shell"),
    other(Config, "gateway", "shell, yantrik-gateway (token hashes)"),
    other(Config, "handoff", "shell provider handoff"),
    other(Config, "agents", "shell agent catalog"),
    other(Config, "plugins", "companion-tools"),
    other(Config, "scripts", "companion-tools coder"),
    other(Config, "browser", "companion-tools browser profile (S)"),
    other(Config, "memory.db", "memory database, protected by ipc-contracts home_paths"),
    // config: an app's own settings from before the framework, its to bring across.
    apps(Config, "weather.json", "Weather, before its own directory", "weather"),
    apps(Config, "music.json", "Music, before the framework", "music"),
    apps(Config, "studio.json", "Studio, before the framework", "studio"),
    // data
    apps(Data, "notes", "notes-service vault, which Notes shows and edits", "notes"),
    apps(Data, "calendar", "calendar-service events, read by notifications-service", "calendar"),
    other(Data, "notifications.json", "notifications-service"),
    other(Data, "notices.json", "notifications-service (legacy)"),
    other(Data, "notices-archive.jsonl", "notifications-service"),
    other(Data, "agents", "shell agent transcripts"),
    other(Data, "runs.db", "shell harness run store"),
    other(Data, "taint-scopes.json", "shell"),
    other(Data, "mind-audit.jsonl", "shell"),
    other(Data, "last-brief", "shell"),
    other(Data, "mind-view-browser", "shell (a browser profile) (S)"),
    other(Data, "trash-v2", "shell fileops, Studio"),
    other(Data, "accounts", "shell (vendor CLI sign-ins) (S)"),
    other(Data, "browsers", "companion-tools browser (cookies, logins) (S)"),
    other(Data, "problems", "yantrik-foundation problem records"),
    other(Data, "memory", "harness memory databases"),
    other(Data, "minds", "harness conversations"),
    // state
    other(State, "perception-journal", "perception-journal"),
    other(State, "gateway-calls.jsonl", "shell gateway"),
    other(State, "harnesses", "harness installers, shell"),
    other(State, "default-mind.json", "shell"),
    other(State, "update-notified", "shell"),
    other(State, "quarantine", "yantrik-ml private_dir (antivirus)"),
    other(State, "provider-pages", "yantrik-ml private_dir (a browser profile) (S)"),
    other(State, "reports", "yantrik-ml private_dir"),
    other(State, "yantrik-os.log", "deploy yantrik-session"),
    // cache
    other(Cache, "tmp", "yantrik-ml private_dir"),
    other(Cache, "work", "foot scrollback"),
    other(Cache, "lock-wallpaper.png", "shell, yantrik-lock"),
    other(Cache, "lock-wallpaper.source", "shell, yantrik-lock"),
    other(Cache, "model-catalogue.json", "shell"),
    other(Cache, "binaries", "dev scripts"),
    other(Cache, "build-cache", "dev scripts"),
];

/// Whether entry `e` covers `name`: its own name, or one made from it by what follows a dot
/// (`<entry>.before-yantrik`, `<entry>.tmp`, `.<entry>.<pid>`), or from its stem
/// (`notifications.json` covers `notifications.tmp<pid>` and `notifications.notices.json`).
pub fn covers(e: &Entry, name: &str) -> bool {
    let name = name.trim_start_matches('.').trim_end_matches('.');
    let made_from = |base: &str| name == base || name.strip_prefix(base).is_some_and(|rest| rest.starts_with('.'));
    let stem = e.name.rsplit_once('.').map_or(e.name, |(stem, _)| stem);
    made_from(e.name) || (!stem.is_empty() && made_from(stem))
}

/// Whether `name` under `<root>/yantrik/` is kept by a component other than app `app`.
pub fn is_others(root: Root, name: &str, app: &str) -> bool {
    ENTRIES
        .iter()
        .any(|e| e.root == root && e.app != Some(app) && covers(e, name))
}

#[cfg(test)]
#[path = "others_tests.rs"]
mod tests;
