//! Accounts — the subscriptions and keys a person brings to this desktop, and what each has left.
//!
//! A person with a Claude Max plan, a ChatGPT Pro plan for Codex and a Qwen sign-in has three
//! minds' worth of thinking paid for, each metered in its own windows. This module is what the
//! Minds panel (`components/minds_panel.slint`, opened from the mind chip) shows of them: which
//! accounts are signed in, which one each vendor answers with, the plan's own meters where the
//! vendor reports them, and today's tokens everywhere.
//!
//! # The person signs in, with the vendor
//!
//! Every sign-in is the vendor's own flow, in the vendor's own program, in a terminal the person
//! watches (`act::sign_in`). The desktop never receives, copies or holds a credential; the
//! program keeps it where it always does. A second account for the same vendor is a second
//! directory the program is pointed at with its own variable, and "Use" points new programs at
//! it (`act::use_account`).
//!
//! # What is read
//!
//! | vendor | signed in? | plan | meters | today |
//! |---|---|---|---|---|
//! | Claude | `.credentials.json` exists (never opened) | two fields of `.claude.json` (`claude`) | — | its transcripts |
//! | Codex | `auth.json` exists (never opened) | its session logs (`codex`) | its session logs | its session logs |
//! | Gemini | `oauth_creds.json` exists (never opened) | — | — | — |
//! | Qwen, xAI | the desktop's AI provider is theirs | — | — | — |
//!
//! No sign-in file is ever opened, and no vendor is called with a person's token: Anthropic,
//! OpenAI and Google each say another program must not collect or use their sign-ins, and a
//! meter read that way would be one. What is not known is not drawn — a meter the vendor's own
//! program does not write down is not estimated. Claude's plan windows come next, from Claude
//! Code's own status line (`rate_limits`), which is the vendor's sanctioned place for them.
//!
//! # Shape
//!
//! [`observe`] gathers facts (files, `PATH`, the counters); [`rows`] is a pure function of them,
//! and is what the tests drive. `wire/minds_panel.rs` runs [`observe`] on a worker while the
//! panel is open and hands the rows to Slint.

pub mod act;
pub mod claude;
pub mod codex;
pub mod logs;
pub mod store;
pub mod vendors;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use store::{Store, PRIMARY};
use vendors::{SignIn, Vendor, VENDORS};

/// One meter of one account.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meter {
    pub name: String,
    /// The window's length, for ordering; 0 for a count.
    pub window_minutes: u64,
    /// How much of the window is used, 0..1, when the vendor says. `None`: a count, no bar.
    pub used: Option<f32>,
    pub value: String,
}

impl Meter {
    pub fn window(name: String, window_minutes: u64, used: f32, value: String) -> Meter {
        Meter { name, window_minutes, used: Some(used), value }
    }

    pub fn count(name: &str, value: String) -> Meter {
        Meter { name: name.into(), window_minutes: 0, used: None, value }
    }
}

/// What an account's row says about it, and what its button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The one its vendor answers with.
    Active,
    /// Signed in; "Use" makes it the active one.
    Ready,
    /// Its program is here and it is not signed in.
    SignIn,
    /// Its program is not installed.
    Missing,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Ready => "ready",
            State::SignIn => "signin",
            State::Missing => "missing",
        }
    }
}

/// One account, as the panel draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// `vendor:label`.
    pub id: String,
    pub label: String,
    pub plan: String,
    pub state: State,
    pub note: String,
    pub meters: Vec<Meter>,
    pub tokens_today: u64,
}

/// One vendor and its accounts.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub vendor: &'static Vendor,
    pub rows: Vec<Row>,
}

/// Something the "+" can offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub vendor: &'static Vendor,
    /// "Sign in", "Add account", "Install", "Set up a key".
    pub what: &'static str,
}

/// What one account's logs and files said.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    pub signed_in: bool,
    pub plan: Option<String>,
    pub meters: Vec<Meter>,
    pub tokens_today: u64,
}

/// Every fact the rows are made from.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub store: Store,
    /// Vendor ids whose program was found.
    pub installed: Vec<&'static str>,
    /// Account id → what was seen.
    pub seen: HashMap<String, Seen>,
    /// The desktop's own AI endpoint, for a key-only vendor.
    pub companion_url: Option<String>,
}

/// The id of `label` at `vendor`.
pub fn account_id(vendor: &str, label: &str) -> String {
    format!("{vendor}:{label}")
}

/// `claude:account-2` → (vendor, label), both as the desktop writes them.
pub fn parse_id(id: &str) -> Option<(&'static Vendor, &str)> {
    let (v, l) = id.split_once(':')?;
    let vendor = vendors::by_id(v)?;
    store::label_ok(l).then_some((vendor, l))
}

/// How a label reads: `primary` → `Main`, `account-2` → `Account 2`.
pub fn label_name(label: &str) -> String {
    match label.strip_prefix("account-") {
        Some(n) => format!("Account {n}"),
        None => "Main".into(),
    }
}

/// The panel's rows: each vendor that is here in any way, and its accounts.
pub fn rows(f: &Facts) -> Vec<Group> {
    let mut out = Vec::new();
    for vendor in VENDORS.iter() {
        if matches!(vendor.sign_in, SignIn::Key { .. }) {
            if vendor.serves(f.companion_url.as_deref()) {
                out.push(Group {
                    vendor,
                    rows: vec![Row {
                        id: account_id(vendor.id, PRIMARY),
                        label: "API key".into(),
                        plan: "the companion's provider".into(),
                        state: State::Active,
                        note: String::new(),
                        meters: Vec::new(),
                        tokens_today: 0,
                    }],
                });
            }
            continue;
        }
        let installed = f.installed.contains(&vendor.id);
        let labels = f.store.labels(vendor.id);
        let active = f.store.active(vendor.id);
        let mut rows_here = Vec::new();
        for label in &labels {
            let id = account_id(vendor.id, label);
            let seen = f.seen.get(&id).cloned().unwrap_or_default();
            // A vendor that is neither installed nor signed in is only in the "+" list.
            if !installed && !seen.signed_in {
                continue;
            }
            let state = if !installed {
                State::Missing
            } else if !seen.signed_in {
                State::SignIn
            } else if label == active {
                State::Active
            } else {
                State::Ready
            };
            let mut meters = seen.meters.clone();
            if seen.signed_in {
                meters.push(Meter::count("Today", format!("{} tokens", tokens(seen.tokens_today))));
            }
            let note = match state {
                State::Missing => format!("{} is not installed here", vendor.name),
                State::SignIn if vendor.many() && label != PRIMARY => "Not signed in yet".into(),
                _ => used_up(&meters).unwrap_or_default(),
            };
            rows_here.push(Row {
                id,
                label: label_name(label),
                plan: seen.plan.unwrap_or_default(),
                state,
                note,
                meters,
                tokens_today: seen.tokens_today,
            });
        }
        if !rows_here.is_empty() {
            out.push(Group { vendor, rows: rows_here });
        }
    }
    out
}

/// "Weekly is used up" when a window is full.
fn used_up(meters: &[Meter]) -> Option<String> {
    meters
        .iter()
        .find(|m| m.used.is_some_and(|u| u >= 0.999))
        .map(|m| format!("{} is used up · back in {}", m.name, m.value))
}

/// What the "+" offers: every vendor, with the one thing that would bring it here.
pub fn choices(f: &Facts) -> Vec<Choice> {
    VENDORS
        .iter()
        .filter_map(|vendor| {
            let what = match vendor.sign_in {
                SignIn::Key { .. } if vendor.serves(f.companion_url.as_deref()) => return None,
                SignIn::Key { .. } => "Set up a key",
                SignIn::Program { .. } if !f.installed.contains(&vendor.id) => "Install",
                SignIn::Program { .. } => {
                    let signed_in = f.seen.get(&account_id(vendor.id, PRIMARY)).is_some_and(|s| s.signed_in);
                    if !signed_in {
                        "Sign in"
                    } else if vendor.many() && f.store.labels(vendor.id).len() < store::MOST_PER_VENDOR {
                        "Add account"
                    } else {
                        return None;
                    }
                }
            };
            Some(Choice { vendor, what })
        })
        .collect()
}

/// The header: "2.4M tokens today", or nothing when nothing was counted.
pub fn today_line(groups: &[Group]) -> String {
    let total: u64 = groups.iter().flat_map(|g| &g.rows).map(|r| r.tokens_today).sum();
    if total == 0 {
        String::new()
    } else {
        format!("{} tokens today", tokens(total))
    }
}

/// 950 → `950`, 12_400 → `12.4K`, 2_400_000 → `2.4M`.
pub fn tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => trim(format!("{:.1}K", n as f64 / 1e3)),
        1_000_000..=999_999_999 => trim(format!("{:.1}M", n as f64 / 1e6)),
        _ => trim(format!("{:.1}B", n as f64 / 1e9)),
    }
}

fn trim(s: String) -> String {
    s.replace(".0K", "K").replace(".0M", "M").replace(".0B", "B")
}

/// Time left, the way the vendors' own pages write it: `12m`, `4h 42m`, `4d 12h`.
pub fn left(secs: i64) -> String {
    let m = (secs.max(0) + 59) / 60;
    match m {
        0..=59 => format!("{m}m"),
        60..=1439 => format!("{}h {}m", m / 60, m % 60),
        _ => format!("{}d {}h", m / 1440, (m % 1440) / 60),
    }
}

/// Look for the vendors' programs on `path` and in the places a per-person npm puts them, which
/// a service's `PATH` does not have.
pub fn installed(home: &Path, path: Option<&std::ffi::OsStr>) -> Vec<&'static str> {
    let mut dirs: Vec<PathBuf> = path.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    for extra in [".npm-global/bin", ".local/bin", ".local/node/bin", ".bun/bin"] {
        dirs.push(home.join(extra));
    }
    VENDORS
        .iter()
        .filter(|v| !v.binary.is_empty() && dirs.iter().any(|d| is_program(&d.join(v.binary))))
        .map(|v| v.id)
        .collect()
}

fn is_program(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// The counters, kept between reads so each log is read from where it stopped.
#[derive(Default)]
pub struct Counters {
    claude: HashMap<PathBuf, claude::Counter>,
    codex: HashMap<PathBuf, codex::Counter>,
    plans: claude::PlanCache,
}

/// Gather every fact: files, programs, logs. Blocking; run it off the UI thread.
pub fn observe(home: &Path, counters: &mut Counters, companion_url: Option<String>, now: i64, midnight: i64) -> Facts {
    let store = Store::load(&store::path_in(home));
    let installed = installed(home, std::env::var_os("PATH").as_deref());
    let mut seen = HashMap::new();
    // One budget for every log this read looks at; what it does not reach is read next time.
    let mut budget = logs::Budget::tick();
    for vendor in VENDORS.iter().filter(|v| matches!(v.sign_in, SignIn::Program { .. })) {
        for label in store.labels(vendor.id) {
            let dir = store::dir_of(home, vendor, &label);
            let signed_in = std::fs::symlink_metadata(dir.join(vendor.marker)).is_ok_and(|m| m.is_file());
            let mut s = Seen { signed_in, ..Seen::default() };
            if signed_in {
                match vendor.id {
                    "claude" => {
                        s.plan = claude::plan(&dir, label == PRIMARY, &mut counters.plans);
                        let c = counters.claude.entry(dir.clone()).or_default();
                        c.update(&dir, midnight, &mut budget);
                        s.tokens_today = c.tokens_today();
                    }
                    "codex" => {
                        let c = counters.codex.entry(dir.clone()).or_default();
                        c.update(&dir, midnight, &mut budget);
                        let got = c.seen(now);
                        s.plan = got.plan;
                        s.meters = got.meters;
                        s.tokens_today = got.tokens_today;
                    }
                    _ => {}
                }
            }
            seen.insert(account_id(vendor.id, &label), s);
        }
    }
    Facts { store, installed, seen, companion_url }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        let mut store = Store::default();
        store.add(vendors::by_id("claude").unwrap()).unwrap();
        store.use_label("claude", "account-2").unwrap();
        let mut seen = HashMap::new();
        seen.insert("claude:primary".into(), Seen { signed_in: true, plan: Some("Max 20x".into()), tokens_today: 1_200_000, ..Seen::default() });
        seen.insert("claude:account-2".into(), Seen { signed_in: true, plan: Some("Max 20x".into()), tokens_today: 1_200_000, ..Seen::default() });
        seen.insert(
            "codex:primary".into(),
            Seen {
                signed_in: true,
                plan: Some("Pro".into()),
                meters: vec![Meter::window("Weekly".into(), 10080, 1.0, "4d 8h".into())],
                tokens_today: 0,
            },
        );
        Facts { store, installed: vec!["claude", "codex", "gemini"], seen, companion_url: Some("https://api.x.ai/v1".into()) }
    }

    #[test]
    fn the_rows_say_which_account_answers_and_what_is_left() {
        let g = rows(&facts());
        let names: Vec<_> = g.iter().map(|g| g.vendor.id).collect();
        assert_eq!(names, ["claude", "codex", "gemini", "xai"], "qwen's key is not the desktop's provider");
        let claude = &g[0].rows;
        assert_eq!(claude[0].label, "Main");
        assert_eq!(claude[0].state, State::Ready);
        assert_eq!(claude[1].label, "Account 2");
        assert_eq!(claude[1].state, State::Active);
        assert_eq!(claude[1].meters.last().unwrap().value, "1.2M tokens");
        let codex = &g[1].rows[0];
        assert_eq!(codex.state, State::Active);
        assert_eq!(codex.note, "Weekly is used up · back in 4d 8h");
        assert_eq!(g[2].rows[0].state, State::SignIn, "gemini is installed and not signed in");
        assert_eq!(g[3].rows[0].state, State::Active);
        assert_eq!(today_line(&g), "2.4M tokens today");
    }

    #[test]
    fn the_plus_offers_what_would_bring_each_vendor_here() {
        let c = choices(&facts());
        let got: Vec<_> = c.iter().map(|c| (c.vendor.id, c.what)).collect();
        assert_eq!(got, [("claude", "Add account"), ("codex", "Add account"), ("qwen", "Set up a key"), ("gemini", "Sign in")]);
        let mut f = facts();
        f.companion_url = None;
        assert!(choices(&f).iter().any(|c| c.vendor.id == "xai" && c.what == "Set up a key"));
    }

    #[test]
    fn a_signed_in_account_whose_program_is_gone_says_so() {
        let mut f = facts();
        f.installed = vec![];
        let g = rows(&f);
        assert_eq!(g[0].rows[0].state, State::Missing);
        assert_eq!(g[0].rows[0].note, "Claude is not installed here");
    }

    #[test]
    fn ids_parse_only_as_the_desktop_writes_them() {
        assert_eq!(parse_id("claude:account-2").map(|(v, l)| (v.id, l)), Some(("claude", "account-2")));
        for bad in ["claude", "claude:../x", "nobody:primary", "claude:primary:x"] {
            assert!(parse_id(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn numbers_read_the_way_people_write_them() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(12_400), "12.4K");
        assert_eq!(tokens(2_000_000), "2M");
        assert_eq!(tokens(2_400_000), "2.4M");
        assert_eq!(left(30), "1m");
        assert_eq!(left(4 * 3600 + 42 * 60), "4h 42m");
        assert_eq!(left(4 * 86_400 + 12 * 3600), "4d 12h");
    }

    #[cfg(unix)]
    #[test]
    fn programs_are_found_where_a_person_npm_puts_them() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("yantrik-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".npm-global/bin")).unwrap();
        let p = home.join(".npm-global/bin/codex");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(home.join(".npm-global/bin/claude"), "not a program").unwrap();
        assert_eq!(installed(&home, None), ["codex"]);
    }
}
