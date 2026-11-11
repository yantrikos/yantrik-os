//! Taking credentials out of the text of a problem record (#1171).
//!
//! A record becomes a public issue when the person sends it, so what it holds must not be a
//! secret. This is a list of shapes, not a promise that nothing else is sensitive: host names,
//! addresses and file names a log line happens to carry are not removed, and the README says so.
//!
//! The shapes, each replaced by `<redacted>` with what names the thing kept:
//! - a token after a well-known prefix (`sk-`, `ghp_`, `AKIA`, `AIza`, `hf_`, `glpat-`, `Bearer `,
//!   `key=`, `password=`, …), up to the next whitespace, quote or comma;
//! - a JSON-shaped pair whose key names a secret (`"api_key": "…"`, `"token"`, `"secret"`,
//!   `"password"`, and keys that end in those such as `access_token`);
//! - `user:pass@` in a URL;
//! - a JSON Web Token (`eyJ…`.`…`.`…`).

/// The prefixes after which the rest of the word is a credential.
const PREFIXES: &[&str] = &[
    "sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "AIza", "hf_", "glpat-", "Bearer ", "bearer ",
    "token=", "TOKEN=", "api_key=", "API_KEY=", "apikey=", "key=", "KEY=", "password=", "PASSWORD=", "secret=",
    "SECRET=",
];

/// Words that make a JSON key a secret's name, matched in any case anywhere in the key.
const SECRET_KEYS: &[&str] = &["api_key", "apikey", "token", "secret", "password", "passwd"];

/// `text` with every credential shape replaced.
pub(crate) fn redact_secrets(text: &str) -> String {
    redact_prefixed(&redact_jwts(&redact_url_credentials(&redact_json_pairs(text))))
}

/// Where a token ends: whitespace, a quote or a comma.
fn token_end(after: &str) -> usize {
    after.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',').unwrap_or(after.len())
}

fn redact_prefixed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for prefix in PREFIXES {
            if let Some(after) = rest.strip_prefix(prefix) {
                // Keep the prefix that names the thing, drop the thing.
                out.push_str(prefix);
                out.push_str("<redacted>");
                rest = &after[token_end(after)..];
                continue 'scan;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// `"api_key": "value"` and friends: the value goes, the key and the punctuation stay. A string
/// value runs to its closing quote (escapes honoured); anything else to the next `,`, `}`, `]`
/// or whitespace.
fn redact_json_pairs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        out.push_str(&rest[..=open]);
        rest = &rest[open + 1..];
        let Some(close) = rest.find('"') else { break };
        let key = &rest[..close];
        let after_key = &rest[close + 1..];
        let colon = after_key.trim_start();
        let named = {
            let lower = key.to_ascii_lowercase();
            !key.contains(char::is_whitespace) && SECRET_KEYS.iter().any(|k| lower.contains(k))
        };
        match (named, colon.strip_prefix(':')) {
            (true, Some(value)) => {
                let gap = &after_key[..after_key.len() - colon.len()];
                let value_start = value.len() - value.trim_start().len();
                let (spaces, value) = value.split_at(value_start);
                out.push_str(key);
                out.push('"');
                out.push_str(gap);
                out.push(':');
                out.push_str(spaces);
                let end = if value.starts_with('"') { string_end(value) } else { bare_end(value) };
                out.push_str(if value.starts_with('"') { "\"<redacted>\"" } else { "<redacted>" });
                rest = &value[end..];
            }
            // Not a secret's key: carry on after the key's closing quote, which is a quote the
            // scan must not mistake for an opening one.
            _ => {
                out.push_str(key);
                out.push('"');
                rest = after_key;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The length of the string literal at the start of `value` (which starts with `"`).
fn string_end(value: &str) -> usize {
    let mut escaped = false;
    for (i, c) in value.char_indices().skip(1) {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return i + 1,
            _ => escaped = false,
        }
    }
    value.len()
}

fn bare_end(value: &str) -> usize {
    value.find(|c: char| c == ',' || c == '}' || c == ']' || c.is_whitespace()).unwrap_or(value.len())
}

/// `scheme://user:pass@host` becomes `scheme://<redacted>@host`: the whole userinfo goes when it
/// holds a colon (a password), and stays when it is a user name alone.
fn redact_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let authority_end = tail.find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace() || c == '"' || c == '\'').unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        match authority.rfind('@') {
            Some(i) if authority[..i].contains(':') => {
                out.push_str("<redacted>");
                rest = &tail[i..];
            }
            _ => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

/// `eyJ…` followed by two more dot-separated base64url parts.
fn redact_jwts(text: &str) -> String {
    const START: &str = "eyJ";
    let url64 = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(START) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let run = tail.find(|c: char| !(url64(c) || c == '.')).unwrap_or(tail.len());
        let parts: Vec<&str> = tail[..run].split('.').collect();
        if parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(url64)) {
            out.push_str("eyJ<redacted>");
        } else {
            out.push_str(&tail[..START.len()]);
            rest = &tail[START.len()..];
            continue;
        }
        rest = &tail[run..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn things_shaped_like_credentials_are_redacted() {
        let out = redact_secrets("using sk-abc123DEF token=xyz, Authorization: Bearer eyJhbGci ghp_0123456789 done");
        assert_eq!(out, "using sk-<redacted> token=<redacted>, Authorization: Bearer <redacted> ghp_<redacted> done");
        assert_eq!(redact_secrets("nothing secret here"), "nothing secret here");
    }

    #[test]
    fn json_shaped_keys_lose_their_values() {
        let out = redact_secrets(
            r#"request {"api_key": "sk_live_9f8e7d", "user": "ada", "token":"abc.def", "secret" : "s3cr3t", "password": "hunter2 with spaces", "Access_Token": 12345, "retries": 3}"#,
        );
        assert_eq!(
            out,
            r#"request {"api_key": "<redacted>", "user": "ada", "token":"<redacted>", "secret" : "<redacted>", "password": "<redacted>", "Access_Token": <redacted>, "retries": 3}"#
        );
        for gone in ["sk_live", "hunter2", "s3cr3t", "12345", "abc.def"] {
            assert!(!out.contains(gone), "{gone}");
        }
        // An escaped quote inside the value does not end it early.
        assert_eq!(redact_secrets(r#"{"password": "a\"b", "n": 1}"#), r#"{"password": "<redacted>", "n": 1}"#);
        // A key that merely sits in a sentence, or a secret word in a value, is left.
        assert_eq!(redact_secrets(r#"the "token" was refused"#), r#"the "token" was refused"#);
        assert_eq!(redact_secrets(r#"{"note": "token expired"}"#), r#"{"note": "token expired"}"#);
    }

    #[test]
    fn a_password_in_a_url_goes_and_a_plain_host_stays() {
        assert_eq!(
            redact_secrets("fetch https://ada:hunter2@example.org/repo.git failed"),
            "fetch https://<redacted>@example.org/repo.git failed"
        );
        assert_eq!(redact_secrets("postgres://u:p%40ss@db:5432/x"), "postgres://<redacted>@db:5432/x");
        assert_eq!(redact_secrets("https://example.org/a@b"), "https://example.org/a@b");
        assert_eq!(redact_secrets("https://ada@example.org/"), "https://ada@example.org/", "a user name alone is no secret");
        assert_eq!(redact_secrets("see http://localhost:8888/search?q=a:b@c"), "see http://localhost:8888/search?q=a:b@c");
    }

    #[test]
    fn a_json_web_token_is_redacted_and_a_lookalike_is_not() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let out = redact_secrets(&format!("session {jwt} expired"));
        assert_eq!(out, "session eyJ<redacted> expired");
        assert!(!out.contains("dBjft") && !out.contains("hbGci"));
        assert_eq!(redact_secrets("eyJ and eyJhbGci.only two"), "eyJ and eyJhbGci.only two");
    }

    #[test]
    fn google_hugging_face_and_gitlab_tokens_are_redacted() {
        let out = redact_secrets("keys AIzaSyD-9tSrke72PouQMnMX-a7eZSW0jkFMBWY hf_abcdefghijklmnopqrstuvwxyz0123 glpat-xYz123_AbC-9 end");
        assert_eq!(out, "keys AIza<redacted> hf_<redacted> glpat-<redacted> end");
    }

    #[test]
    fn the_shapes_work_together_and_unicode_survives() {
        let out = redact_secrets(r#"échec "api_key": "k" at https://a:b@h/ with ghp_1 – fin"#);
        assert_eq!(out, r#"échec "api_key": "<redacted>" at https://<redacted>@h/ with ghp_<redacted> – fin"#);
    }
}
