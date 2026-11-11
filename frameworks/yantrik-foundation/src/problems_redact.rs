//! Taking credentials out of the text of a problem record (#1171).
//!
//! A record becomes a public issue when the person sends it, so what it holds must not be a
//! secret. This is a list of shapes, not a promise that nothing else is sensitive: host names,
//! addresses and file names a log line happens to carry are not removed, and the README says so.
//!
//! The shapes, each replaced by `<redacted>` with what names the thing kept:
//! - a private key block (`-----BEGIN … PRIVATE KEY-----` to its END line, or to the end);
//! - a token after a well-known prefix (`sk-`, `sk_live_`, `ghp_`, `gsk_`, `ya29.`, `AKIA`, `AIza`,
//!   `hf_`, `glpat-`, `Bearer `, `key=`, `password=`, …) at the start of a word, up to the next
//!   whitespace, quote or comma, and the credential after `Basic `;
//! - a pair whose key names a secret, in any of the shapes a log carries one: JSON
//!   (`"api_key": "…"`), a Rust `Debug` struct (`api_key: "…"`, `api_key: Some("…")`), YAML or an
//!   env line (`api_key: …`, `API_KEY=…`), with `-` read as `_` (`x-api-key`);
//! - `user:pass@` in a URL, whatever the password holds;
//! - a JSON Web Token (`eyJ…`.`…`.`…`), also at the end of a sentence.

/// The prefixes after which the rest of the word is a credential.
const PREFIXES: &[&str] = &[
    "sk-", "sk_live_", "sk_test_", "rk_live_", "rk_test_", "gsk_", "ya29.", "ghp_", "gho_", "github_pat_", "xoxb-",
    "xoxp-", "AKIA", "AIza", "hf_", "glpat-", "Bearer ", "bearer ", "token=", "TOKEN=", "api_key=", "API_KEY=", "apikey=",
    "key=", "KEY=", "password=", "PASSWORD=", "secret=", "SECRET=",
];

/// Words that make a key a secret's name, matched in any case anywhere in the key once `-` is `_`.
const SECRET_KEYS: &[&str] =
    &["api_key", "apikey", "token", "secret", "password", "passwd", "passphrase", "private_key", "credential"];

/// `text` with every credential shape replaced.
pub(crate) fn redact_secrets(text: &str) -> String {
    redact_pairs(&redact_basic(&redact_prefixed(&redact_jwts(&redact_url_credentials(&redact_pem(text))))))
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Where a token ends: whitespace, a quote, a comma or the `&` of the next query parameter.
fn token_end(after: &str) -> usize {
    after.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '&')).unwrap_or(after.len())
}

fn redact_prefixed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut previous = ' ';
    'scan: while !rest.is_empty() {
        // A prefix counts at the start of a word: "disk-usage" holds "sk-" and is not a key.
        if !previous.is_ascii_alphanumeric() {
            for prefix in PREFIXES {
                if let Some(after) = rest.strip_prefix(prefix) {
                    // Keep the prefix that names the thing, drop the thing.
                    out.push_str(prefix);
                    out.push_str("<redacted>");
                    previous = '>';
                    rest = &after[token_end(after)..];
                    continue 'scan;
                }
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        previous = ch;
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// `Basic dXNlcjpwYXNzd29yZA==`: the base64 of `user:password`. Only a base64 word of some length
/// counts, so the English "basic usage" stays.
fn redact_basic(text: &str) -> String {
    let b64 = |c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.to_ascii_lowercase().find("basic ") {
        let (head, tail) = rest.split_at(at + 6);
        out.push_str(head);
        let word = tail.find(|c: char| !b64(c)).unwrap_or(tail.len());
        if word >= 8 {
            out.push_str("<redacted>");
            rest = &tail[word..];
        } else {
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// A private key block, from its BEGIN line to its END line (or to the end of the text, when the
/// record cut it short). Certificates and public keys are not secret and stay.
fn redact_pem(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    const END: &str = "-----END ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(BEGIN) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let label_end = tail[BEGIN.len()..].find("-----").map_or(tail.len(), |i| BEGIN.len() + i);
        if !tail[BEGIN.len()..label_end.max(BEGIN.len())].contains("PRIVATE KEY") {
            out.push_str(BEGIN);
            rest = &tail[BEGIN.len()..];
            continue;
        }
        out.push_str("<redacted private key>");
        rest = tail.find(END).map_or("", |e| {
            let after_label = &tail[e + END.len()..];
            after_label.find("-----").map_or("", |close| &after_label[close + 5..])
        });
    }
    out.push_str(rest);
    out
}

/// Whether `key` (as written in a log) names a secret. A count of tokens is not one (`max_tokens`).
fn names_a_secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    let auth = key.split('_').any(|part| part == "auth");
    (auth || SECRET_KEYS.iter().any(|k| key.contains(k))) && !key.ends_with("tokens")
}

/// Pairs whose key names a secret: the value goes, the key and the punctuation stay. A value is a
/// quoted string (escapes honoured), `Some("…")` as `Debug` prints an `Option`, or a bare word that
/// runs to the next `,`, `}`, `]`, `)`, `&` or whitespace. The separator is `:` or `=`, after a key
/// that may be quoted.
fn redact_pairs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if !is_word(c) {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let end = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
        let (key, after) = rest.split_at(end);
        out.push_str(key);
        rest = after;
        if !names_a_secret(key) {
            continue;
        }
        let closing_quote = usize::from(rest.starts_with('"') || rest.starts_with('\''));
        let spaced = rest[closing_quote..].trim_start_matches([' ', '\t']);
        let Some(value) = spaced.strip_prefix(':').or_else(|| spaced.strip_prefix('=')) else { continue };
        let value_at = value.trim_start_matches([' ', '\t']);
        let gap = &rest[..rest.len() - value_at.len()];
        out.push_str(gap);
        let (redacted, left) = redact_value(value_at);
        out.push_str(&redacted);
        rest = left;
    }
    out
}

/// The value at the start of `value` as it is printed once redacted, and what follows it.
fn redact_value(value: &str) -> (String, &str) {
    if let Some(inner) = value.strip_prefix("Some(") {
        let (redacted, left) = redact_value(inner);
        return (format!("Some({redacted}"), left);
    }
    match value.chars().next() {
        Some(q @ ('"' | '\'')) => (format!("{q}<redacted>{q}"), &value[quoted_end(value, q)..]),
        Some(_) => ("<redacted>".into(), &value[bare_end(value)..]),
        None => (String::new(), value),
    }
}

/// The length of the string literal at the start of `value` (which starts with `quote`).
fn quoted_end(value: &str, quote: char) -> usize {
    let mut escaped = false;
    for (i, c) in value.char_indices().skip(1) {
        match c {
            '\\' if !escaped => escaped = true,
            c if c == quote && !escaped => return i + 1,
            _ => escaped = false,
        }
    }
    value.len()
}

fn bare_end(value: &str) -> usize {
    value.find(|c: char| matches!(c, ',' | '}' | ']' | ')' | '&') || c.is_whitespace()).unwrap_or(value.len())
}

/// `scheme://user:pass@host` becomes `scheme://<redacted>@host`: the whole userinfo goes when it
/// holds a colon (a password, which may hold a `/`), and stays when it is a user name alone. A
/// `host:port/path` whose path holds an `@` is no userinfo, nor is a `/` before the colon.
fn redact_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let token = &tail[..tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ')' | '>' | ';')).unwrap_or(tail.len())];
        rest = tail;
        let Some(arobase) = token.rfind('@') else { continue };
        let userinfo = &token[..arobase];
        let Some(colon) = userinfo.find(':') else { continue };
        let after_colon = &userinfo[colon + 1..];
        let digits = after_colon.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_colon.len());
        let is_port_then_path = digits > 0 && after_colon[digits..].starts_with(['/', '?', '#']);
        if !userinfo[..colon].contains(['/', '?', '#']) && !is_port_then_path {
            out.push_str("<redacted>");
            rest = &tail[arobase..];
        }
    }
    out.push_str(rest);
    out
}

/// `eyJ…` followed by two more dot-separated base64url parts (and, at the end of a sentence, a full stop).
fn redact_jwts(text: &str) -> String {
    const START: &str = "eyJ";
    let url64 = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(START) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let run = tail.find(|c: char| !(url64(c) || c == '.')).unwrap_or(tail.len());
        let token = tail[..run].strip_suffix('.').unwrap_or(&tail[..run]);
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(url64)) {
            out.push_str("eyJ<redacted>");
            rest = &tail[token.len()..];
        } else {
            out.push_str(START);
            rest = &tail[START.len()..];
        }
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
        // A prefix inside a word is not a key.
        assert_eq!(redact_secrets("disk-usage 80% monkey=banana"), "disk-usage 80% monkey=banana");
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
        // A count of tokens is not a token.
        assert_eq!(redact_secrets(r#"{"max_tokens": 4096}"#), r#"{"max_tokens": 4096}"#);
    }

    /// A Rust `Debug` struct, which is what a panic or an `unwrap` prints (round-3 review, row 1).
    #[test]
    fn a_rust_debug_struct_loses_its_key() {
        let out = redact_secrets(r#"called `Result::unwrap()` on ApiConfig { api_key: "abc123secretvalue", model: "x" }"#);
        assert_eq!(out, r#"called `Result::unwrap()` on ApiConfig { api_key: "<redacted>", model: "x" }"#);
        let out = redact_secrets(r#"LLMConfig { backend: "api", api_key: Some("abc123secretvalue"), api_model: Some("m") }"#);
        assert_eq!(out, r#"LLMConfig { backend: "api", api_key: Some("<redacted>"), api_model: Some("m") }"#);
    }

    /// YAML and env-file echoes, with no quotes anywhere (row 2).
    #[test]
    fn an_unquoted_pair_loses_its_value() {
        assert_eq!(redact_secrets("api_key: abc123secretvalue"), "api_key: <redacted>");
        assert_eq!(redact_secrets("config:\n  api_key: abc123secretvalue\n  model: x\n"), "config:\n  api_key: <redacted>\n  model: x\n");
        assert_eq!(redact_secrets("OPENAI_API_KEY=abc123secretvalue run"), "OPENAI_API_KEY=<redacted> run");
        assert_eq!(redact_secrets("GET /v1?api_key=abc123&n=1"), "GET /v1?api_key=<redacted>&n=1");
        assert_eq!(redact_secrets("model: x\nretries: 3"), "model: x\nretries: 3");
    }

    /// A header or key written with hyphens (row 3).
    #[test]
    fn a_hyphenated_key_is_read_as_underscored() {
        assert_eq!(redact_secrets(r#"{"x-api-key": "abc123secretvalue"}"#), r#"{"x-api-key": "<redacted>"}"#);
        assert_eq!(redact_secrets("x-api-key: abc123secretvalue"), "x-api-key: <redacted>");
        assert_eq!(redact_secrets("client-secret=abc123secretvalue"), "client-secret=<redacted>");
    }

    /// Private keys, passphrases, credentials and auth (row 4).
    #[test]
    fn private_keys_passphrases_credentials_and_auth_go() {
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\n-----END PRIVATE KEY-----";
        assert_eq!(redact_secrets(&format!("key was {pem} done")), "key was <redacted private key> done");
        assert_eq!(redact_secrets(r#"{"private_key": "-----BEGIN PRIVATE KEY-----\nMIIEv\n-----END PRIVATE KEY-----\n"}"#), r#"{"private_key": "<redacted>"}"#);
        assert_eq!(redact_secrets("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk"), "<redacted private key>", "a record cut short");
        assert_eq!(redact_secrets(r#"{"passphrase": "correct horse"}"#), r#"{"passphrase": "<redacted>"}"#);
        assert_eq!(redact_secrets(r#"{"credentials": "abc123secretvalue", "auth": "abc123secretvalue"}"#), r#"{"credentials": "<redacted>", "auth": "<redacted>"}"#);
        assert_eq!(redact_secrets("auth_token: abc123secretvalue"), "auth_token: <redacted>");
        // A certificate is public; an author is not an auth.
        assert_eq!(redact_secrets("-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----"), "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----");
        assert_eq!(redact_secrets("author: Ada"), "author: Ada");
    }

    /// A token at the end of a sentence (row 5).
    #[test]
    fn a_json_web_token_is_redacted_and_a_lookalike_is_not() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let out = redact_secrets(&format!("session {jwt} expired"));
        assert_eq!(out, "session eyJ<redacted> expired");
        assert!(!out.contains("dBjft") && !out.contains("hbGci"));
        assert_eq!(redact_secrets(&format!("invalid token {jwt}.")), "invalid token eyJ<redacted>.");
        assert_eq!(redact_secrets("eyJ and eyJhbGci.only two"), "eyJ and eyJhbGci.only two");
    }

    /// Basic authentication (row 6).
    #[test]
    fn a_basic_authorization_header_loses_its_credential() {
        assert_eq!(redact_secrets("Authorization: Basic dXNlcjpwYXNzd29yZA=="), "Authorization: Basic <redacted>");
        assert_eq!(redact_secrets("basic usage of the tool"), "basic usage of the tool");
    }

    /// Providers whose keys carry a prefix of their own (row 7).
    #[test]
    fn groq_stripe_and_google_oauth_tokens_are_redacted() {
        // The Stripe-shaped keys are assembled here so this file holds no string a secret scanner takes for one.
        let stripe = ["sk_", "live_", "abcdefghij0123456789"].concat();
        let out = redact_secrets(&format!("a gsk_abcdefghijklmnop1234 b {stripe} c rk_live_51Habc d ya29.a0AfH6SMBx-9_k e"));
        assert_eq!(out, "a gsk_<redacted> b sk_live_<redacted> c rk_live_<redacted> d ya29.<redacted> e");
    }

    #[test]
    fn google_hugging_face_and_gitlab_tokens_are_redacted() {
        let out = redact_secrets("keys AIzaSyD-9tSrke72PouQMnMX-a7eZSW0jkFMBWY hf_abcdefghijklmnopqrstuvwxyz0123 glpat-xYz123_AbC-9 end");
        assert_eq!(out, "keys AIza<redacted> hf_<redacted> glpat-<redacted> end");
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

    /// A password that holds a `/` or an `@` (row 8).
    #[test]
    fn a_password_with_a_slash_or_an_at_sign_goes_whole() {
        assert_eq!(redact_secrets("clone https://user:pa/ss@host/repo.git"), "clone https://<redacted>@host/repo.git");
        assert_eq!(redact_secrets("clone https://user:p@ss@host/x"), "clone https://<redacted>@host/x");
        assert_eq!(redact_secrets("https://user:12345@host/"), "https://<redacted>@host/");
        assert_eq!(redact_secrets(r#"urls "https://a:b@h/","https://example.org/x@y""#), r#"urls "https://<redacted>@h/","https://example.org/x@y""#);
    }

    #[test]
    fn the_shapes_work_together_and_unicode_survives() {
        let out = redact_secrets(r#"échec "api_key": "k" at https://a:b@h/ with ghp_1 – fin"#);
        assert_eq!(out, r#"échec "api_key": "<redacted>" at https://<redacted>@h/ with ghp_<redacted> – fin"#);
    }
}
