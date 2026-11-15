//! Files that hold a credential wherever they are kept (#1247).
//!
//! [`super::PROTECTED`] is a list of places, and a credential copied out of its place is as secret
//! as the original: on VM 520 an ordinary chat turn could `find` and `cat` a `~/backup/tokens.json`.
//! These lists match the *name* of the last part of a path, ignoring case, so `is_protected_with`
//! refuses the copy on every side that shares the rule. The match is by exact name, ending or
//! start, never by a word somewhere inside it: `tokens.md` and `keynote.txt` stay readable.

use std::collections::HashSet;
use std::path::Path;

/// Files that hold a credential wherever they are kept: a copy in a backup folder is as secret as
/// the original. Compared in lower case against the whole last part of the path.
pub const SECRET_NAMES: &[&str] = &[
    "tokens.json", "token.json", "credentials.json", "credentials", ".credentials.json", "auth.json",
    "oauth_creds.json", "client_secret.json", "service_account.json", ".env", ".netrc", ".pgpass",
    ".git-credentials", ".vault-token", ".npmrc", ".pypirc", "id_rsa", "id_dsa", "id_ecdsa", "id_ed25519",
    "id_ecdsa_sk", "id_ed25519_sk", "secring.gpg", "pubring.kbx", "trustdb.gpg", "login.keyring",
    "user.keystore", "key4.db", "logins.json", "cookies.sqlite", "cookies", "login data", "web data",
    // Cloud and CLI credentials copied out of their folders (#1247, review M4).
    "application_default_credentials.json", "access_tokens.db", "credentials.db", "kubeconfig", "hosts.yml",
    ".s3cfg", ".boto", "credentials.tfrc.json",
    // Other dotfiles that hold a password or a token.
    ".envrc", ".my.cnf", ".htpasswd", ".authinfo", ".authinfo.gpg",
    // Browser stores: Firefox's older key store and sign-ins, Chrome's cookie key, its sign-in
    // database for another account and the journals SQLite leaves beside a store.
    "key3.db", "signons.sqlite", "local state", "login data for account", "cookies-journal",
    "login data-journal",
];

/// Endings matched at the end of the name.
pub const SECRET_SUFFIXES: &[&str] =
    &[
        ".key", ".pem", ".p12", ".pfx", ".jks", ".keystore", ".kdbx", ".keyring", ".ovpn", ".gpg-key", ".ppk", ".p8",
        ".keytab", ".age", ".kdb", ".keychain", ".keychain-db",
    ];

/// Starts of names, for the dated and suffixed copies: `client_secret_123.json`, `.env.local`,
/// `id_rsa.bak`.
pub const SECRET_PREFIXES: &[&str] =
    &["client_secret", "service-account", "service_account", ".env.", "id_rsa", "id_dsa", "id_ed25519", "id_ecdsa"];

/// The one exception to the `id_*` prefixes: a public key is not a secret.
const PUBLIC_KEY_ENDING: &str = ".pub";

fn matches_a_list(name: &str) -> bool {
    SECRET_NAMES.contains(&name)
        || SECRET_SUFFIXES.iter().any(|end| name.ends_with(end))
        || (!name.ends_with(PUBLIC_KEY_ENDING) && SECRET_PREFIXES.iter().any(|start| name.starts_with(start)))
}

/// Whether a file called `name` (a last part, not a path) holds a credential by its name, or is a
/// copy of one made by an editor, a package manager or a file manager (`tokens.json.bak`,
/// `.tokens.json.swp`, `tokens (1).json`; see [`super::copy_marker`]). Exact names, endings and
/// starts only; `notes.bak.md` and `old-tokens-notes.txt` are not copies of anything.
pub fn is_secret_name(name: &str) -> bool {
    // No file has a longer name (NAME_MAX), so a longer one is no one's copy: it is judged as it
    // stands, which keeps a page's 4 KB title from costing seconds (review R6-M2). Its length is
    // the name's as given: lower-casing makes `İ` longer, and a real 254-byte name is a name
    // (review R7-L2).
    let too_long = name.len() > NAME_MAX;
    let name = name.to_lowercase();
    if too_long {
        return matches_a_list(&name);
    }
    named_or_copied(&name, &mut HashSet::new())
}

/// The longest name a file has, in bytes.
pub const NAME_MAX: usize = 255;

/// Whether `name` is a copy of something by its marker alone, whatever it is a copy of:
/// `client_secret.pub - Copy.json` is, `id_rsa.pub - gedit` is not.
pub fn is_a_copy_name(name: &str) -> bool {
    !super::copy_marker::originals(&name.to_lowercase()).is_empty()
}

/// A name is judged as it stands and by what it is a copy of (markers peel one at a time), so
/// `deploy-10:30:00.pem` and `id_rsa%20old` are keys whatever a copy rule keeps of them (review
/// H1). The one exception is a public key: `id_rsa.pub.bak` is as public as `id_rsa.pub`, though
/// `id_rsa.pub.bak` by itself starts like a private key.
///
/// `seen` holds the originals already judged, so stacked markers cost each distinct name once (review
/// R6-M2). A name judged earlier was not secret, or the answer would have ended the walk.
fn named_or_copied(name: &str, seen: &mut HashSet<String>) -> bool {
    if !seen.insert(name.to_string()) {
        return false;
    }
    let originals = super::copy_marker::originals(name);
    let public_copy = originals.iter().any(|original| original.ends_with(PUBLIC_KEY_ENDING));
    (!public_copy && matches_a_list(name)) || originals.iter().any(|original| named_or_copied(original, seen))
}

/// Whether `path` names a secret file: by its last part as written, and by the last part of where
/// it leads when it is a link (`~/notes.txt -> ~/backup/tokens.json` is that file).
pub(super) fn names_a_secret(path: &Path) -> bool {
    names_a_secret_by_name(path)
        || path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
            .then(|| path.canonicalize().ok())
            .flatten()
            .is_some_and(|real| names_a_secret_by_name(&real))
}

/// Whether the last part of `path` as written is a secret's name: no filesystem.
pub(super) fn names_a_secret_by_name(path: &Path) -> bool {
    path.file_name().is_some_and(|n| is_secret_name(&n.to_string_lossy()))
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
