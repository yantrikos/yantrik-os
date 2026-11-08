//! A Mind's sealed workshop, on the shell's surface (issue #1120): `workshop_create`,
//! `workshop_revoke`, `workshop_list` and `agent_run_sealed`.
//!
//! # Why `agent_run_sealed` is its own action
//!
//! The gate that asks (the shell's `gate::decide`, the bridge's `decide`, and the taint) reads an
//! action's NAME, grade and published sentence, never an argument: an argument is the caller's
//! word. `agent_run` stays `sensitive` and open-ended, and asks as it always did, in a tainted
//! conversation too. `agent_run_sealed` is a different action that says in its sentence that it
//! cannot reach the network or the person's files, and it is `standard`, so no card is raised for
//! it. That is only true because the action itself refuses to run anything that is not sealed:
//! the sandbox is built here, from the shell's own grant record ([`crate::workshop`]), after the
//! caller's token has named the Mind. A workshop that is not that Mind's grant, a `cwd` outside
//! it, a missing `bwrap` or `systemd-run`: nothing runs, and nothing falls back to `agent_run`.
//! `taint_ask` and `yos-mcp` hold the same line from their side (their tests name this action).
//!
//! # The grant
//!
//! `workshop_create` is `sensitive`, declares `always_asks` and explains each call on the card
//! ("Give <the Mind> a workshop at ~/Documents/x"). That declaration is the shell's way of forcing
//! a card whatever the mode: `gate::decide` refuses the call without a grant in every mode (full
//! bypass included, no session or standing rule answering), `request_approval` asks whatever its
//! table says (`taint_ask::resolve`, the path the taint already uses to ask in every mode), and
//! the card offers no "Allow for this session". The grant a person allows is bound to the folder
//! as typed. A Mind never answers it (#1124, H1).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};
use yantrik_agent_terminal::Sealed;

use crate::control_agent_terminal::{calling_agent, run_job, text, wait_arg, Call, Prepared};
use crate::workshop;

/// The sentence the card shows for one `workshop_create`. Empty folders say what is wrong instead.
fn explain_grant(args: &Value) -> String {
    explain_in(&workshop::home(), args)
}

fn explain_in(home: &Path, args: &Value) -> String {
    let raw = text(args, "folder");
    match workshop::check_folder(home, &raw, false) {
        // Not made yet: the rules are run as a grant would, without making it.
        Err(e) if e.contains("does not exist") => format!("Give the Mind asking a workshop at {}.", raw.trim()),
        Err(e) => format!("Give the Mind asking a workshop at {}. This will be refused: {e}", raw.trim()),
        Ok(real) => format!("Give the Mind asking a workshop at {}.", real.display()),
    }
}

/// The card's sentence names the Mind that asked. The action cannot know who that is while it is
/// explaining (it is asked over a socket, with the arguments and nothing else), so the shell's
/// `request_approval` puts the asker it verified in place of the words "the Mind asking" (#1124, L4).
/// `who` is the token-vouched agent id or the verified program, cleaned of anything that draws
/// differently from how it reads.
pub(crate) fn name_the_mind(app: &str, action: &str, explained: &str, who: &str) -> String {
    if (app, action) != ("shell", "workshop_create") {
        return explained.to_string();
    }
    let clean: String = who
        .chars()
        .filter(|c| !c.is_control() && !crate::approvals::is_format_char(*c))
        .take(60)
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return explained.to_string();
    }
    explained.replacen("the Mind asking", clean, 1)
}

pub(crate) fn specs() -> [Action; 4] {
    [
        Action::new(
            "workshop_create",
            "Ask the person for a workshop: one new folder under ~/Documents where you may build \
             with `agent_run_sealed`, sealed away from the network and the rest of the machine. \
             The person decides on a card. Acts for the agent named by the agent token your call \
             carries — never an argument.",
        )
        .risk("sensitive")
        // The person allows each grant on a card, in every mode, and no session rule covers it
        // (#1124 H1): `gate::decide` refuses a call without a grant and `request_approval` asks
        // whatever its table says.
        .always_asks()
        .arg(Param::text("folder").describe(
            "A new folder under ~/Documents, written ~/Documents/<name>. Never the home folder, \
             Desktop or Downloads, never through a symlink, never with a dot name. The shell makes it",
        ))
        .explain(explain_grant),
        Action::new(
            "workshop_revoke",
            "Take a workshop back: delete the grant. A Mind revokes its own; the person names the \
             Mind in `agent`. The folder and what is in it stay.",
        )
        .arg(Param::text("agent").optional().describe("The Mind's agent id. Left out by a Mind: its own")),
        Action::new("workshop_list", "The workshops granted: a Mind sees its own, the person sees all.").risk("safe"),
        // No sentence here may say what makes `agent_run` ask (the OPEN_ENDED, LEAVES_MACHINE or
        // fetches-an-address sentences): a test holds that, because this action is graded `standard`
        // and asks nothing, on the strength of what the code below does.
        Action::new(
            "agent_run_sealed",
            "Run one command line in your workshop, sealed: no network, no session bus, no screen, \
             none of the person's files but the workshop, a scrubbed environment (HOME is the \
             workshop), a temporary /tmp, and caps on CPU, memory and time. Output is shown live in \
             Mind View. Needs a workshop the person granted you (`workshop_create`). Answers when it \
             exits, with `exit_code` and the `tail` of its output; if it is still going after \
             `wait` seconds it answers `running: true` with a `job` id for `agent_job`. Acts for \
             the agent named by the agent token your call carries — never an argument.",
        )
        .defers()
        .arg(Param::text("command").describe("One command line, as it would be typed. It runs under bash"))
        .arg(Param::text("workshop").describe("Your workshop's folder, exactly as granted"))
        .arg(Param::text("cwd").optional().describe("Where to run it: inside the workshop, relative to it or absolute. Left out: the workshop"))
        .arg(Param::number("wait").optional().describe(
            "A number of seconds, e.g. 30. How long to wait before answering `running: true`. Default 120, at most 600",
        )),
    ]
}

pub fn actions(surface: ControlSurface) -> ControlSurface {
    let [grant, revoke, list, sealed] = specs();
    surface
        .action(grant, |args| workshop_create(args, &workshop::home(), &workshop::record_path()))
        .action(revoke, |args| {
            let agent = mind()?;
            workshop_revoke(args, &workshop::record_path(), agent, the_person_is_calling())
        })
        .action(list, |_| workshop_list(&workshop::record_path()))
        .action(sealed, agent_run_sealed)
}

/// The Mind the token names, or why the call is not a Mind's.
fn mind() -> Result<Option<String>, String> {
    match calling_agent() {
        None => Ok(None),
        Some(Ok(agent)) => Ok(Some(agent.0)),
        Some(Err(why)) => Err(why),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn workshop_create(args: &Value, home: &Path, record: &Path) -> Result<Value, String> {
    let agent = mind()?.ok_or("only a Mind asks for a workshop, and the person answers on the card: this call carries no agent token.")?;
    let folder = workshop::check_folder(home, &text(args, "folder"), true)?;
    let made = workshop::grant(record, &agent, &folder, now())?;
    Ok(json!({
        "granted": true,
        "workshop": made.folder,
        "next": "run commands in it with `agent_run_sealed` (workshop = this folder). \
                 They have no network. The person can revoke it any time.",
    }))
}

/// Is the caller the person's own process, as the shell classes it? A program it cannot place is
/// not (`caller_class` answers `mind` for anything not shown to be the person's).
fn the_person_is_calling() -> bool {
    yantrik_app_runtime::control::caller().is_some_and(|c| crate::caller_class::class_of(c.pid as i64) == "person")
}

/// A Mind revokes its own grant. The person names the Mind. A caller with no token that the shell
/// cannot show to be the person has no say in anyone's grant (#1124, L5).
fn workshop_revoke(args: &Value, record: &Path, mind: Option<String>, person: bool) -> Result<Value, String> {
    let agent = match mind {
        Some(own) => own,
        None if person => text(args, "agent").trim().to_string(),
        None => return Err("only the person, or a Mind for its own workshop, takes a workshop back.".into()),
    };
    if agent.is_empty() {
        return Err("`agent` names the Mind whose workshop is taken back.".into());
    }
    let had = workshop::revoke(record, &agent)?;
    Ok(json!({ "revoked": had, "agent": agent }))
}

fn workshop_list(record: &Path) -> Result<Value, String> {
    // Grants whose folder is gone or replaced are dropped as the record is read.
    let all = workshop::prune(record)?;
    let own = mind()?;
    let shown: Vec<Value> = all
        .into_iter()
        .filter(|g| own.as_ref().map_or(true, |a| *a == g.agent))
        .map(|g| json!({ "agent": g.agent, "folder": g.folder, "granted_unix": g.granted_unix }))
        .collect();
    Ok(json!({ "workshops": shown }))
}

/// What a sealed command needs settled before it starts, from the grant and nothing else.
pub(crate) fn prepare(home: &Path, record: &Path, agent: &str, named: &str, cwd: Option<&Path>) -> Result<Prepared, String> {
    let shop: PathBuf = workshop::granted_for(record, home, agent, named)?;
    let cwd = workshop::cwd_inside(&shop, cwd)?;
    Ok(Prepared { cwd: Some(cwd), sealed: Some(Sealed::new(shop)) })
}

fn agent_run_sealed(args: &Value) -> Result<Value, String> {
    let command = text(args, "command");
    let named = text(args, "workshop");
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty()).map(PathBuf::from);
    let wait = wait_arg(args)?;
    run_job(
        command,
        wait,
        Call::current(),
        Box::new(move |agent| {
            let record = workshop::record_path();
            let ready = prepare(&workshop::home(), &record, &agent.0, &named, cwd.as_deref())?;
            Ok(ready)
        }),
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use yantrik_ipc_transport::gate::{fetches_address, leaves_machine, open_ended};

    fn grant_for(home: &Path, rec: &Path, agent: &str, folder: &str) -> PathBuf {
        let real = workshop::check_folder(home, folder, true).unwrap();
        workshop::grant(rec, agent, &real, 1).unwrap();
        real
    }

    fn home() -> tempfile::TempDir {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join("Documents")).unwrap();
        h
    }

    #[test]
    fn the_sealed_action_asks_nothing_only_because_it_is_not_what_agent_run_is() {
        let [grant, _, list, sealed] = specs();
        assert_eq!(sealed.permission, "standard");
        for a in [&sealed, &list, &grant] {
            assert!(!open_ended(&a.description) && !leaves_machine(&a.description), "{}", a.name);
            assert!(fetches_address(&a.description).is_none(), "{}", a.name);
        }
        // So it is not a sink: a tainted conversation is not asked about it, and `agent_run` still is.
        assert!(!crate::taint_ask::is_sink("shell", "agent_run_sealed", &sealed.description));
        let run = crate::control_agent_terminal::specs().into_iter().find(|a| a.name == "agent_run").unwrap();
        assert!(crate::taint_ask::is_sink("shell", "agent_run", &run.description));
        // The grant is asked about, in the ordinary way.
        assert_eq!(grant.permission, "sensitive");
    }

    #[test]
    fn a_command_is_sealed_only_by_the_grant_record() {
        let h = home();
        let rec = h.path().join("workshops.json");
        let shop = grant_for(h.path(), &rec, "hermes", "~/Documents/shop");
        let ok = prepare(h.path(), &rec, "hermes", shop.to_str().unwrap(), None).unwrap();
        assert_eq!(ok.cwd.as_deref(), Some(shop.as_path()));
        assert_eq!(ok.sealed.unwrap().workshop, shop);
        // Not granted: another Mind, an un-granted folder, a symlink to the granted one, `..`.
        let other = workshop::check_folder(h.path(), "~/Documents/other", true).unwrap();
        std::os::unix::fs::symlink(&shop, h.path().join("Documents/alias")).unwrap();
        for (agent, named) in [
            ("pi", shop.to_str().unwrap().to_string()),
            ("hermes", other.display().to_string()),
            ("hermes", h.path().join("Documents/alias").display().to_string()),
            ("hermes", format!("{}/..", shop.display())),
            ("hermes", h.path().display().to_string()),
            ("hermes", String::new()),
        ] {
            assert!(prepare(h.path(), &rec, agent, &named, None).is_err(), "{agent} {named:?}");
        }
        // A cwd that leaves the workshop.
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), shop.join("out")).unwrap();
        for cwd in ["..", "out", "/", "../other"] {
            assert!(prepare(h.path(), &rec, "hermes", shop.to_str().unwrap(), Some(Path::new(cwd))).is_err(), "{cwd}");
        }
        // Revoked: nothing is sealed any more, and nothing falls back.
        workshop::revoke(&rec, "hermes").unwrap();
        assert!(prepare(h.path(), &rec, "hermes", shop.to_str().unwrap(), None).is_err());
    }

    #[test]
    fn the_card_says_what_it_grants() {
        let h = home();
        let said = explain_in(h.path(), &json!({ "folder": "~/Desktop/x" }));
        assert!(said.contains("refused"), "{said}");
        assert!(explain_in(h.path(), &json!({ "folder": "~/Documents/new" })).starts_with("Give the Mind asking a workshop at ~/Documents/new"));
    }

    /// #1124 L4 and N7: the card says which Mind asks, from the verified asker, with nothing invisible in it.
    /// The sentence it names it in is the registry's own, which `published_detail` now hands the card for
    /// the shell's surface (`published_explanation`).
    #[test]
    fn the_card_names_the_mind_that_asked() {
        let h = home();
        let said = explain_in(h.path(), &json!({ "folder": "~/Documents/new" }));
        let named = name_the_mind("shell", "workshop_create", &said, "hermes:main");
        assert!(named.starts_with("Give hermes:main a workshop at ~/Documents/new"), "{named}");
        let tricked = name_the_mind("shell", "workshop_create", &said, &format!("her{}mes{}:main", char::from_u32(0x202E).unwrap(), char::from_u32(10).unwrap()));
        assert!(tricked.starts_with("Give hermes:main a workshop"), "{tricked}");
        // Another action's sentence is left alone, and an empty asker leaves the words as they were.
        assert_eq!(name_the_mind("shell", "agent_run", &said, "x"), said);
        assert_eq!(name_the_mind("shell", "workshop_create", &said, "  "), said);
    }

    /// #1124 L5: with no token, only the person may take a Mind's workshop back.
    #[test]
    fn only_the_person_or_the_mind_itself_revokes() {
        let h = home();
        let rec = h.path().join("workshops.json");
        grant_for(h.path(), &rec, "hermes", "~/Documents/shop");
        let named = json!({ "agent": "hermes" });
        assert!(workshop_revoke(&named, &rec, None, false).unwrap_err().contains("only the person"));
        assert_eq!(workshop::load(&rec).unwrap().len(), 1, "an unplaced caller revoked nothing");
        // Another Mind's own call revokes its own, whatever it names.
        assert_eq!(workshop_revoke(&named, &rec, Some("pi".into()), false).unwrap()["revoked"], false);
        assert_eq!(workshop::load(&rec).unwrap().len(), 1);
        assert_eq!(workshop_revoke(&named, &rec, Some("hermes".into()), false).unwrap()["revoked"], true);
        grant_for(h.path(), &rec, "hermes", "~/Documents/shop2");
        assert_eq!(workshop_revoke(&named, &rec, None, true).unwrap()["revoked"], true);
    }

    /// Only with `bwrap` and a user systemd to hold the caps: the sandbox itself, run for real.
    #[test]
    fn a_sealed_command_cannot_reach_the_network_or_the_homes_config() {
        let have = |t: &str| ["/usr/bin", "/bin"].iter().any(|d| Path::new(&format!("{d}/{t}")).exists());
        let bus = std::env::var_os("XDG_RUNTIME_DIR").is_some();
        if !have("bwrap") || !have("systemd-run") || !bus {
            eprintln!("skipped: needs bwrap, systemd-run and a user manager");
            return;
        }
        let h = home();
        let secret_dir = h.path().join(".config");
        std::fs::create_dir(&secret_dir).unwrap();
        std::fs::write(secret_dir.join("token"), "do-not-read").unwrap();
        let shop = workshop::check_folder(h.path(), "~/Documents/shop", true).unwrap();
        let s = Sealed::new(shop.clone());
        let env: Vec<(String, String)> = vec![("USER".into(), "p".into()), ("LANG".into(), "C.UTF-8".into()), ("TERM".into(), "xterm".into())];
        let mut argv = vec![["/usr/bin/bwrap", "/bin/bwrap"].into_iter().find(|p| Path::new(p).exists()).unwrap().to_string()];
        argv.extend(s.bwrap_args(Path::new("/"), &shop, &env).unwrap());
        let probe = format!(
            "cat {}/token 2>&1; echo \"home=$HOME\"; echo \"bus=${{DBUS_SESSION_BUS_ADDRESS-unset}}\"; \
             ls /run/user 2>&1 | head -1; echo hi > {}/made; \
             (exec 3<>/dev/tcp/1.1.1.1/53) 2>&1; echo \"net=$?\"",
            secret_dir.display(),
            shop.display()
        );
        argv.extend(["/bin/bash".into(), "-c".into(), probe]);
        // Not a session leader here, as in the shell, where a bash stands in front of it.
        let out = std::process::Command::new("/bin/bash")
            .args(["-c", "\"$@\"; exit $?", "bash"])
            .args(&argv)
            .env_clear()
            .env("XDG_RUNTIME_DIR", std::env::var("XDG_RUNTIME_DIR").unwrap())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(!text.contains("do-not-read"), "the home config was readable: {text}");
        assert!(text.contains(&format!("home={}", shop.display())), "{text}");
        assert!(text.contains("bus=unset"), "{text}");
        assert!(!text.contains("net=0"), "the network was reachable: {text}");
        assert!(shop.join("made").exists(), "the workshop must be writable: {text}");
    }
}
