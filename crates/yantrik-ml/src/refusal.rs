//! A provider that answered and refused the request: what it said, and the parameters it will not
//! take.
//!
//! VM 542, 16 Oct 2026: the person chose Claude in the first run, the key test passed, and every
//! companion turn then came back `400 {"error":{"message":"`temperature` is deprecated for this
//! model."}}`. Current Claude models refuse sampling parameters outright, and the OpenAI-compatible
//! clients sent `temperature` on every request. The turn was served by the offline responder, and
//! the first run said "it is not set up to think yet", which hid the one sentence that said what
//! was wrong.
//!
//! So, for every OpenAI-compatible client in this crate (one module, never copies):
//! - a 400 that names a sampling parameter as one the model will not take ([`refused_params`],
//!   the one matcher, which the gateway calls too) is answered by sending the same request once
//!   more without it, and once that is answered the refusal is remembered per (address, model)
//!   for the life of the process, so later calls (and the key test, which shares the process)
//!   leave it out from the start ([`send_dropping_refused`]);
//! - on Anthropic's own endpoint a sampling value nobody chose (the [`GenerationConfig`] default)
//!   is not sent at all ([`prepare`]): Anthropic then uses the model's own default, which is what
//!   every current Claude model wants, and older ones lose nothing they were asked for;
//! - any other refusal is [`ProviderRefused`], carrying the provider's own one-line reason, so a
//!   caller can tell the person what the provider said instead of falling back to canned words.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::types::GenerationConfig;

/// The request fields a model may refuse to be sent, and that a request can go without: each
/// only tunes sampling, so leaving it out means the provider's default. This is the one place that
/// says what may be dropped, for the companion's clients and the gateway alike. `top_k` and the
/// penalties are here too: the companion sends `frequency_penalty` on every turn by default
/// (`repeat_penalty` 1.1), and OpenAI's reasoning models refuse the penalties outright, so a
/// refusal of one would otherwise fail every turn on such a model.
pub const SAMPLING_PARAMETERS: [&str; 5] = ["temperature", "top_p", "top_k", "frequency_penalty", "presence_penalty"];

/// What a provider says when a model will not take a parameter at all. A range error
/// ("temperature values above 1 are not allowed", "must be at most 1") or a conflict ("`temperature`
/// and `top_p` cannot both be specified") is not one of these: the parameter is allowed, the value
/// or the pair is wrong, and the caller is shown it rather than having the parameter dropped for
/// good.
const REFUSED: [&str; 7] =
    ["is deprecated", "deprecated for this model", "not supported", "unsupported", "does not support", "unrecognized request argument", "unknown parameter"];

/// Anthropic's API host: its OpenAI-compatible endpoint is `https://api.anthropic.com/v1`.
const ANTHROPIC_HOST: &str = "api.anthropic.com";

/// The host (and port) of a URL as errors and refusals name it, never its path, query or
/// `user:pass@`: a path can carry an account id (Cloudflare), a query a key. Read from the one
/// rule for drawing an address (`yantrik_shown_address`), so a key holding a `/`
/// (`https://user:p/w@host/v1`) is "the provider", never "user:p" (review of #856). Not a URL at
/// all is "the provider" too.
pub fn host_of(url: &str) -> String {
    let host = if url.contains("://") { yantrik_shown_address::shown_host(url) } else { String::new() };
    if host.is_empty() { "the provider".to_string() } else { host }
}

/// A request that never got an answer, as it may be said or logged: ureq's own text with every
/// address in it drawn by `yantrik_shown_address`. Its "bad uri" and "configured for https only"
/// errors carry the whole address, sign-in details and `?key=` included.
#[cfg(feature = "api-llm")]
pub fn unanswered(e: ureq::Error) -> anyhow::Error {
    anyhow::anyhow!(yantrik_shown_address::without_sign_in(&e.to_string()))
}

/// Whether `base_url` is Anthropic's own API: a decision, so it reads the address as it is.
pub fn is_anthropic(base_url: &str) -> bool {
    let authority = base_url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or("");
    authority.rsplit('@').next().unwrap_or("").eq_ignore_ascii_case(ANTHROPIC_HOST)
}

/// Text from a provider, made safe to show and to log: only characters that print. Control
/// characters (C0 and C1, so ESC, NUL and BEL), bidirectional overrides and isolates, zero-width
/// characters and the byte-order mark are dropped, and every run of whitespace (newlines too)
/// becomes one space. An endpoint the person configured writes this text; without this a bidi
/// override could make "X refused the request: …" read as something else in the UI, and an
/// escape sequence would reach any terminal or log viewer that shows it raw.
pub fn sanitize(text: &str) -> String {
    let invisible = |c: char| {
        matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
    };
    let printable: String = text
        .chars()
        .filter_map(|c| if c.is_whitespace() { Some(' ') } else if c.is_control() || invisible(c) { None } else { Some(c) })
        .collect();
    printable.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A provider's error body, short enough to say what went wrong and safe to show: cleaned
/// ([`sanitize`]), the key masked, then cut, so a key echoed across the cut cannot leave its head
/// behind.
pub fn excerpt(body: &str, key: Option<&str>) -> String {
    let clean = sanitize(body);
    let masked = match key.filter(|k| k.len() >= 8) {
        Some(k) => clean.replace(k, "<key>"),
        None => clean,
    };
    masked.chars().take(200).collect()
}

/// The provider's own words in an error body, as it wrote them: `error.message` (OpenAI's shape,
/// Anthropic's in both its APIs, llama.cpp's), a bare `error` string, or a top-level `message`.
fn raw_message(body: &[u8]) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = json["error"]["message"]
        .as_str()
        .or_else(|| json["error"].as_str())
        .or_else(|| json["message"].as_str())?;
    Some(message.to_string())
}

/// The provider's own words from an error body ([`raw_message`]), cleaned ([`sanitize`]): one line.
pub fn provider_message(body: &str) -> Option<String> {
    let line = sanitize(&raw_message(body.as_bytes())?);
    (!line.is_empty()).then_some(line)
}

/// Whether `text` holds `word` with no identifier character either side (`top_p` is not in
/// `top_probability` or `top_p2`).
fn names(text: &str, word: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

/// The [`SAMPLING_PARAMETERS`] a provider's message says the model will not take. A parameter
/// counts only when it is named in the same sentence as the refusal: "`temperature` and `top_p`
/// cannot both be specified. `top_p` is deprecated." names both, and refuses one.
pub fn refused_in(message: &str) -> Vec<&'static str> {
    let message = message.to_ascii_lowercase();
    let mut found = Vec::new();
    for sentence in message.split(['.', ';', '\n']) {
        if !REFUSED.iter().any(|w| sentence.contains(w)) {
            continue;
        }
        for p in SAMPLING_PARAMETERS {
            if names(sentence, p) && !found.contains(&p) {
                found.push(p);
            }
        }
    }
    found
}

/// The parameters a 400's body says the model will not take, of those `sent` carries. The one
/// matcher: the companion's clients ([`send_dropping_refused`]) and the gateway both call it.
///
/// Conservative on purpose: only the provider's own message (`error.message`, a bare `error`, or a
/// top-level `message`), only a parameter the request really sent, and only when the message says
/// it is not taken at all ([`refused_in`]). Providers echo an unknown key's name in their 400, so
/// when `sent` has another key holding the parameter's name (`"x temperature"`), the message may be
/// that echo, and the parameter is not taken as refused. The caller checks the status is 400.
pub fn refused_params(error_body: &[u8], sent: &serde_json::Value) -> Vec<&'static str> {
    let Some(sent) = sent.as_object() else { return Vec::new() };
    let Some(message) = raw_message(error_body) else { return Vec::new() };
    refused_in(&message)
        .into_iter()
        .filter(|p| sent.contains_key(*p) && !sent.keys().any(|k| k != p && k.to_ascii_lowercase().contains(p)))
        .collect()
}

type Refusals = HashMap<(String, String), Vec<&'static str>>;

fn refusals() -> &'static Mutex<Refusals> {
    static REFUSED: OnceLock<Mutex<Refusals>> = OnceLock::new();
    REFUSED.get_or_init(Default::default)
}

fn key(base_url: &str, model: &str) -> (String, String) {
    (base_url.trim().trim_end_matches('/').to_ascii_lowercase(), model.to_string())
}

/// The most (address, model) pairs remembered. Entries only grow with the pairs the person
/// configures or key-tests, so this is never reached in use; past it a new pair is still retried
/// on each call, just not remembered.
const MOST_REMEMBERED: usize = 64;

/// Remember for this process that `model` at `base_url` will not take `parameter`.
pub fn remember(base_url: &str, model: &str, parameter: &'static str) {
    insert(&mut refusals().lock().unwrap_or_else(|e| e.into_inner()), key(base_url, model), parameter);
}

fn insert(all: &mut Refusals, key: (String, String), parameter: &'static str) {
    if !all.contains_key(&key) && all.len() >= MOST_REMEMBERED {
        return;
    }
    let known = all.entry(key).or_default();
    if !known.contains(&parameter) {
        known.push(parameter);
    }
}

/// What `model` at `base_url` is known to refuse.
pub fn remembered(base_url: &str, model: &str) -> Vec<&'static str> {
    refusals().lock().unwrap_or_else(|e| e.into_inner()).get(&key(base_url, model)).cloned().unwrap_or_default()
}

/// Make a built chat-completions body fit what this provider takes: leave out what the model is
/// known to refuse, and on Anthropic's endpoint the sampling values nobody chose. Every
/// OpenAI-compatible body builder ends with this, the key test included, so all send one shape.
pub fn prepare(base_url: &str, model: &str, config: &GenerationConfig, body: &mut serde_json::Value) {
    let Some(fields) = body.as_object_mut() else { return };
    for parameter in remembered(base_url, model) {
        fields.remove(parameter);
    }
    if is_anthropic(base_url) {
        let unchosen = GenerationConfig::default();
        if config.temperature == unchosen.temperature {
            fields.remove("temperature");
        }
        if config.top_p == unchosen.top_p {
            fields.remove("top_p");
        }
    }
}

/// Send `body` with `post`, and when the provider refuses sampling parameters it names
/// ([`refused_params`]), send it once more without them; once that is answered, the refusal is
/// remembered for every later call.
///
/// `post` sends one request: `Ok(Ok(_))` is an answer, `Ok(Err((status, body)))` the provider's
/// refusal as it said it, `Err` a failure on the way. Each parameter is dropped at most once, so
/// this sends at most one request more than there are [`SAMPLING_PARAMETERS`].
pub fn send_dropping_refused<T>(
    base_url: &str,
    model: &str,
    body: &serde_json::Value,
    mut post: impl FnMut(&serde_json::Value) -> anyhow::Result<Result<T, (u16, String)>>,
) -> anyhow::Result<Result<T, (u16, String)>> {
    let mut body = body.clone();
    if let Some(fields) = body.as_object_mut() {
        for parameter in remembered(base_url, model) {
            fields.remove(parameter);
        }
    }
    // Remembered only once a request without them is answered: the error text can echo what a
    // caller sent, and an endpoint can answer the matching 400 and then fail the retry, so a 400
    // alone is not the provider's word (as in the gateway since 91f80cd).
    let mut dropped: Vec<&'static str> = Vec::new();
    loop {
        let (status, text) = match post(&body)? {
            Ok(answer) => {
                for parameter in dropped {
                    remember(base_url, model, parameter);
                }
                return Ok(Ok(answer));
            }
            Err(refusal) => refusal,
        };
        let refused = if status == 400 { refused_params(text.as_bytes(), &body) } else { Vec::new() };
        if refused.is_empty() {
            return Ok(Err((status, text)));
        }
        tracing::warn!(
            host = %host_of(base_url),
            model,
            parameters = ?refused,
            "the model refused sampling parameters (400); asking once more without them, and not sending them again"
        );
        if let Some(fields) = body.as_object_mut() {
            for parameter in refused {
                fields.remove(parameter);
                dropped.push(parameter);
            }
        }
    }
}

/// The provider answered and refused the request (a 4xx that is not a timeout or a rate limit):
/// the model was reached, so this is not "offline", and its reason is what the person should read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRefused {
    pub status: u16,
    /// Who refused, as the person knows them: "Claude" for Anthropic, else the host.
    pub provider: String,
    /// The provider's own one-line reason, or the status when it gave none.
    pub reason: String,
}

impl ProviderRefused {
    /// A refusal from `base_url`, if `status` is one: a 4xx other than 408 (timeout) and 429
    /// (rate limit), which say nothing about the request itself.
    pub fn of(status: u16, base_url: &str, body: &str, api_key: Option<&str>) -> Option<ProviderRefused> {
        if !(400..500).contains(&status) || status == 408 || status == 429 {
            return None;
        }
        let provider = if is_anthropic(base_url) { "Claude".to_string() } else { host_of(base_url) };
        let reason = provider_message(body)
            .map(|m| excerpt(&m, api_key).trim_end_matches('.').to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| format!("HTTP {status}"));
        Some(ProviderRefused { status, provider, reason })
    }

    /// The sentence the person reads in the chat and in the first run's check.
    pub fn said(&self) -> String {
        format!("{self}. Try again, or choose another model.")
    }
}

impl std::fmt::Display for ProviderRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} refused the request: {}", self.provider, self.reason)
    }
}

impl std::error::Error for ProviderRefused {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// api.anthropic.com/v1/chat/completions on VM 542, 16 Oct 2026, as the error log line showed it
    /// (`code` and `message` as logged; the log cut the rest).
    const TEMPERATURE_DEPRECATED: &str = include_str!("llm/testdata/anthropic_temperature_deprecated.json");

    fn body() -> serde_json::Value {
        serde_json::json!({"model": "claude-sonnet-5-5", "messages": [], "max_tokens": 8, "temperature": 0.1, "top_p": 0.9, "stream": true})
    }

    #[test]
    fn the_recorded_refusal_names_temperature() {
        assert_eq!(refused_params(TEMPERATURE_DEPRECATED.as_bytes(), &body()), vec!["temperature"]);
        assert_eq!(provider_message(TEMPERATURE_DEPRECATED).as_deref(), Some("`temperature` is deprecated for this model."));
    }

    /// The security review's 16 inputs (PR #827), and what the one matcher answers for each. The
    /// gateway runs the same table through `body::refused_params`, so the two callers cannot drift.
    const TABLE: &str = include_str!("llm/testdata/refusal_table.json");

    #[test]
    fn the_one_matcher_answers_the_review_table() {
        let table: Vec<(String, Vec<String>)> = serde_json::from_str(TABLE).unwrap();
        assert_eq!(table.len(), 16);
        let sent = serde_json::json!({"model": "m", "messages": [], "temperature": 0.7, "top_p": 0.9, "top_k": 40, "frequency_penalty": 0.1});
        for (message, want) in table {
            let body = serde_json::json!({"error": {"message": message}}).to_string();
            assert_eq!(refused_params(body.as_bytes(), &sent), want, "{message}");
        }
    }

    #[test]
    fn other_refusals_are_not_taken_for_a_parameter() {
        let sent = body();
        let refused = |b: &str| refused_params(b.as_bytes(), &sent);
        // A 400 about something else; a value out of range; a parameter not sent; not JSON; a
        // longer name.
        assert!(refused(r#"{"error":{"message":"messages: at least one message is required"}}"#).is_empty());
        assert!(refused(r#"{"error":{"message":"temperature: must be between 0 and 1"}}"#).is_empty());
        assert!(refused(r#"{"error":{"message":"`top_k` is not supported"}}"#).is_empty());
        assert!(refused("not json at all: temperature is deprecated").is_empty());
        assert!(refused(r#"{"error":{"message":"top_probability is deprecated"}}"#).is_empty());
        // The other one the same models refuse, once temperature is gone.
        assert_eq!(refused(r#"{"error":{"message":"`top_p` is not supported for this model."}}"#), vec!["top_p"]);
    }

    #[test]
    fn a_refusal_that_may_echo_the_callers_own_key_drops_nothing() {
        let echoed = |key: &str| serde_json::json!({"error": {"message": format!("Unrecognized request argument supplied: {key}")}}).to_string();
        for key in ["x temperature", "x. temperature is not supported", "my_temperature"] {
            let sent = serde_json::json!({"model": "x", "temperature": 0.7, key: 1});
            assert!(refused_params(echoed(key).as_bytes(), &sent).is_empty(), "{key}");
        }
    }

    #[test]
    fn a_400_that_is_not_about_a_parameter_is_not_retried() {
        let base = "https://refusal-test-4.example/v1";
        let mut calls = 0;
        let reply = send_dropping_refused(base, "m", &body(), |_| {
            calls += 1;
            Ok(Err::<(), _>((422, TEMPERATURE_DEPRECATED.to_string())))
        })
        .unwrap();
        assert!(reply.is_err(), "only a 400 is read for a parameter");
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_refused_parameter_is_dropped_retried_once_and_remembered() {
        let base = "https://refusal-test-1.example/v1";
        let sent = RefCell::new(Vec::new());
        let reply = send_dropping_refused(base, "m", &body(), |b| {
            sent.borrow_mut().push(b.clone());
            Ok(if b.get("temperature").is_some() { Err((400, TEMPERATURE_DEPRECATED.to_string())) } else { Ok("answer") })
        })
        .unwrap();
        assert_eq!(reply, Ok("answer"));
        let sent = sent.into_inner();
        assert_eq!(sent.len(), 2, "the refusal, then one retry");
        assert!(sent[0].get("temperature").is_some());
        assert!(sent[1].get("temperature").is_none(), "the retry goes without it");
        assert_eq!(sent[1]["top_p"], 0.9, "and keeps everything else");
        assert_eq!(remembered(base, "m"), vec!["temperature"]);
        assert!(remembered(base, "another-model").is_empty(), "per model");

        // The next call leaves it out from the start: one request, no refusal.
        let mut calls = 0;
        send_dropping_refused(base, "m", &body(), |b| {
            calls += 1;
            assert!(b.get("temperature").is_none());
            Ok(Ok::<_, (u16, String)>(()))
        })
        .unwrap()
        .unwrap();
        assert_eq!(calls, 1);
    }

    #[test]
    fn any_other_400_is_returned_as_it_came_and_not_retried() {
        let base = "https://refusal-test-2.example/v1";
        let mut calls = 0;
        let reply = send_dropping_refused(base, "m", &body(), |_| {
            calls += 1;
            Ok(Err::<(), _>((400, r#"{"error":{"message":"bad message"}}"#.to_string())))
        })
        .unwrap();
        assert_eq!(reply, Err((400, r#"{"error":{"message":"bad message"}}"#.to_string())));
        assert_eq!(calls, 1);
        assert!(remembered(base, "m").is_empty());
    }

    #[test]
    fn a_refusal_is_remembered_only_once_the_retry_without_it_is_answered() {
        let base = "https://refusal-test-5.example/v1";
        let mut calls = 0;
        let reply = send_dropping_refused(base, "m", &body(), |b| {
            calls += 1;
            Ok(if b.get("temperature").is_some() { Err((400, TEMPERATURE_DEPRECATED.to_string())) } else { Err::<(), _>((500, "down".to_string())) })
        })
        .unwrap();
        assert_eq!(reply, Err((500, "down".to_string())));
        assert_eq!(calls, 2, "the refusal, then one retry");
        assert!(remembered(base, "m").is_empty(), "a retry that failed proves nothing");

        let failed_on_the_way = send_dropping_refused(base, "m", &body(), |b| {
            if b.get("temperature").is_some() { Ok(Err::<(), _>((400, TEMPERATURE_DEPRECATED.to_string()))) } else { anyhow::bail!("reset") }
        });
        assert!(failed_on_the_way.is_err());
        assert!(remembered(base, "m").is_empty());
    }

    #[test]
    fn past_the_cap_a_new_pair_is_not_remembered_and_a_known_one_still_is() {
        let mut all = Refusals::new();
        for i in 0..MOST_REMEMBERED {
            insert(&mut all, key(&format!("https://cap-{i}.example/v1"), "m"), "temperature");
        }
        insert(&mut all, key("https://cap-over.example/v1", "m"), "temperature");
        assert_eq!(all.len(), MOST_REMEMBERED);
        assert!(!all.contains_key(&key("https://cap-over.example/v1", "m")));
        insert(&mut all, key("https://cap-0.example/v1", "m"), "top_p");
        assert_eq!(all[&key("https://cap-0.example/v1", "m")], vec!["temperature", "top_p"], "a pair already known keeps learning");
    }

    #[test]
    fn each_parameter_is_dropped_at_most_once() {
        let base = "https://refusal-test-3.example/v1";
        let mut calls = 0;
        let reply = send_dropping_refused(base, "m", &body(), |b| {
            calls += 1;
            let named = if b.get("temperature").is_some() { "temperature" } else { "top_p" };
            Ok(Err::<(), _>((400, format!(r#"{{"error":{{"message":"`{named}` is deprecated for this model."}}}}"#))))
        })
        .unwrap();
        assert!(reply.is_err(), "a request with nothing left to drop is the caller's refusal");
        assert_eq!(calls, 3, "with both, without temperature, without top_p");
    }

    #[test]
    fn on_anthropics_endpoint_sampling_nobody_chose_is_not_sent() {
        let unchosen = GenerationConfig::default();
        let mut sent = serde_json::json!({"temperature": unchosen.temperature, "top_p": unchosen.top_p});
        prepare("https://api.anthropic.com/v1", "claude-haiku-4-5", &unchosen, &mut sent);
        assert_eq!(sent, serde_json::json!({}));

        // A value someone chose is still sent (an older Claude takes it; a newer one is retried).
        let chosen = GenerationConfig { temperature: 0.1, ..Default::default() };
        let mut sent = serde_json::json!({"temperature": 0.1, "top_p": unchosen.top_p});
        prepare("https://api.anthropic.com/v1", "claude-haiku-4-5", &chosen, &mut sent);
        assert_eq!(sent, serde_json::json!({"temperature": 0.1}));

        // Anywhere else nothing changes.
        let mut sent = serde_json::json!({"temperature": unchosen.temperature});
        prepare("https://api.openai.com/v1", "gpt-4o", &unchosen, &mut sent);
        assert_eq!(sent["temperature"], unchosen.temperature);
    }

    #[test]
    fn provider_text_keeps_only_what_prints() {
        // A bidi override that would turn the sentence around, a clear-screen escape, a bell, a
        // NUL, zero-width characters, a BOM, an isolate and a C1 control, with newlines and tabs.
        let said = "\u{202e}txet\x1b[2J\x07 is\0 de\u{200b}pre\u{feff}cated\n\tfor\u{2066} this\u{9b} model\u{2069}.";
        assert_eq!(sanitize(said), "txet[2J is deprecated for this model.");
        assert_eq!(excerpt(said, None), "txet[2J is deprecated for this model.");

        let body = serde_json::json!({"error": {"message": said}}).to_string();
        assert_eq!(provider_message(&body).as_deref(), Some("txet[2J is deprecated for this model."));
        let r = ProviderRefused::of(400, "https://api.anthropic.com/v1", &body, None).unwrap();
        assert_eq!(r.to_string(), "Claude refused the request: txet[2J is deprecated for this model");
        assert!(!r.said().chars().any(|c| c.is_control() || ('\u{2000}'..='\u{20ff}').contains(&c)), "{:?}", r.said());

        // The key is masked after cleaning, so a key split by an invisible character is still masked.
        let key = "sk-0123456789abcdef";
        assert_eq!(excerpt("bad key sk-01234\u{200b}56789abcdef", Some(key)), "bad key <key>");
    }

    #[test]
    fn a_refusal_says_who_refused_and_their_reason_in_one_line() {
        let r = ProviderRefused::of(400, "https://api.anthropic.com/v1", TEMPERATURE_DEPRECATED, None).unwrap();
        assert_eq!(r.to_string(), "Claude refused the request: `temperature` is deprecated for this model");
        assert_eq!(r.said(), "Claude refused the request: `temperature` is deprecated for this model. Try again, or choose another model.");

        let key = "sk-0123456789abcdef";
        let r = ProviderRefused::of(401, "https://api.groq.com/openai/v1", &format!(r#"{{"error":{{"message":"Invalid API key {key}\nprovided"}}}}"#), Some(key)).unwrap();
        assert_eq!(r.to_string(), "api.groq.com refused the request: Invalid API key <key> provided");
        assert_eq!(ProviderRefused::of(404, "http://10.0.0.2:8080/v1", "", None).unwrap().reason, "HTTP 404");
        for not_a_refusal in [200, 408, 429, 500, 503] {
            assert!(ProviderRefused::of(not_a_refusal, "https://api.anthropic.com/v1", TEMPERATURE_DEPRECATED, None).is_none());
        }
    }

    /// Review of #856: the refusal sentence the person reads in the chat, and every error and log
    /// line that names a host, never names a part of the sign-in details.
    #[test]
    fn a_refusal_never_names_a_part_of_the_sign_in_details() {
        let said = |url: &str| ProviderRefused::of(401, url, "", None).unwrap().said();
        assert!(said("https://user:p/w@aig.example/v1").starts_with("the provider refused the request"), "{}", said("https://user:p/w@aig.example/v1"));
        assert!(said("https://user:pw@aig.example/v1?api-key=x#f").starts_with("aig.example refused"));
        assert!(said("http://u@127.0.0.1:11434").starts_with("127.0.0.1:11434 refused"));
        assert!(said("http://u:p@[::1]:4000/v1").starts_with("[::1]:4000 refused"));
        for url in [
            "https://user:p/w@aig.example/v1",
            "https://user:pw@aig.example/v1?api-key=x#f",
            "http://u@127.0.0.1:11434",
            "http://u:p@[::1]:4000/v1",
            "user:pw@aig.example",
            "user%40x:pw@aig.example",
            "https://aig.example/a@b",
            "aig.example/v1?api-key=x",
        ] {
            for shown in [said(url), host_of(url)] {
                for secret in ["user", "pw", "p/w", "api-key", "u@", "u:p", "%40", "#f"] {
                    assert!(!shown.contains(secret), "{url} names {secret:?}: {shown}");
                }
            }
        }
        // Deciding is not drawing: Anthropic is still told by the address as it is.
        assert!(is_anthropic("https://key@api.anthropic.com/v1"));
        assert!(!is_anthropic("https://api.anthropic.com@evil.example/v1"));
    }

    #[cfg(feature = "api-llm")]
    #[test]
    fn a_request_that_never_got_an_answer_says_no_sign_in_or_key() {
        let https_only = unanswered(ureq::Error::RequireHttpsOnly("http://user:pw@aig.example/v1/models?key=k".into()));
        assert_eq!(https_only.to_string(), "configured for https only: http://aig.example/v1/models");
        let bad = unanswered(ureq::Error::BadUri("user:pw@aig.example/v1 is missing scheme".into()));
        assert_eq!(bad.to_string(), "bad uri: aig.example/v1 is missing scheme");
    }
}
