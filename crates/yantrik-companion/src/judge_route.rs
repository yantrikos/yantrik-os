//! Tool routing by a judge (a System One model: Jev, Kev, ...).
//!
//! Without a judge, the chat model is shown a shortlist of tool schemas, as many as its tier
//! allows, and has to pick one itself. With a judge, the tools closest in meaning to the request
//! are put to it as a choice question; when it is sure of one tool, the chat model is shown that
//! tool and `discover_tools` alone (which injects the schema of any other tool the model finds it
//! needs). That saves the tokens of every schema left out, the always-on ones included, and takes
//! the pick away from a model that may get it wrong. A request
//! the judge thinks needs several tools keeps the ordinary shortlist, with the judge's pick in it.
//! When the judge is unsure, slow, or unreachable, selection is exactly what it was without one.

use std::time::Instant;

use serde_json::json;
use yantrik_ml::judge::{Answer, Judge, Question};
use yantrik_ml::{ChatMessage, ModelCapabilityProfile};
use yantrikdb_core::YantrikDB;

use crate::companion::{select_tools_adaptive, ALWAYS_TOOLS, CORE_TOOLS};
use crate::config::JudgeConfig;
use yantrik_companion_core::judge_config::JudgeKind;
use crate::tool_cache::ToolCache;

/// The option offered beside the tools, for a request that needs none.
pub const NO_TOOL: &str = "no_tool";

/// Always shown beside a routed pick: the way to any tool the pick turns out not to cover.
const DISCOVER: &str = "discover_tools";

/// What the judge said about one request.
#[derive(Debug, Clone, PartialEq)]
pub struct Routing {
    /// The tool it picked, or `None` for no tool.
    pub pick: Option<String>,
    /// The probability it gave that pick.
    pub p: f64,
    /// The probability that the request needs several different tools.
    pub multi_step: f64,
}

/// What the companion does with it.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Show the chat model this tool and `discover_tools`.
    Only(String),
    /// Show only the always-on tools.
    NoTool,
    /// Several tools are needed: the ordinary shortlist, with this one in it.
    Widen(String),
    /// Not sure enough: the ordinary shortlist.
    Fallback,
}

/// The policy, kept in code: follow the judge only when it is at least `route_at` sure.
pub fn decide(r: &Routing, route_at: f64) -> Decision {
    if r.p < route_at {
        return Decision::Fallback;
    }
    match (&r.pick, r.multi_step >= 0.5) {
        (None, _) => Decision::NoTool,
        (Some(t), true) => Decision::Widen(t.clone()),
        (Some(t), false) => Decision::Only(t.clone()),
    }
}

/// Where the chat model runs, for a `chat_model` judge's locality: its address, the provider's
/// default one, or, with none, this machine only for the backends that run in this process.
/// Anything else without an address (the Claude CLI) is the cloud: saying what was judged stayed
/// home when it did not is the mistake that matters.
///
/// With a fallback configured, the farther of the two: when the primary is down, a chat-model
/// judge's questions go to the fallback (security review, 29 Sep 2026).
pub fn chat_locality(llm: &yantrik_companion_core::config::LLMConfig) -> yantrik_ml::judge::Locality {
    use yantrik_ml::judge::Locality;
    let of = |url: Option<String>, backend: &str| match url {
        Some(url) => Locality::of_endpoint(&url),
        None if matches!(backend, "candle" | "llamacpp") => Locality::ThisMachine,
        None => Locality::Cloud,
    };
    let primary = of(llm.resolve_api_base_url(), &llm.backend);
    let fallback = llm.fallback.as_ref().map(|f| of(f.api_base_url.clone().filter(|u| !u.trim().is_empty()), &f.backend));
    let far = |l: Locality| match l {
        Locality::Nowhere | Locality::ThisMachine => 0,
        Locality::Home => 1,
        Locality::Cloud => 2,
    };
    match fallback {
        Some(f) if far(f) > far(primary) => f,
        _ => primary,
    }
}

/// The decision model a configuration names, built: a System One server, the chat model, or
/// none. `llm` is the companion's chat model, for `chat_model`; `chat_at` is where it runs
/// (`chat_locality`), for the verdict's locality.
pub fn build_judge(
    config: &JudgeConfig,
    llm: &std::sync::Arc<dyn yantrik_ml::LLMBackend>,
    chat_at: yantrik_ml::judge::Locality,
) -> Option<std::sync::Arc<dyn Judge>> {
    match config.kind() {
        JudgeKind::Off => None,
        JudgeKind::SystemOne(dialect) => Some(std::sync::Arc::new(
            yantrik_ml::judge::SystemOneJudge::new(
                &config.endpoint,
                &config.model,
                Some(config.api_key_env.as_str()),
                std::time::Duration::from_millis(config.timeout_ms),
            )
            .with_dialect(yantrik_ml::judge::Dialect::named(dialect)),
        )),
        JudgeKind::ChatModel => Some(std::sync::Arc::new(yantrik_ml::judge::ChatJudge::new(llm.clone(), chat_at))),
    }
}

/// Put the request to the judge: which of `shortlist` (similarity, name, compact card) does it
/// need, and does it need more than one tool? The person's last few messages go with it, so a
/// reply like "yes, send it" is read in its context.
///
/// Only what the person typed is sent, never the assistant's replies: those can carry a vault
/// value, an email body or a web page, and the judge may be a cloud service. Anything shaped like
/// a credential in what is sent is redacted; picking `vault_store` does not need the token.
pub fn ask(judge: &dyn Judge, request: &str, earlier: &[ChatMessage], shortlist: &[(f32, String, String)]) -> Result<Routing, String> {
    let mut options: Vec<(String, String)> = shortlist.iter().map(|(_, name, card)| (name.clone(), card.clone())).collect();
    options.push((NO_TOOL.into(), "No tool: the request is conversation, or is answered from what is already known".into()));
    let mut earlier: Vec<String> = earlier
        .iter()
        .rev()
        .filter(|m| m.role == "user")
        .take(3)
        .map(|m| redact(&m.content.chars().take(600).collect::<String>()))
        .collect();
    earlier.reverse();
    let state = json!({"request": redact(request), "persons_earlier_messages": earlier});
    let answers = judge.ask(&state, &[
        ("tool", Question::Choice {
            instructions: "Which tool should the assistant use first to carry out `request`?".into(),
            options,
        }),
        ("multi_step", Question::Noul {
            instructions: "Does carrying out `request` need two or more different tools, one after another?".into(),
        }),
    ]).map_err(|e| format!("{e:#}"))?;
    let (pick, p) = match answers.get("tool") {
        Some(a @ Answer::Choice { choice, .. }) => {
            let p = a.picked_probability().unwrap_or(0.0);
            ((choice != NO_TOOL).then(|| choice.clone()), p)
        }
        _ => return Err("the judge gave no tool answer".into()),
    };
    let multi_step = match answers.get("multi_step") {
        Some(Answer::Noul(p)) => *p,
        _ => return Err("the judge gave no multi_step answer".into()),
    };
    Ok(Routing { pick, p, multi_step })
}

/// Credential-shaped text replaced by `<redacted>`, the prefix that names it kept. The same
/// shapes `yantrik_app_runtime::problems` scrubs from a public issue.
fn redact(text: &str) -> String {
    const PREFIXES: &[&str] = &[
        "sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "Bearer ", "bearer ",
        "token=", "TOKEN=", "api_key=", "API_KEY=", "apikey=", "key=", "KEY=", "password=",
        "PASSWORD=", "secret=", "SECRET=",
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for prefix in PREFIXES {
            if let Some(after) = rest.strip_prefix(prefix) {
                let end = after
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
                    .unwrap_or(after.len());
                out.push_str(prefix);
                out.push_str("<redacted>");
                rest = &after[end..];
                continue 'scan;
            }
        }
        let ch = rest.chars().next().unwrap_or(' ');
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// The companion's tool selection with a judge in front: `select_tools_adaptive` unless the
/// judge is configured, reachable and sure.
pub fn select_tools(
    judge: Option<&dyn Judge>,
    config: &JudgeConfig,
    query: &str,
    earlier: &[ChatMessage],
    db: &YantrikDB,
    profile: &ModelCapabilityProfile,
) -> Vec<&'static str> {
    let ordinary = || select_tools_adaptive(query, db, profile);
    let Some(judge) = judge.filter(|_| config.use_on("route_tools")) else { return ordinary() };
    let shortlist = ToolCache::select_ranked_with_scores(&db.conn(), db, query, config.shortlist);
    if shortlist.is_empty() {
        return ordinary();
    }
    let started = Instant::now();
    let routing = match ask(judge, query, earlier, &shortlist) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(judge = judge.name(), error = %e, "judge unavailable; ordinary tool selection");
            return ordinary();
        }
    };
    let decision = decide(&routing, config.route_at);
    tracing::info!(
        judge = judge.name(), ms = started.elapsed().as_millis() as u64,
        pick = ?routing.pick, p = routing.p, multi_step = routing.multi_step, decision = ?decision,
        "judge routed tools"
    );
    apply(decision, ordinary)
}

/// Turn a decision into the tool names the chat model is shown. A pick that is not one of the
/// companion's own tools (an MCP tool, say) cannot be named statically: the ordinary list then.
fn apply(decision: Decision, ordinary: impl FnOnce() -> Vec<&'static str>) -> Vec<&'static str> {
    let known = |t: &str| CORE_TOOLS.iter().copied().find(|&c| c == t);
    match decision {
        Decision::NoTool => ALWAYS_TOOLS.to_vec(),
        Decision::Only(t) => match known(&t) {
            Some("discover_tools") => vec![DISCOVER],
            Some(t) => vec![DISCOVER, t],
            None => ordinary(),
        },
        Decision::Widen(t) => {
            let mut tools = ordinary();
            if let Some(t) = known(&t) {
                if !tools.contains(&t) {
                    tools.insert(0, t);
                }
            }
            tools
        }
        Decision::Fallback => ordinary(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use anyhow::Result;

    /// A judge that answers from a script and remembers what it was asked.
    struct Scripted {
        tool: (&'static str, f64),
        multi_step: f64,
        asked: Mutex<Vec<serde_json::Value>>,
    }

    impl Judge for Scripted {
        fn name(&self) -> &str { "scripted" }
        fn ask(&self, state: &serde_json::Value, questions: &[(&str, Question)]) -> Result<HashMap<String, Answer>> {
            self.asked.lock().unwrap().push(state.clone());
            let Question::Choice { options, .. } = &questions[0].1 else { panic!("tool must be a choice") };
            assert!(options.iter().any(|(k, _)| k == NO_TOOL), "no-tool must always be offered");
            let mut out = HashMap::new();
            out.insert("tool".into(), Answer::Choice {
                choice: self.tool.0.into(),
                probabilities: [(self.tool.0.to_string(), self.tool.1)].into_iter().collect(),
                confidence: self.tool.1,
            });
            out.insert("multi_step".into(), Answer::Noul(self.multi_step));
            Ok(out)
        }
    }

    fn judge(tool: &'static str, p: f64, multi_step: f64) -> Scripted {
        Scripted { tool: (tool, p), multi_step, asked: Mutex::new(Vec::new()) }
    }

    fn shortlist() -> Vec<(f32, String, String)> {
        vec![(0.61, "get_weather".into(), "get_weather: current conditions for a place".into()),
             (0.40, "read_file".into(), "read_file: read a text file".into())]
    }

    #[test]
    fn a_sure_single_pick_is_followed() {
        let r = ask(&judge("get_weather", 0.93, 0.1), "what's it like in Dallas", &[], &shortlist()).unwrap();
        assert_eq!(r, Routing { pick: Some("get_weather".into()), p: 0.93, multi_step: 0.1 });
        assert_eq!(decide(&r, 0.7), Decision::Only("get_weather".into()));
    }

    #[test]
    fn an_unsure_pick_falls_back_to_the_ordinary_shortlist() {
        let r = ask(&judge("get_weather", 0.55, 0.1), "hmm", &[], &shortlist()).unwrap();
        assert_eq!(decide(&r, 0.7), Decision::Fallback);
    }

    #[test]
    fn no_tool_is_its_own_answer() {
        let r = ask(&judge(NO_TOOL, 0.9, 0.0), "thanks, that's all", &[], &shortlist()).unwrap();
        assert_eq!(r.pick, None);
        assert_eq!(decide(&r, 0.7), Decision::NoTool);
    }

    #[test]
    fn several_tools_keep_the_shortlist_with_the_pick_first() {
        let r = ask(&judge("read_file", 0.8, 0.9), "read notes.txt and mail it to Sam", &[], &shortlist()).unwrap();
        assert_eq!(decide(&r, 0.7), Decision::Widen("read_file".into()));
        let tools = apply(Decision::Widen("read_file".into()), || vec!["remember", "send_email"]);
        assert_eq!(tools, vec!["read_file", "remember", "send_email"]);
    }

    #[test]
    fn only_shows_the_pick_and_the_way_to_the_rest() {
        let tools = apply(Decision::Only("read_file".into()), || panic!("the ordinary list is not needed"));
        assert_eq!(tools, vec!["discover_tools", "read_file"]);
        assert_eq!(apply(Decision::Only("discover_tools".into()), Vec::new), vec!["discover_tools"]);
    }

    #[test]
    fn a_pick_outside_the_companions_own_tools_uses_the_ordinary_list() {
        let tools = apply(Decision::Only("mcp__github__create_issue".into()), || vec!["remember"]);
        assert_eq!(tools, vec!["remember"]);
    }

    #[test]
    fn the_persons_last_messages_go_with_the_request_and_nothing_the_assistant_said() {
        let j = judge("send_email", 0.9, 0.0);
        let mut earlier = Vec::new();
        for i in 0..5 {
            earlier.push(ChatMessage::user(&format!("turn {i}")));
            earlier.push(ChatMessage::assistant("Your wifi password is hunter2"));
        }
        ask(&j, "yes, send it", &earlier, &shortlist()).unwrap();
        let state = j.asked.lock().unwrap()[0].to_string();
        assert!(state.contains(r#""persons_earlier_messages":["turn 2","turn 3","turn 4"]"#), "{state}");
        assert!(!state.contains("hunter2"), "an assistant reply reached the judge: {state}");
    }

    #[test]
    fn credentials_the_person_typed_are_redacted_before_the_judge_sees_them() {
        let j = judge("vault_store", 0.9, 0.0);
        let earlier = vec![ChatMessage::user("my key=abc123 for the router")];
        ask(&j, "keep my github token safe: ghp_AbCdEf123, thanks", &earlier, &shortlist()).unwrap();
        let state = j.asked.lock().unwrap()[0].to_string();
        assert!(!state.contains("ghp_AbCdEf123") && !state.contains("abc123"), "{state}");
        assert!(state.contains("ghp_<redacted>, thanks") && state.contains("key=<redacted> for"), "{state}");
    }
}
