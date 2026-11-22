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
/// `PATH` always, then `extra` (a door's own, named in its code or the person's server config),
/// sorted and without repeats (the last value of a name wins). Nothing else is passed on.
pub fn allowed(source: impl IntoIterator<Item = (String, String)>, home: &Path, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = source.into_iter().filter(|(k, _)| kept(k) && k != "HOME").collect();
    if !out.iter().any(|(k, _)| k == "PATH") {
        out.push(("PATH".to_string(), DEFAULT_PATH.to_string()));
    }
    out.push(("HOME".to_string(), home.to_string_lossy().into_owned()));
    out.extend(extra.iter().filter(|(k, _)| !k.is_empty() && !k.contains('=') && !k.contains('\0')).cloned());
    // The last of a name wins, so a door's own value replaces the list's.
    let mut last: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (k, v) in out {
        last.insert(k, v);
    }
    last.into_iter().collect()
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
}
