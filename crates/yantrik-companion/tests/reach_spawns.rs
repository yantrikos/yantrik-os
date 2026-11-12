//! The second reach scan: a tool that starts a process by a name it does not spell (#1171).
//!
//! `reach_sources.rs` finds `Command::new("chromium")`. It cannot find `Command::new(app)`, whose name is
//! data. The scanning helpers are shared through `common/scan.rs`.

mod common;

use std::collections::HashSet;
use std::path::Path;

use common::scan::{
    load_sources, raw_sources, source, tool, tool_blocks, walk_with, without_test_items, Finding,
    Index, Source, DEPTH, NETWORK_PROGRAMS,
};
use common::{tool_names, tools};
use yantrik_companion_core::reach::{reach, Reach};

// ── The second scan: a program named by data, a shell line, or a network program's name in a literal ──
//
// `marker_in` finds `Command::new("chromium")`. It cannot find `Command::new(app)`, where the name is
// data (a `const` table, a saved template, a model's argument): that is how `apply_workspace_template`
// started Chromium with nobody there (round 7). So a tool that starts a process by a name it does not
// spell, or goes through a shell line, or holds a network program's name as a string anywhere in its
// module, is flagged unless it is non-local, or listed in [`SPAWNS_LOCAL`] with the reason it is fine.

/// What in `text` starts a process by a name it does not spell, or runs a shell line.
fn spawn_in(text: &str) -> Option<String> {
    const NEW: &str = "Command::new(";
    for (at, _) in text.match_indices(NEW) {
        let rest = text[at + NEW.len()..].trim_start();
        if !rest.starts_with('"') {
            return Some("Command::new(<a name that is not a literal>)".into());
        }
        let program = rest[1..].split('"').next().unwrap_or("");
        if matches!(program, "sh" | "bash" | "zsh" | "dash") {
            return Some(format!("Command::new(\"{program}\")"));
        }
        if program == "swaymsg" {
            return Some("swaymsg".into());
        }
    }
    if text.contains("\"swaymsg\"") || text.contains("swaymsg exec") {
        return Some("swaymsg exec".into());
    }
    if text.contains("\"-c\"")
        && (text.contains("\"sh\"") || text.contains("/bin/sh") || text.contains("\"bash\""))
    {
        return Some("sh -c".into());
    }
    None
}

/// A network program's name as a string literal (`"chromium"`), wherever it is in the text.
fn network_word_in(text: &str) -> Option<String> {
    NETWORK_PROGRAMS
        .iter()
        .find(|w| text.contains(&format!("\"{w}\"")))
        .map(|w| format!("\"{w}\" as a string"))
}

/// Tools that start a process by a computed name, a shell line, or hold a network program's name,
/// and that are `Local` for the arguments the test gives, with the reason that is fine. Each reason
/// was checked against the code: a tool is added here only when what it starts cannot reach out.
const SPAWNS_LOCAL: &[(&str, &str)] = &[
    ("network_interfaces", "asks the network service for the interface list over IPC and starts nothing; `ping` is in networking.rs only for `network_ping`/`network_traceroute`, which are in FETCH_NAMES"),
    ("network_ports", "`ss -tlnp` (or `-tnp`/`-tanp`, chosen from three words) and a `netstat -tlnp` fallback: literal programs reading the local socket table"),
    ("network_vpn_status", "literal `wg show`, `pgrep -a openvpn`, `nmcli ... connection show --active`, `ip -br link show type tun`: local reads, nothing sent"),
    ("list_processes", "a fixed `ps aux --sort=<one of three words> | head -n <usize>` line"),
    ("system_control", "`wpctl set-volume`/`set-mute` with a number capped at 50: no text of the model's reaches the line"),
    ("clipboard_transform", "`echo -n '<text with its single quotes removed>' | base64 -d`: a quoted word, a local decode"),
    ("antivirus_scan", "`clamdscan` or `clamscan` of a local path, chosen in the code; the update (`freshclam`) is `antivirus_update`, which is not Local"),
    ("package_search", "the backend's own query of its local package index (`apt-cache`, `pacman -Ss`, `apk search`), from a fixed table; `package_install` is not Local"),
    ("package_info", "the backend's query of its local index (`apt-cache show`, `pacman -Si`, `apk info`), from a fixed table"),
    ("package_list", "`dpkg --list`, `pacman -Q`, `apk list --installed`: what is installed, from a fixed table"),
    ("package_remove", "`apt-get remove`, `pacman -R`, `apk del`: removes what is installed and fetches nothing, from a fixed table"),
    ("run_command", "`safe_command::run` starts only a program on safe_command::PROGRAMS (`ls`, `cat`, `date`, ...); `curl` is on its deny-list"),
    ("script_write", "coder.rs holds the `pip3` word in the table `install_deps` reads; this tool only writes a file"),
    ("script_read", "as script_write: a file read"),
    ("script_patch", "as script_write: a file edit"),
    ("script_list", "as script_write: a directory listing"),
    ("script_delete", "as script_write: a file removal"),
    ("code_execute", "`coder::install_deps` is the package step, which asks this gate (`install_deps`) and is person-only; strict sandbox has no network"),
    ("script_run", "as code_execute"),
    ("list_windows", "window.rs names `chromium` only to match a window's title or app id; it starts nothing"),
    ("focus_window", "as list_windows: raises a window that exists"),
    ("close_window", "as list_windows: closes a window that exists"),
    ("focus_context", "as list_windows: a table of window names for a context"),
    ("clipboard_history", "clipboard.rs holds `curl` in its sensitive-text patterns, and `clipboard_fetch_url` (not Local) in the same file; this tool reads the history"),
    ("clipboard_analyze", "as clipboard_history"),
    ("text_action", "as clipboard_history: sends the text to the person's own model"),
];

fn spawners(sources: &[Source]) -> Vec<Finding> {
    let index = Index::new(sources);
    let mut found = Vec::new();
    for source in sources {
        for block in tool_blocks(&source.code).0 {
            let (mut seen, mut via) = (HashSet::new(), Vec::new());
            let mut marker = walk_with(
                spawn_in,
                &index,
                (&source.krate, &source.module),
                block,
                DEPTH,
                &mut seen,
                &mut via,
            );
            let mut how = via.join(" -> ");
            // The whole module counts for the network words: a `const` table outside the block is the usual place.
            if marker.is_none() {
                marker = network_word_in(&source.code);
                how = "its module".into();
            }
            let Some(marker) = marker else { continue };
            for tool in tool_names(block) {
                found.push(Finding {
                    tool,
                    marker: marker.clone(),
                    via: how.clone(),
                });
            }
        }
    }
    found
}

#[test]
fn a_local_tool_that_starts_a_process_by_a_computed_name_or_a_shell_line_is_listed_with_a_reason() {
    let registered = tools();
    let (mut checked, mut unlisted) = (0, Vec::new());
    for Finding { tool, marker, via } in spawners(&load_sources()) {
        let Some((_, category)) = registered.iter().find(|(n, _)| *n == tool) else {
            continue;
        };
        checked += 1;
        if reach(&tool, category, &serde_json::json!({})) == Reach::Local
            && !SPAWNS_LOCAL.iter().any(|(n, _)| *n == tool)
        {
            unlisted.push(format!("`{tool}` is Local and holds {marker} ({via})"));
        }
    }
    assert!(
        unlisted.is_empty(),
        "start what it starts through `app_act::gated_act`, or class it person-only in yantrik_companion_core::reach, or list it in          SPAWNS_LOCAL with the reason:
{}",
        unlisted.join("
")
    );
    assert!(
        checked > 5,
        "the scan found {checked} tools: it is not reading the sources"
    );
}

#[test]
fn the_spawn_scan_catches_the_workspace_template_shape() {
    // The round-7 route: a table outside the block, the name reaching `Command::new` as a variable.
    let code = format!(
        "const T: &[&str] = &[\"chromium\"];\n{}",
        tool("t", "for a in T {{ Command::new(a).spawn(); }}")
    );
    assert_eq!(spawners(&[source("a", "m", &code)]).len(), 1);
    let name_only = format!(
        "const T: &[&str] = &[\"chromium\"];\n{}",
        tool("t", "launch(T)")
    );
    assert_eq!(
        spawners(&[source("a", "m", &name_only)]).len(),
        1,
        "a network word as a literal, anywhere in the module"
    );
    for body in [
        "Command::new(app).spawn()",
        "Command::new(\"swaymsg\").args([\"exec\", app]).spawn()",
        "Command::new(\"sh\").args([\"-c\", line]).output()",
    ] {
        assert_eq!(
            spawners(&[source("a", "m", &tool("t", body))]).len(),
            1,
            "{body}"
        );
    }
    let fixed = tool("t", "Command::new(\"ls\").output()");
    assert!(spawners(&[source("a", "m", &fixed)]).is_empty());
    // And through a helper.
    let helper = format!(
        "fn run(p: &str) {{ Command::new(p).spawn(); }}\n{}",
        tool("t", "run(x)")
    );
    assert_eq!(spawners(&[source("a", "m", &helper)]).len(), 1);
}

#[test]
fn apply_workspace_template_starts_nothing_by_name() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-companion-tools/src/workspace.rs"),
    )
    .unwrap();
    let production = src.split("#[cfg(test)]").next().unwrap();
    assert!(
        !production.contains("Command::new"),
        "workspace.rs starts no program itself"
    );
    assert!(
        !production.contains("swaymsg"),
        "and has no swaymsg fallback"
    );
    assert!(
        !production.contains("\"chromium\""),
        "and ships no chromium template"
    );
    assert!(
        production.contains("app_act::gated_act("),
        "each app is opened through the one road"
    );
}

// ── The scan reads whole files, and a spawn cannot hide behind an alias ──

#[test]
fn a_test_module_before_a_tool_does_not_hide_the_tool() {
    // The round-8 blind spot: everything after the first `#[cfg(test)]` was dropped.
    let text = format!(
        "#[cfg(test)]\npub mod fake {{ pub fn f() {{ let x = 1; }} }}\n#[cfg(test)]\nuse std::io;\n{}\n#[cfg(test)]\nmod tests {{ fn t() {{ Command::new(\"ls\"); }} }}\n",
        tool("late_tool", "Command::new(x).spawn()")
    );
    let code = without_test_items(&text);
    assert!(code.contains("late_tool"), "{code}");
    assert!(
        !code.contains("fake") && !code.contains("mod tests"),
        "the test items themselves are gone: {code}"
    );
    assert_eq!(
        spawners(&[source("a", "m", &code)])
            .iter()
            .map(|f| f.tool.as_str())
            .collect::<Vec<_>>(),
        ["late_tool"]
    );
}

#[test]
fn the_scanned_crates_have_no_way_to_start_a_process_that_the_scan_cannot_see() {
    let mut found = Vec::new();
    for (path, text) in raw_sources() {
        for word in [
            "process::Command as",
            "Command as ",
            "libc::exec",
            "unistd::exec",
            "posix_spawn",
        ] {
            if text.contains(word) {
                found.push(format!("{path}: `{word}`"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "start a process with `Command::new(…)` so the reach scans see it:\n{}",
        found.join("\n")
    );
}

/// Round-8 LOW 3: an option cannot ride in as a package name (`apk search -U` refreshes the index), and
/// `service_control` runs only the three actions its schema lists.
#[test]
fn package_queries_refuse_an_option_and_service_control_checks_its_action() {
    let package = include_str!("../../yantrik-companion-tools/src/package.rs");
    assert_eq!(
        package.matches("starts_with('-')").count(),
        2,
        "package_search and package_info"
    );
    let service = include_str!("../../yantrik-companion-tools/src/service.rs");
    assert!(service.contains("matches!(action, \"start\" | \"stop\" | \"restart\")"));
}
