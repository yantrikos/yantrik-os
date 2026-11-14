//! Files that hold a credential wherever they are kept (#1247).
//!
//! [`super::PROTECTED`] is a list of places, and a credential copied out of its place is as secret
//! as the original: on VM 520 an ordinary chat turn could `find` and `cat` a `~/backup/tokens.json`.
//! These lists match the *name* of the last part of a path, ignoring case, so `is_protected_with`
//! refuses the copy on every side that shares the rule. The match is by exact name, ending or
//! start, never by a word somewhere inside it: `tokens.md` and `keynote.txt` stay readable.

use std::path::Path;

/// Files that hold a credential wherever they are kept: a copy in a backup folder is as secret as
/// the original. Compared in lower case against the whole last part of the path.
pub const SECRET_NAMES: &[&str] = &[
    "tokens.json", "token.json", "credentials.json", "credentials", ".credentials.json", "auth.json",
    "oauth_creds.json", "client_secret.json", "service_account.json", ".env", ".netrc", ".pgpass",
    ".git-credentials", ".vault-token", ".npmrc", ".pypirc", "id_rsa", "id_dsa", "id_ecdsa", "id_ed25519",
    "id_ecdsa_sk", "id_ed25519_sk", "secring.gpg", "pubring.kbx", "trustdb.gpg", "login.keyring",
    "user.keystore", "key4.db", "logins.json", "cookies.sqlite", "cookies", "login data", "web data",
];

/// Endings matched at the end of the name.
pub const SECRET_SUFFIXES: &[&str] =
    &[".key", ".pem", ".p12", ".pfx", ".jks", ".keystore", ".kdbx", ".keyring", ".ovpn", ".gpg-key"];

/// Starts of names, for the dated and suffixed copies: `client_secret_123.json`, `.env.local`,
/// `id_rsa.bak`.
pub const SECRET_PREFIXES: &[&str] =
    &["client_secret", "service-account", "service_account", ".env.", "id_rsa", "id_ed25519", "id_ecdsa"];

/// The one exception to the `id_*` prefixes: a public key is not a secret.
const PUBLIC_KEY_ENDING: &str = ".pub";

/// Whether a file called `name` (a last path part, not a path) holds a credential by its name.
pub fn is_secret_name(name: &str) -> bool {
    let name = name.to_lowercase();
    SECRET_NAMES.contains(&name.as_str())
        || SECRET_SUFFIXES.iter().any(|end| name.ends_with(end))
        || (!name.ends_with(PUBLIC_KEY_ENDING) && SECRET_PREFIXES.iter().any(|start| name.starts_with(start)))
}

/// Whether `path` names a secret file: by its last part as written, and by the last part of where
/// it leads when it is a link (`~/notes.txt -> ~/backup/tokens.json` is that file).
pub(super) fn names_a_secret(path: &Path) -> bool {
    let named = |p: &Path| p.file_name().is_some_and(|n| is_secret_name(&n.to_string_lossy()));
    named(path)
        || path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
            .then(|| path.canonicalize().ok())
            .flatten()
            .is_some_and(|real| named(&real))
}

/// The shortest start of a name that a search pattern must give before it asks for a secret by
/// name: `id_*` and `tok*` do, `i*` and `*` do not.
const LEAST_GIVEN: usize = 3;

/// Whether a shell-style pattern (`*` and `?`) given to a name search asks for a secret by name:
/// it spells one (`tokens.json`), ends as the suffixes do (`*.pem`), or starts with enough of a
/// secret's name to match one (`id_*`, `.env*`). A broad pattern that only happens to include
/// secrets (`*`, `*.json`) is left to the output filter, which drops what it must not show.
pub fn pattern_asks_for_a_secret(pattern: &str) -> bool {
    let pattern = pattern.to_lowercase();
    if !pattern.contains(['*', '?']) {
        return is_secret_name(&pattern);
    }
    if SECRET_SUFFIXES.iter().any(|end| pattern.ends_with(end)) {
        return true;
    }
    let given = pattern.chars().take_while(|c| !matches!(c, '*' | '?')).count();
    given >= LEAST_GIVEN
        && SECRET_NAMES
            .iter()
            .map(|n| n.to_string())
            .chain(SECRET_PREFIXES.iter().flat_map(|start| [start.to_string(), format!("{start}x")]))
            .any(|name| wildcard_matches(pattern.as_bytes(), name.as_bytes()))
}

/// `*` (any run) and `?` (any one byte) against `name`; every other byte stands for itself.
fn wildcard_matches(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|skip| wildcard_matches(rest, &name[skip..])),
        Some((b'?', rest)) => name.split_first().is_some_and(|(_, tail)| wildcard_matches(rest, tail)),
        Some((byte, rest)) => name.split_first().is_some_and(|(first, tail)| first == byte && wildcard_matches(rest, tail)),
    }
}
