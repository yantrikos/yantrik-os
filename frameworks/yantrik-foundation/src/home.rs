//! A path as a person writes it: `~/Documents/notes` means their home. Expanded once, at the
//! boundary where their text comes in; everything inside carries a `PathBuf`. Seven apps had
//! their own copy of this.

use std::path::{Path, PathBuf};

use crate::place::home;

/// `~` and `~/…` become the home directory; anything else is the path as written.
pub fn expand(text: &str) -> PathBuf {
    let text = text.trim();
    if text == "~" {
        return home();
    }
    match text.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(text),
    }
}

/// The path as a person reads it: the home directory shown as `~`.
pub fn shown(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tilde_is_the_home_and_nothing_else_is_touched() {
        assert_eq!(expand("~"), home());
        assert_eq!(expand(" ~/a/b "), home().join("a/b"));
        assert_eq!(expand("/etc/hosts"), PathBuf::from("/etc/hosts"));
        assert_eq!(expand("~user/x"), PathBuf::from("~user/x"), "another user's home is not guessed");
        assert_eq!(shown(&home().join("Documents")), "~/Documents");
        assert_eq!(shown(&home()), "~");
        assert_eq!(shown(Path::new("/tmp/x")), "/tmp/x");
    }
}
