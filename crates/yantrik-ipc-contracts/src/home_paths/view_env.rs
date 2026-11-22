//! The environment of a command in the outside view: an allow list (#1265, round 8).
//!
//! Every door that starts a view (the job, `mind_command` for the companion's spawned programs,
//! MCP servers, `claude` and the task manager) builds the command's environment here, so none
//! hands on what the shell happens to hold (API keys, an agent socket, the session bus address,
//! `GNUPGHOME`, git's variables). The words are `--clearenv`, then one `--setenv` per name that
//! is on the list, then whatever the door's own code names on purpose (`extra`).

use std::path::Path;

/// The names a command keeps: `LC_*` as well.
const KEPT: &[&str] = &["PATH", "HOME", "LANG", "TERM", "USER"];

/// Where programs are looked for when the source has no `PATH`.
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

fn kept(name: &str) -> bool {
    KEPT.contains(&name) || name.strip_prefix("LC_").is_some_and(|rest| !rest.is_empty())
}

/// The variables of `source` that are on the list, with `HOME` always the account's `home`, a
/// `PATH` always, then `extra`, sorted and without repeats (the last value of a name wins). Nothing
/// else is passed on.
///
/// `extra` comes only from code constants of the calling door or from the person's own server config,
/// never from a value a Mind chooses. It cannot set `HOME`: that is the account's, or the door's own
/// stand-in set by the door after this call.
pub fn allowed(source: impl IntoIterator<Item = (String, String)>, home: &Path, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = source.into_iter().filter(|(k, _)| kept(k) && k != "HOME").collect();
    if !out.iter().any(|(k, _)| k == "PATH") {
        out.push(("PATH".to_string(), DEFAULT_PATH.to_string()));
    }
    out.push(("HOME".to_string(), home.to_string_lossy().into_owned()));
    out.extend(extra.iter().filter(|(k, _)| !k.is_empty() && !k.contains('=') && !k.contains('\0') && k != "HOME").cloned());
    // The last of a name wins, so a door's own value replaces the list's.
    let mut last: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (k, v) in out {
        last.insert(k, v);
    }
    last.into_iter().collect()
}

/// Whether a name changes where a program loads code or settings from (`LD_*`, `BASH_ENV`, `GIT_*`,
/// `PYTHONPATH`, `NODE_OPTIONS` and the like): a name a manifest's `env` must not set.
pub fn loads_code_or_settings(name: &str) -> bool {
    const PREFIXES: &[&str] = &["LD_", "GIT_", "XDG_", "DBUS_"];
    const NAMES: &[&str] =
        &["BASH_ENV", "ENV", "PROMPT_COMMAND", "SSH_AUTH_SOCK", "PYTHONSTARTUP", "PYTHONPATH", "NODE_OPTIONS", "PERL5OPT", "RUBYOPT"];
    NAMES.contains(&name) || PREFIXES.iter().any(|p| name.starts_with(p))
}

/// The `env` of a skill manifest (third-party data) split into what is passed and the names dropped
/// because they change where a program loads code or settings from. Nothing is refused; the caller
/// logs each dropped name. The person's own server config does not go through this.
pub fn from_manifest(env: impl IntoIterator<Item = (String, String)>) -> (Vec<(String, String)>, Vec<String>) {
    let (mut kept, mut dropped) = (Vec::new(), Vec::new());
    for (k, v) in env {
        if loads_code_or_settings(&k) { dropped.push(k) } else { kept.push((k, v)) }
    }
    kept.sort();
    dropped.sort();
    (kept, dropped)
}

/// This process's own environment through [`allowed`].
pub fn process_env(home: &Path, extra: &[(String, String)]) -> Vec<(String, String)> {
    allowed(std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))), home, extra)
}

/// The `bwrap` words: `--clearenv`, then each of `env`. They go before every other `--setenv`.
pub fn words(env: &[(String, String)]) -> Vec<String> {
    let mut a = vec!["--clearenv".to_string()];
    for (k, v) in env {
        a.extend(["--setenv".to_string(), k.clone(), v.clone()]);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn only_the_list_passes_and_home_is_the_accounts() {
        let src = pairs(&[
            ("PATH", "/x"), ("HOME", "/wrong"), ("LANG", "C"), ("LC_ALL", "C"), ("TERM", "dumb"), ("USER", "p"),
            ("ANTHROPIC_API_KEY", "k"), ("SSH_AUTH_SOCK", "/s"), ("DBUS_SESSION_BUS_ADDRESS", "u"), ("GNUPGHOME", "g"),
            ("XDG_RUNTIME_DIR", "/r"), ("GIT_DIR", "/g"), ("LC_", "bare"),
        ]);
        let got = allowed(src, Path::new("/home/p"), &[]);
        let names: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["HOME", "LANG", "LC_ALL", "PATH", "TERM", "USER"]);
        assert_eq!(got[0].1, "/home/p");
    }

    #[test]
    fn a_path_is_always_there_and_extra_is_named_by_the_door() {
        let got = allowed(Vec::new(), Path::new("/home/p"), &pairs(&[("YANTRIK_OUTPUT_DIR", "/o")]));
        assert!(got.iter().any(|(k, v)| k == "PATH" && v == DEFAULT_PATH));
        assert!(got.iter().any(|(k, v)| k == "YANTRIK_OUTPUT_DIR" && v == "/o"));
        let w = words(&got);
        assert_eq!(w[0], "--clearenv");
        assert_eq!(w.iter().filter(|x| *x == "--clearenv").count(), 1);
    }

    #[test]
    fn extra_cannot_replace_home() {
        let got = allowed(Vec::new(), Path::new("/home/p"), &pairs(&[("HOME", "/evil"), ("X", "1")]));
        assert!(got.contains(&("HOME".to_string(), "/home/p".to_string())) && !got.iter().any(|(_, v)| v == "/evil"));
        assert!(got.contains(&("X".to_string(), "1".to_string())));
    }

    #[test]
    fn a_manifests_loader_names_are_dropped_and_the_rest_kept() {
        let env = pairs(&[
            ("LD_PRELOAD", "x"), ("BASH_ENV", "x"), ("ENV", "x"), ("PROMPT_COMMAND", "x"), ("GIT_CONFIG_COUNT", "1"), ("XDG_CONFIG_HOME", "x"),
            ("SSH_AUTH_SOCK", "x"), ("DBUS_SESSION_BUS_ADDRESS", "x"), ("PYTHONSTARTUP", "x"), ("PYTHONPATH", "x"), ("NODE_OPTIONS", "x"),
            ("PERL5OPT", "x"), ("RUBYOPT", "x"), ("API_URL", "u"), ("ENVIRONMENT", "prod"), ("GITHUB_TOKEN", "t"),
        ]);
        let (kept, dropped) = from_manifest(env);
        let k: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(k, ["API_URL", "ENVIRONMENT", "GITHUB_TOKEN"]);
        assert_eq!(dropped.len(), 13);
        assert!(dropped.contains(&"LD_PRELOAD".to_string()) && dropped.contains(&"GIT_CONFIG_COUNT".to_string()));
    }
}
