//! Which provider a URL or a mind's own words name: one answer, in one spelling.
//!
//! On VM 520 one provider showed up four ways ("ollama.com", "ollama-cloud:…", "ollamacloud/…",
//! "ollama-cloud/…"), because each mind writes its own `detail` line. Everything that names a
//! provider comes through here and is called by the catalogue's display name.
//!
//! The one rule that matters most: **a provider is never inferred from a model name.**
//! "deepseek-v4.1-flash" contains the catalogue id `deepseek`, and reading it that way would say
//! DeepSeek pays for minds that run on Ollama Cloud. A provider is named only by a URL, a host, or
//! an explicit `provider:model` / `provider/model` prefix whose left side is a catalogue id.

use yantrik_ml::{ProviderDescriptor, ProviderKind, KNOWN_PROVIDERS};

/// A provider, as far as the facts go.
#[derive(Clone, Debug)]
pub enum ProviderRef {
    /// One the catalogue knows.
    Known(&'static ProviderDescriptor),
    /// An address the catalogue does not know, by its host.
    Custom(String),
    /// An address on this machine or the local network, by its host.
    Local(String),
    /// Nothing said which provider it is.
    NotReported,
}

/// Two refs are the same provider: catalogue entries by id, addresses by host.
impl PartialEq for ProviderRef {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ProviderRef::Known(a), ProviderRef::Known(b)) => a.id == b.id,
            (ProviderRef::Custom(a), ProviderRef::Custom(b)) | (ProviderRef::Local(a), ProviderRef::Local(b)) => a == b,
            (ProviderRef::NotReported, ProviderRef::NotReported) => true,
            _ => false,
        }
    }
}

impl ProviderRef {
    /// How it is shown, everywhere: "Ollama Cloud", "Custom endpoint · aig.mycluster.cyou".
    pub fn label(&self) -> String {
        match self {
            ProviderRef::Known(p) => p.display_name.to_string(),
            ProviderRef::Custom(host) => format!("Custom endpoint · {host}"),
            ProviderRef::Local(host) => format!("This network · {host}"),
            ProviderRef::NotReported => "provider not reported".to_string(),
        }
    }

    /// The same provider, for counting: two refs are one provider when this is equal.
    pub fn key(&self) -> Option<String> {
        match self {
            ProviderRef::Known(p) => Some(p.id.to_string()),
            ProviderRef::Custom(h) | ProviderRef::Local(h) => Some(h.to_ascii_lowercase()),
            ProviderRef::NotReported => None,
        }
    }
}

/// The host (and port) of a URL, without scheme, userinfo or path.
pub fn host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    authority.rsplit('@').next().unwrap_or(authority).to_string()
}

/// The host without its port: "[::1]:11434" → "::1", "10.0.0.2:8000" → "10.0.0.2".
fn host_name(h: &str) -> &str {
    if let Some(inside) = h.strip_prefix('[') {
        return inside.split(']').next().unwrap_or("");
    }
    h.rsplit_once(':').filter(|(n, _)| !n.contains(':')).map_or(h, |(n, _)| n)
}

/// Whether an address is this machine or the local network: loopback, the private ranges
/// (10/8, 172.16/12, 192.168/16), link-local, or a name that says so (`localhost`, `*.local`).
/// Parsed, not matched by prefix: `10.example.com` is somebody else's host. This decides whether
/// a key may cross plain http (provider_handoff), so it says "local" only when that is certain.
pub fn is_local(url_or_host: &str) -> bool {
    let h = host(url_or_host).to_ascii_lowercase();
    let name = host_name(&h);
    match name.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => name == "localhost" || name.ends_with(".local"),
    }
}

/// Whether an address is this machine itself: loopback or `localhost`, not merely the local
/// network. What the composer's "on this machine" rests on.
pub fn is_loopback(url_or_host: &str) -> bool {
    let h = host(url_or_host).to_ascii_lowercase();
    let name = host_name(&h);
    match name.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => name == "localhost",
    }
}

/// The provider a URL (or a bare host) points at.
pub fn from_url(url: &str) -> ProviderRef {
    let h = host(url).to_ascii_lowercase();
    if h.is_empty() {
        return ProviderRef::NotReported;
    }
    if let Some(p) = by_host(&h) {
        return ProviderRef::Known(p);
    }
    if is_local(&h) {
        ProviderRef::Local(h)
    } else {
        ProviderRef::Custom(h)
    }
}

/// What a mind's own line (`Attach.detail`) says it runs on: the provider, and the model when one
/// can be told apart. From VM 520:
///   "ollama-cloud:deepseek-v4.1-flash"            → Ollama Cloud, deepseek-v4.1-flash
///   "deepseek-v4.1-flash · ollama.com"            → Ollama Cloud, deepseek-v4.1-flash
///   "ollamacloud/deepseek-v4.1-flash · pi 0.87.0" → Ollama Cloud, deepseek-v4.1-flash
///   "Hermes 0.14.0 · deepseek-v4.1-flash"         → not reported, deepseek-v4.1-flash
pub fn from_reported(text: &str) -> (ProviderRef, Option<String>) {
    let segments: Vec<&str> = text.split('\u{b7}').map(str::trim).filter(|s| !s.is_empty()).collect();
    let mut provider = ProviderRef::NotReported;
    let mut model = None;
    'found: for seg in &segments {
        for word in seg.split_whitespace() {
            if word.contains("://") {
                provider = from_url(word);
                break 'found;
            }
            if let Some((p, right)) = provider_prefix(word) {
                provider = p;
                model = Some(right.to_string());
                break 'found;
            }
            if is_host(word) {
                provider = from_url(word);
                break 'found;
            }
        }
    }
    if model.is_none() {
        // A segment that is one word, and neither an address, a version, nor a provider prefix,
        // is a model name: "deepseek-v4.1-flash", "qwen3.5:9b".
        model = segments
            .iter()
            .filter(|s| !s.contains(char::is_whitespace))
            .find(|s| !is_host(s) && !s.contains("://") && !is_version(s) && provider_prefix(s).is_none())
            .map(|s| s.to_string());
    }
    (provider, model)
}

/// `provider:model` or `provider/model`, where the left side really names a provider: a catalogue
/// id as a mind may spell it, or a host. "qwen3.5:9b" is a model with a tag and
/// "192.168.4.35:11434" an address with a port; neither is a prefix.
fn provider_prefix(word: &str) -> Option<(ProviderRef, &str)> {
    let i = word.find([':', '/'])?;
    let (left, right) = (&word[..i], &word[i + 1..]);
    if left.is_empty() || right.is_empty() {
        return None;
    }
    if word[i..].starts_with(':') && right.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if let Some(p) = by_alias(left) {
        return Some((ProviderRef::Known(p), right));
    }
    is_host(left).then(|| (from_url(left), right))
}

/// A host name: dotted, ending in an alphabetic label of two or more letters ("ollama.com"), or an
/// IP address, with or without a port. "deepseek-v4.1-flash", "qwen3.8" and "2026.9.1" are not.
fn is_host(word: &str) -> bool {
    let h = host(word);
    let name = host_name(&h);
    if name.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let last = name.rsplit('.').next().unwrap_or("");
    name.contains('.')
        && last.len() >= 2
        && last.chars().all(|c| c.is_ascii_alphabetic())
        && name.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

fn is_version(word: &str) -> bool {
    let w = word.trim_start_matches('v');
    !w.is_empty() && w.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// A catalogue entry by an id as a mind may spell it: `ollama-cloud`, `ollamacloud`, `ollama_cloud`.
fn by_alias(word: &str) -> Option<&'static ProviderDescriptor> {
    let squash = |s: &str| s.to_ascii_lowercase().replace(['-', '_'], "");
    let want = squash(word);
    KNOWN_PROVIDERS.iter().find(|p| !p.id.is_empty() && squash(p.id) == want)
}

/// A catalogue entry whose address is on `host`. A local runtime's default (localhost:11434)
/// matches only that exact host and port, so another machine's address is never called "Ollama"
/// on a port number's say-so.
fn by_host(host: &str) -> Option<&'static ProviderDescriptor> {
    KNOWN_PROVIDERS
        .iter()
        .filter(|p| !p.default_base_url.is_empty() && !p.default_base_url.contains('{'))
        .find(|p| {
            let theirs = self::host(p.default_base_url).to_ascii_lowercase();
            if p.kind == ProviderKind::Local {
                theirs == host
            } else {
                theirs == host || host_name(&theirs) == host_name(host)
            }
        })
}
