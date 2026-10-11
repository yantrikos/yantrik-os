//! `chat_view`'s answer, read into plain structs the screen draws from.

use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub index: usize,
    pub role: String,
    pub text: String,
    pub streaming: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Call { name: String, label: String, target: String, args: String, state: String, summary: String, seconds: Option<u64>, repeats: u64 },
    Approval { what: String, outcome: String },
    Question { prompt: String, answered: bool },
    Note(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    pub n: u64,
    pub prompt: String,
    pub ended: bool,
    pub ok: Option<bool>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mind {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub answering: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatView {
    pub revision: String,
    pub total: usize,
    pub messages: Vec<Message>,
    pub mind: Mind,
    pub minds: Vec<Mind>,
    pub state: String,
    pub turn: Option<Turn>,
    pub mode: String,
    pub mind_view_running: bool,
    pub mind_view_apps: Vec<String>,
    pub waiting_on_you: usize,
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn mind(v: &Value) -> Mind {
    Mind {
        id: s(&v["id"]),
        name: s(&v["name"]),
        detail: s(&v["detail"]),
        answering: v["answering"].as_bool().unwrap_or(false),
    }
}

fn step(v: &Value) -> Option<Step> {
    Some(match v["kind"].as_str()? {
        "call" => Step::Call {
            name: s(&v["name"]),
            label: s(&v["label"]),
            target: s(&v["target"]),
            args: s(&v["args"]),
            state: s(&v["state"]),
            summary: s(&v["summary"]),
            seconds: v["seconds"].as_u64(),
            repeats: v["repeats"].as_u64().unwrap_or(0),
        },
        "approval" => Step::Approval { what: s(&v["what"]), outcome: s(&v["outcome"]) },
        "question" => Step::Question { prompt: s(&v["prompt"]), answered: v["answered"].as_bool().unwrap_or(false) },
        "note" => Step::Note(s(&v["text"])),
        _ => return None,
    })
}

impl ChatView {
    pub fn read(v: &Value) -> ChatView {
        let list = |key: &str| v[key].as_array().cloned().unwrap_or_default();
        ChatView {
            revision: s(&v["revision"]),
            total: v["total"].as_u64().unwrap_or(0) as usize,
            messages: list("messages")
                .iter()
                .map(|m| Message {
                    index: m["index"].as_u64().unwrap_or(0) as usize,
                    role: s(&m["role"]),
                    text: s(&m["text"]),
                    streaming: m["streaming"].as_bool().unwrap_or(false),
                })
                .collect(),
            mind: mind(&v["mind"]),
            minds: list("minds").iter().map(mind).collect(),
            state: s(&v["state"]),
            turn: v["turn"].as_object().map(|_| {
                let t = &v["turn"];
                Turn {
                    n: t["n"].as_u64().unwrap_or(0),
                    prompt: s(&t["prompt"]),
                    ended: !t["ended"].is_null(),
                    ok: t["ok"].as_bool(),
                    steps: t["items"].as_array().map(|a| a.iter().filter_map(step).collect()).unwrap_or_default(),
                }
            }),
            mode: s(&v["mode"]),
            mind_view_running: v["mind_view"]["running"].as_bool().unwrap_or(false),
            mind_view_apps: v["mind_view"]["apps"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            waiting_on_you: v["waiting_on_you"].as_u64().unwrap_or(0) as usize,
        }
    }

    /// Whether the mind is at work: a turn open, or a reply still arriving.
    pub fn busy(&self) -> bool {
        self.turn.as_ref().is_some_and(|t| !t.ended) || self.messages.last().is_some_and(|m| m.streaming)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_view_reads_whole_and_tolerates_what_is_missing() {
        let v = ChatView::read(&json!({
            "revision": "ab", "total": 2,
            "messages": [{"index": 1, "role": "user", "text": "hi", "streaming": false}],
            "mind": {"id": "mind", "name": "Yantrik Mind", "detail": "deepseek"},
            "minds": [{"id": "mind", "name": "Yantrik Mind", "answering": true}],
            "state": "runningtool",
            "turn": {"n": 3, "prompt": "hi", "ended": null, "items": [
                {"kind": "call", "name": "os_act", "target": "blender", "args": "kind=monkey", "state": "running"},
                {"kind": "approval", "what": "blender.new_scene", "outcome": "pending"},
                {"kind": "mystery"}
            ]},
            "mode": "auto", "mind_view": {"running": true, "apps": ["blender"]}, "waiting_on_you": 1
        }));
        assert_eq!(v.messages[0].text, "hi");
        assert_eq!(v.mind.name, "Yantrik Mind");
        assert!(v.minds[0].answering);
        let t = v.turn.as_ref().unwrap();
        assert!(!t.ended);
        assert_eq!(t.steps.len(), 2, "an unknown kind is skipped");
        assert!(v.busy());
        assert_eq!(v.mind_view_apps, ["blender"]);

        let empty = ChatView::read(&json!({}));
        assert!(empty.messages.is_empty() && empty.turn.is_none() && !empty.busy());
    }
}
