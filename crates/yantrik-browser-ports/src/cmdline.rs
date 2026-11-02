//! Reading a browser's command line for the one flag that says whose it is.
//!
//! A Chromium main process rewrites its own `/proc/<pid>/cmdline` into ONE space-joined string,
//! with no NUL between the words (the 520 gate found this on #1088). A program that splits on NUL
//! and looks for an argument that is exactly `--user-data-dir=...` finds nothing, and takes every
//! Chromium for the one on the default profile. So a command line is read both ways: as NUL
//! separated arguments when it has NULs, and as one string, split at spaces, when it does not.
//! A profile path with a space in it is then not read back whole in the second form; a caller
//! that knows the path it expects asks [`has_profile`], which compares the whole path and does
//! not depend on where the path ends.

const PROFILE: &str = "--user-data-dir=";
const DEBUGGING_PORT: &str = "--remote-debugging-port=";

/// The words of a command line up to the first bare `--`, and whether it was NUL separated (so
/// that a word is exactly one argument, spaces and all).
fn words(raw: &[u8]) -> (Vec<String>, bool) {
    let text = String::from_utf8_lossy(raw);
    let nul = raw.contains(&0);
    let all: Vec<String> = if nul {
        text.split('\0').filter(|a| !a.is_empty()).map(str::to_string).collect()
    } else {
        text.split(' ').filter(|a| !a.is_empty()).map(str::to_string).collect()
    };
    (all.into_iter().take_while(|w| w != "--").collect(), nul)
}

/// Every value `flag` (like `--user-data-dir=`) was given, in order. The `-flag=` single-dash
/// spelling counts too.
pub fn values(raw: &[u8], flag: &str) -> Vec<String> {
    let single = &flag[1..];
    words(raw)
        .0
        .iter()
        .filter_map(|w| w.strip_prefix(flag).or_else(|| w.strip_prefix(single)))
        .map(str::to_string)
        .collect()
}

/// How the profile was named.
#[derive(Debug, PartialEq, Eq)]
pub enum Profile {
    /// No `--user-data-dir`: the default profile.
    Default,
    /// Named once. Cut at the first space when the command line was one string.
    Named(String),
    /// Named twice or more, or with nothing after the `=`: not guessed at.
    Unclear,
}

pub fn profile(raw: &[u8]) -> Profile {
    let mut found = values(raw, PROFILE);
    match (found.len(), found.pop()) {
        (0, _) => Profile::Default,
        (1, Some(v)) if !v.is_empty() => Profile::Named(v),
        _ => Profile::Unclear,
    }
}

/// Whether the command line names `dir` as its one profile: the whole path, however the line was
/// written, and no other `--user-data-dir` beside it.
pub fn has_profile(raw: &[u8], dir: &std::path::Path) -> bool {
    if values(raw, PROFILE).len() != 1 {
        return false;
    }
    let want = format!("{PROFILE}{}", dir.display());
    let (_, nul) = words(raw);
    if nul {
        return words(raw).0.iter().any(|w| *w == want || *w == want[1..]);
    }
    // One string: the path may hold spaces, so look for it whole, flush against a word start and
    // followed by a space or the end.
    let text = String::from_utf8_lossy(raw);
    text.match_indices(&want).any(|(at, _)| {
        (at == 0 || text.as_bytes()[at - 1] == b' ')
            && matches!(text.as_bytes().get(at + want.len()), None | Some(b' '))
    })
}

/// Whether the command line asks for DevTools on `port`.
pub fn serves_port(raw: &[u8], port: u16) -> bool {
    values(raw, DEBUGGING_PORT).iter().any(|v| v.parse() == Ok(port))
}

/// The port the command line asks for, when it asks for exactly one.
pub fn debugging_port(raw: &[u8]) -> Option<u16> {
    match values(raw, DEBUGGING_PORT).as_slice() {
        [one] => one.parse().ok(),
        _ => None,
    }
}

/// Whether this is a Chromium main process (not a renderer, GPU or zygote helper).
pub fn is_main(raw: &[u8]) -> bool {
    let text = String::from_utf8_lossy(raw);
    !text.split(['\0', ' ']).any(|t| t.starts_with("--type="))
}

/// The program at the front of the line.
pub fn program(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).split(['\0', ' ']).next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const MV: &str = "/home/yantrik/.local/share/yantrik/mind-view-browser/chromium";

    fn nul(args: &[&str]) -> Vec<u8> {
        let mut v = args.join("\0").into_bytes();
        v.push(0);
        v
    }

    /// What `/proc/<pid>/cmdline` of a Chromium main process reads: one string, no NULs.
    fn joined(args: &[&str]) -> Vec<u8> {
        args.join(" ").into_bytes()
    }

    #[test]
    fn a_profile_is_read_from_nul_separated_and_from_chromiums_one_string() {
        let args = ["/usr/lib/chromium/chromium", "--remote-debugging-port=9223", &format!("--user-data-dir={MV}"), "about:blank"];
        for raw in [nul(&args), joined(&args)] {
            assert_eq!(profile(&raw), Profile::Named(MV.to_string()), "{:?}", String::from_utf8_lossy(&raw));
            assert!(has_profile(&raw, Path::new(MV)));
            assert!(!has_profile(&raw, Path::new("/home/yantrik/.config/chromium")));
            assert_eq!(debugging_port(&raw), Some(9223));
            assert!(serves_port(&raw, 9223) && !serves_port(&raw, 9222));
            assert!(is_main(&raw));
        }
    }

    #[test]
    fn no_flag_is_the_default_profile() {
        for raw in [nul(&["chromium", "--remote-debugging-port=9222"]), joined(&["chromium", "--remote-debugging-port=9222"])] {
            assert_eq!(profile(&raw), Profile::Default);
            assert!(!has_profile(&raw, Path::new(MV)));
        }
    }

    #[test]
    fn a_flag_given_twice_or_empty_is_not_guessed_at() {
        let twice = ["chromium", "--user-data-dir=/a", &format!("--user-data-dir={MV}")];
        for raw in [nul(&twice), joined(&twice)] {
            assert_eq!(profile(&raw), Profile::Unclear);
            assert!(!has_profile(&raw, Path::new(MV)), "the person's profile must not pass for Mind View's");
        }
        assert_eq!(profile(&nul(&["chromium", "--user-data-dir="])), Profile::Unclear);
    }

    #[test]
    fn a_path_is_matched_whole_not_as_a_prefix() {
        let raw = joined(&["chromium", &format!("--user-data-dir={MV}-extra")]);
        assert!(!has_profile(&raw, Path::new(MV)));
        let raw = nul(&["chromium", &format!("--user-data-dir={MV}-extra")]);
        assert!(!has_profile(&raw, Path::new(MV)));
        let raw = joined(&["chromium", &format!("--user-data-dir={MV}")]);
        assert!(has_profile(&raw, Path::new(MV)));
    }

    #[test]
    fn a_path_with_a_space_is_matched_whole() {
        let dir = "/home/a b/.local/share/yantrik/mind-view-browser/chromium";
        for raw in [joined(&["chromium", &format!("--user-data-dir={dir}"), "--no-first-run"]), nul(&["chromium", &format!("--user-data-dir={dir}")])] {
            assert!(has_profile(&raw, Path::new(dir)));
            // Not a path that merely ends where the first space does, in the NUL form that is exact.
            assert!(!has_profile(&raw, Path::new("/home/a b/.local/share/yantrik")) || raw.iter().all(|b| *b != 0));
        }
    }

    #[test]
    fn the_single_dash_spelling_and_the_bare_double_dash() {
        assert_eq!(profile(&nul(&["chromium", "-user-data-dir=/p"])), Profile::Named("/p".into()));
        // After `--` nothing is a flag.
        assert_eq!(profile(&nul(&["chromium", "--", "--user-data-dir=/p"])), Profile::Default);
        assert_eq!(profile(&joined(&["chromium", "--", "--user-data-dir=/p"])), Profile::Default);
        // `--x=` is not also counted as `-x=`.
        assert_eq!(values(&nul(&["chromium", "--user-data-dir=/p"]), PROFILE).len(), 1);
    }

    #[test]
    fn helpers_are_not_main_processes() {
        assert!(!is_main(&joined(&["/usr/lib/chromium/chromium", "--type=renderer", "--user-data-dir=/p"])));
        assert!(!is_main(&nul(&["chromium", "--type=gpu-process"])));
        assert_eq!(program(&joined(&["/usr/lib/chromium/chromium", "--x"])), "/usr/lib/chromium/chromium");
        assert_eq!(program(&nul(&["/usr/bin/chromium", "--x"])), "/usr/bin/chromium");
    }
}
