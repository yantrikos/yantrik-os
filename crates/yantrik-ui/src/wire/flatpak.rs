//! Flatpak apps from Flathub, for the Package Manager screen (#399).
//!
//! Debian's archive is where this OS gets its base system, and it does not carry the apps
//! people ask for first on a new machine: VS Code, Spotify, Discord, Steam. Flathub does, and
//! Flatpak is how every mainstream desktop now installs them. Without this the Package Manager
//! could offer none of them, and there was no other way in from the UI.
//!
//! Per-user, always: every command here runs `flatpak --user`, which installs into
//! `~/.local/share/flatpak` and needs no root. The desktop account has no blanket sudo, and its
//! narrow rule covers only the updater, the apt helper and the timezone (#397). A system-wide
//! installation would need either a new sudo rule or a wider root helper, and both are the
//! kind of door that rule was written to close. An app installed this way is also confined by
//! Flatpak's own sandbox, which a Debian package is not.
//!
//! The same split as `wire::apt`: the functions that RUN things are thin, and the functions that
//! PARSE their output or BUILD a command are pure and tested. Every app id is checked with
//! [`is_app_id`] before it reaches a command line, and nothing here goes through a shell.

use std::process::Command;

/// The remote every command here names. Per-user, added the first time it is needed.
pub const REMOTE: &str = "flathub";

/// Where the Flathub remote's description lives. Flathub's own published address.
pub const REMOTE_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";

/// The longest app id Flatpak itself accepts.
const MAX_ID_LEN: usize = 255;

/// One Flatpak app, installed or found on Flathub.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct App {
    /// The reverse-DNS id, `com.visualstudio.code`. What every command takes.
    pub id: String,
    /// What the app calls itself, `Visual Studio Code`. What the list shows.
    pub name: String,
    pub version: String,
    pub branch: String,
    pub description: String,
    /// Installed size, as flatpak prints it. Empty for a search result.
    pub size_text: String,
    pub installed: bool,
    pub upgradable: bool,
}

// ── Checking ────────────────────────────────────────────────────────────────────────────

/// Whether `id` is a Flatpak app id, and so safe to hand to `flatpak` as one.
///
/// Reverse-DNS, as `^[A-Za-z][A-Za-z0-9_-]*(\.[A-Za-z0-9_-]+){2,}$`: at least three
/// dot-separated parts, starting with a letter, and at most 255 characters. The ids come from
/// flatpak's own output, but they reach a command line from a row a person clicked, and a
/// value that starts with `-` would be read by flatpak as an option. Starting with a letter
/// rules that out, and the character set rules out everything else a command could misread.
pub fn is_app_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return false;
    }
    let mut parts = id.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    let word = |s: &str| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    if !first.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) || !word(first) {
        return false;
    }
    let mut rest = 0;
    for part in parts {
        if !word(part) {
            return false;
        }
        rest += 1;
    }
    rest >= 2
}

/// Refuse an id that is not one, in words the Package Manager can show.
fn checked(id: &str) -> Result<&str, String> {
    if is_app_id(id) {
        Ok(id)
    } else {
        Err(format!("`{id}` is not a Flatpak app id"))
    }
}

// ── Building commands ───────────────────────────────────────────────────────────────────

fn flatpak(args: &[&str]) -> Vec<String> {
    std::iter::once("flatpak").chain(args.iter().copied()).map(str::to_string).collect()
}

/// The command that adds Flathub to this account's own installation.
///
/// `--if-not-exists` so a second run is not an error. It still fetches the `.flatpakrepo`
/// file before it looks, which is why [`has_flathub`] is asked first and this runs only when
/// the remote is missing.
pub fn remote_add_command() -> Vec<String> {
    flatpak(&["remote-add", "--user", "--if-not-exists", REMOTE, REMOTE_URL])
}

/// The command that lists this account's remotes by name.
pub fn remotes_command() -> Vec<String> {
    flatpak(&["remotes", "--user", "--columns=name"])
}

/// The command that searches Flathub's catalogue.
///
/// `flatpak search` reads the remote's appstream data and refreshes it itself when it is more
/// than a day old, so there is no separate appstream step. The columns are named so their
/// order is ours, not a default that could change between versions. `--` ends the options,
/// so a query that starts with `-` is searched for rather than read as a flag.
pub fn search_command(query: &str) -> Vec<String> {
    let mut cmd = flatpak(&[
        "search",
        "--user",
        "--columns=application,name,version,branch,remotes,description",
        "--",
    ]);
    cmd.push(query.to_string());
    cmd
}

/// The command that lists the apps this account has installed (not the runtimes under them).
pub fn list_command() -> Vec<String> {
    flatpak(&[
        "list",
        "--user",
        "--app",
        "--columns=application,name,version,branch,size,description",
    ])
}

/// The command that names the installed apps with a newer build on their remote.
pub fn updates_command() -> Vec<String> {
    flatpak(&["remote-ls", "--user", "--updates", "--app", "--columns=application"])
}

/// The command that installs `id` from Flathub, with the runtime it needs.
///
/// `--noninteractive` because there is no terminal to answer a question on, and `-y` for the
/// one question it would otherwise ask: whether to pull the runtime as well.
pub fn install_command(id: &str) -> Result<Vec<String>, String> {
    let id = checked(id)?;
    Ok(flatpak(&["install", "--user", "--noninteractive", "-y", REMOTE, id]))
}

/// The command that removes `id`. The app's own data in `~/.var/app` stays, as a removed
/// Debian package's configuration does.
pub fn uninstall_command(id: &str) -> Result<Vec<String>, String> {
    let id = checked(id)?;
    Ok(flatpak(&["uninstall", "--user", "--noninteractive", "-y", id]))
}

/// The command that updates one installed app.
pub fn update_one_command(id: &str) -> Result<Vec<String>, String> {
    let id = checked(id)?;
    Ok(flatpak(&["update", "--user", "--noninteractive", "-y", id]))
}

/// The command that updates every app and runtime this account has installed.
pub fn update_all_command() -> Vec<String> {
    flatpak(&["update", "--user", "--noninteractive", "-y"])
}

// ── Running things ──────────────────────────────────────────────────────────────────────

/// Whether `flatpak` is on this machine at all.
///
/// A machine installed before the image carried it has none until the updater reconciles its
/// base packages, and the screen says that rather than failing every search.
pub fn available() -> bool {
    super::dock::find_program("flatpak").is_some()
}

/// Why Flathub cannot be used here, in the words the screen shows. `None` when it can.
pub fn unavailable_reason() -> Option<String> {
    (!available()).then(|| {
        "Flathub needs Flatpak, which this machine does not have yet. Install the latest \
         Yantrik update, which adds it."
            .to_string()
    })
}

fn run(cmd: &[String]) -> Result<String, String> {
    let (bin, args) = cmd.split_first().ok_or("no command")?;
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("could not run {bin}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        Err(format!("{} {}", stdout.trim(), stderr.trim()).trim().to_string())
    }
}

/// Whether this account already has the Flathub remote.
pub fn has_flathub() -> bool {
    run(&remotes_command()).map(|text| parse_remotes(&text).iter().any(|r| r == REMOTE)).unwrap_or(false)
}

/// Add Flathub to this account if it is not there yet. Needs the network the first time.
pub fn ensure_flathub() -> Result<(), String> {
    if let Some(why) = unavailable_reason() {
        return Err(why);
    }
    if has_flathub() {
        return Ok(());
    }
    run(&remote_add_command())
        .map(|_| ())
        .map_err(|e| format!("could not add Flathub: {e}"))
}

/// Flathub apps matching `query`, at most `limit` of them.
pub fn search(query: &str, limit: usize) -> Result<Vec<App>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    ensure_flathub()?;
    let text = run(&search_command(query)).map_err(|e| format!("Flathub search failed: {e}"))?;
    let mut apps = parse_search(&text);
    apps.truncate(limit);
    Ok(apps)
}

/// The apps this account has installed, each marked upgradable when its remote has newer.
///
/// No Flatpak on the machine is not an error here: it means nothing is installed this way.
pub fn list_installed() -> Result<Vec<App>, String> {
    if !available() {
        return Ok(Vec::new());
    }
    let mut apps = parse_list(&run(&list_command())?);
    if apps.is_empty() {
        return Ok(apps);
    }
    // Best-effort, like `apt list --upgradable`: it asks the remote, and an offline machine
    // should still see and remove what it has.
    let updates = run(&updates_command()).map(|t| parse_updates(&t)).unwrap_or_default();
    for app in &mut apps {
        app.upgradable = updates.iter().any(|u| *u == app.id);
    }
    Ok(apps)
}

// ── Parsing ─────────────────────────────────────────────────────────────────────────────

/// Split one line of flatpak's table output into its columns.
///
/// When its output is not a terminal, flatpak prints each row as the columns asked for,
/// separated by tabs, with no header and no truncation.
fn columns(line: &str) -> Vec<&str> {
    line.split('\t').map(str::trim).collect()
}

/// `flatpak search --columns=application,name,version,branch,remotes,description`.
///
/// A line whose first column is not an app id is not a result: that covers flatpak's "No
/// matches found", which it prints on stdout, and a header, should a version print one. An app
/// published on more than one remote lists them comma-separated, and only Flathub's are kept,
/// because Flathub is the remote an install names. The same id twice (two branches) is one row.
pub fn parse_search(text: &str) -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    for line in text.lines() {
        let f = columns(line);
        let id = f.first().copied().unwrap_or("");
        if !is_app_id(id) || out.iter().any(|a| a.id == id) {
            continue;
        }
        let remotes = f.get(4).copied().unwrap_or("");
        if !remotes.is_empty() && !remotes.split(',').any(|r| r.trim() == REMOTE) {
            continue;
        }
        let name = f.get(1).copied().unwrap_or("");
        out.push(App {
            id: id.to_string(),
            name: if name.is_empty() { id.to_string() } else { name.to_string() },
            version: f.get(2).copied().unwrap_or("").to_string(),
            branch: f.get(3).copied().unwrap_or("").to_string(),
            description: f.get(5).copied().unwrap_or("").to_string(),
            ..Default::default()
        });
    }
    out
}

/// `flatpak list --app --columns=application,name,version,branch,size,description`.
pub fn parse_list(text: &str) -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    for line in text.lines() {
        let f = columns(line);
        let id = f.first().copied().unwrap_or("");
        if !is_app_id(id) || out.iter().any(|a| a.id == id) {
            continue;
        }
        let name = f.get(1).copied().unwrap_or("");
        out.push(App {
            id: id.to_string(),
            name: if name.is_empty() { id.to_string() } else { name.to_string() },
            version: f.get(2).copied().unwrap_or("").to_string(),
            branch: f.get(3).copied().unwrap_or("").to_string(),
            // GLib formats sizes with a no-break space between number and unit.
            size_text: f.get(4).copied().unwrap_or("").replace('\u{a0}', " "),
            description: f.get(5).copied().unwrap_or("").to_string(),
            installed: true,
            upgradable: false,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// `flatpak remote-ls --updates --app --columns=application`: one app id per line.
pub fn parse_updates(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| columns(l).first().copied().unwrap_or("").to_string())
        .filter(|id| is_app_id(id))
        .collect()
}

/// `flatpak remotes --columns=name`: one remote name per line.
pub fn parse_remotes(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| columns(l).first().copied().unwrap_or("").to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from flatpak 1.16 with its stdout a pipe, as the Package Manager runs it:
    // tab-separated, no header, nothing cut short.
    const SEARCH: &str = "\
com.visualstudio.code\tVisual Studio Code\t1.93.1-1726079302\tstable\tflathub\tCode editing. Redefined.
com.vscodium.codium\tVSCodium\t1.93.1.24256\tstable\tflathub\tTelemetry-less code editing
com.visualstudio.code.tool.podman\tpodman\t\tstable\tflathub\tPodman for VS Code
org.example.Other\tOther\t2.0\tstable\tsomewhere-else\tNot on Flathub
com.visualstudio.code\tVisual Studio Code\t1.94.0\tbeta\tflathub\tCode editing. Redefined.
";

    const LIST: &str = "\
com.spotify.Client\tSpotify\t1.2.31.1205.g4d59ad7c\tstable\t1.3\u{a0}GB\tOnline music streaming service
com.discordapp.Discord\tDiscord\t0.0.66\tstable\t312.4\u{a0}MB\tMessaging, voice and video client
";

    #[test]
    fn app_ids_are_reverse_dns_and_nothing_else() {
        for good in [
            "com.visualstudio.code",
            "com.spotify.Client",
            "com.valvesoftware.Steam",
            "org.gnome.Platform.Locale",
            "io.github.some_user.My-App",
            "a.b.c",
        ] {
            assert!(is_app_id(good), "{good} is an app id");
        }
        for bad in [
            "",
            "spotify",
            "com.spotify",
            "-com.spotify.Client",
            "--system",
            "1com.example.App",
            "com..example.App",
            "com.example.App.",
            ".com.example.App",
            "com.example.App;rm -rf ~",
            "com.example.App --system",
            "com.example.$(id)",
            "com/example/App",
            "com.exämple.App",
            "flathub com.example.App",
        ] {
            assert!(!is_app_id(bad), "{bad:?} is not an app id");
        }
    }

    #[test]
    fn an_app_id_is_at_most_255_characters() {
        let at_limit = format!("a.b.{}", "c".repeat(251));
        assert_eq!(at_limit.len(), 255);
        assert!(is_app_id(&at_limit));
        assert!(!is_app_id(&format!("{at_limit}d")));
    }

    #[test]
    fn a_command_is_never_built_from_something_that_is_not_an_id() {
        assert!(install_command("--system").is_err());
        assert!(uninstall_command("com.example.App -y --all").is_err());
        assert!(update_one_command("spotify").is_err());
    }

    #[test]
    fn every_command_is_per_user_and_needs_no_one_at_a_terminal() {
        assert_eq!(
            install_command("com.spotify.Client").unwrap(),
            ["flatpak", "install", "--user", "--noninteractive", "-y", "flathub", "com.spotify.Client"]
        );
        assert_eq!(
            uninstall_command("com.spotify.Client").unwrap(),
            ["flatpak", "uninstall", "--user", "--noninteractive", "-y", "com.spotify.Client"]
        );
        assert_eq!(
            update_one_command("com.spotify.Client").unwrap(),
            ["flatpak", "update", "--user", "--noninteractive", "-y", "com.spotify.Client"]
        );
        assert_eq!(update_all_command(), ["flatpak", "update", "--user", "--noninteractive", "-y"]);
        assert_eq!(
            remote_add_command(),
            ["flatpak", "remote-add", "--user", "--if-not-exists", "flathub", REMOTE_URL]
        );
        for cmd in [
            remote_add_command(),
            remotes_command(),
            search_command("code"),
            list_command(),
            updates_command(),
            update_all_command(),
        ] {
            assert_eq!(cmd[0], "flatpak");
            assert!(cmd.iter().any(|a| a == "--user"), "{cmd:?} must be per-user");
            assert!(!cmd.iter().any(|a| a == "--system" || a == "sudo"), "{cmd:?}");
        }
    }

    #[test]
    fn a_query_is_searched_for_never_read_as_an_option() {
        let cmd = search_command("--system");
        let end = cmd.iter().position(|a| a == "--").expect("options are ended");
        assert_eq!(cmd[end + 1..], ["--system".to_string()]);
    }

    #[test]
    fn search_results_carry_the_id_the_install_needs() {
        let apps = parse_search(SEARCH);
        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            ["com.visualstudio.code", "com.vscodium.codium", "com.visualstudio.code.tool.podman"],
            "one row per id, and only apps Flathub serves"
        );
        let code = &apps[0];
        assert_eq!(code.name, "Visual Studio Code");
        assert_eq!(code.version, "1.93.1-1726079302");
        assert_eq!(code.branch, "stable");
        assert_eq!(code.description, "Code editing. Redefined.");
        assert!(!code.installed);
        assert_eq!(apps[2].version, "", "an empty column stays empty and does not shift the rest");
        assert_eq!(apps[2].description, "Podman for VS Code");
    }

    #[test]
    fn no_matches_is_no_results() {
        assert!(parse_search("No matches found\n").is_empty());
        assert!(parse_search("").is_empty());
        // A header, should a version print one when not on a terminal.
        let with_header = format!("Application ID\tName\tVersion\tBranch\tRemotes\tDescription\n{SEARCH}");
        assert_eq!(parse_search(&with_header).len(), 3);
    }

    #[test]
    fn an_app_on_several_remotes_is_kept_when_flathub_is_one() {
        let apps = parse_search("org.x.Y\tY\t1\tstable\tfedora,flathub\tWhy\n");
        assert_eq!(apps.len(), 1);
    }

    #[test]
    fn installed_apps_are_listed_by_name_with_their_size() {
        let apps = parse_list(LIST);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Discord", "sorted by name");
        assert_eq!(apps[0].id, "com.discordapp.Discord");
        assert_eq!(apps[0].size_text, "312.4 MB");
        assert_eq!(apps[1].id, "com.spotify.Client");
        assert_eq!(apps[1].version, "1.2.31.1205.g4d59ad7c");
        assert_eq!(apps[1].description, "Online music streaming service");
        assert!(apps.iter().all(|a| a.installed && !a.upgradable));
    }

    #[test]
    fn an_app_without_a_name_is_listed_by_its_id() {
        let apps = parse_list("org.x.Y\t\t1.0\tstable\t10 MB\t\n");
        assert_eq!(apps[0].name, "org.x.Y");
    }

    #[test]
    fn updates_and_remotes_are_one_name_per_line() {
        assert_eq!(
            parse_updates("com.spotify.Client\n\ncom.discordapp.Discord\nNothing to do.\n"),
            ["com.spotify.Client", "com.discordapp.Discord"]
        );
        assert_eq!(parse_remotes("flathub\nfedora\n"), ["flathub", "fedora"]);
        assert!(parse_remotes("").is_empty());
    }
}
