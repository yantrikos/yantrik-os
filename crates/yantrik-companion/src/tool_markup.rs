//! Tool calls a model wrote as text, and the rule that such markup is never the reply.
//!
//! VM 542, 16 Oct 2026: the companion on Claude showed the person, as its whole answer,
//! `<invoke name="recall"> <parameter name="query">Maya's location city</parameter> </invoke>`
//! and `<tool_call>\n<name>discover_tools</name>\n<parameter name="keyword">calendar</parameter>`.
//! The text parser only read Qwen's `<tool_call>{json}</tool_call>`, so anything else fell
//! through as words. These are read as calls when the format is known; a reply that is markup
//! and nothing else is never shown.

use yantrik_ml::{ApiToolCall, ApiToolCallFunction, LLMResponse, ToolCall};

/// Shown instead of a reply that was only tool-call markup the companion could not run.
pub const UNUSABLE_REPLY: &str =
    "The model answered in a form I couldn't use, so I have no answer to give. Please ask again.";

/// Where markup starts. `<function=` is Nemotron's form inside `<tool_call>`.
const OPENERS: &[&str] = &["<function_calls", "<invoke", "<tool_call", "<function="];

/// The next id for a call written as text (`adopt_text_calls`).
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Blocks removed whole, outermost first, each as (open, close).
const BLOCKS: &[(&str, &str)] = &[
    ("<function_calls>", "</function_calls>"),
    ("<tool_call>", "</tool_call>"),
    ("<invoke", "</invoke>"),
];

/// The tool calls written as text in `text`: Qwen's JSON form, Claude's
/// `<invoke name="..."><parameter name="...">` form, and `<tool_call><name>...` with parameters.
pub fn calls_in(text: &str) -> Vec<ToolCall> {
    // One block inside another is not a form any model writes, and reading it would count its
    // parameters for both calls (security review of 181c5881, #830, item 4).
    if nested(text) {
        tracing::warn!("tool-call markup with one block inside another; read as no calls");
        return Vec::new();
    }
    let json = yantrik_ml::parse_tool_calls(text);
    if !json.is_empty() {
        return json;
    }
    let mut calls: Vec<ToolCall> = bodies(text, "<invoke", "</invoke>")
        .filter_map(|(head, body)| Some(call(&decode(attr(head, "name")?), body)))
        .collect();
    calls.extend(
        bodies(text, "<tool_call>", "</tool_call>")
            .filter_map(|(_, body)| Some(call(&decode(between(body, "<name>", "</name>")?.trim()), body))),
    );
    calls
}

/// Does a call block hold another opener before it closes? `<function_calls>` is the wrapper
/// Claude puts around its `<invoke>`s, so only what is inside an `<invoke>` or a `<tool_call>`
/// counts.
fn nested(text: &str) -> bool {
    [("<invoke", "</invoke>"), ("<tool_call>", "</tool_call>")].iter().any(|(open, close)| {
        bodies(text, open, close).any(|(_, body)| {
            ["<invoke", "<tool_call", "<function_calls"].iter().any(|o| tag_starts(body, o).is_some())
        })
    })
}

/// Where `tag` starts in `text` as a tag of that name: `<invoke` followed by a space or `>`, not
/// `<invoked` or `<invoke_x` (#830 review, item 4). A tag that ends in `>` is matched as it is.
fn tag_starts(text: &str, tag: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = text[from..].find(tag).map(|i| from + i) {
        let next = text[at + tag.len()..].chars().next();
        if tag.ends_with('>') || next.is_some_and(|c| c == '>' || c == '/' || c.is_whitespace()) {
            return Some(at);
        }
        from = at + tag.len();
    }
    None
}

/// `text` without any tool-call markup, trimmed. An unclosed block runs to the end.
pub fn text_outside(text: &str) -> String {
    let mut out = yantrik_ml::extract_text_content(text);
    for (open, close) in BLOCKS {
        while let Some(start) = out.find(open) {
            let end = out[start..].find(close).map(|e| start + e + close.len()).unwrap_or(out.len());
            out.replace_range(start..end, "");
        }
    }
    for opener in OPENERS {
        if let Some(start) = out.find(opener) {
            out.truncate(start);
        }
    }
    out.trim().to_string()
}

/// A reply that holds tool-call markup and no words outside it.
pub fn is_markup_only(text: &str) -> bool {
    OPENERS.iter().any(|o| text.contains(o)) && text_outside(text).is_empty()
}

/// On the native path, take calls written as text as if they had come as `tool_calls`, so the
/// round runs them and returns their results the native way. The ids are made here: the API
/// needs an id on each result that matches a call in the assistant turn before it, and Claude
/// refuses a conversation in which two calls share an id, so each id is new for the process.
/// `streamed` is the text as it was streamed, when the reply was; otherwise `r.text` is read.
pub fn adopt_text_calls(r: &mut LLMResponse, streamed: Option<&str>) {
    if !r.tool_calls.is_empty() || !r.api_tool_calls.is_empty() {
        return;
    }
    let calls = calls_in(streamed.unwrap_or(&r.text));
    if calls.is_empty() {
        return;
    }
    tracing::warn!(count = calls.len(), "the model wrote its tool calls as text; running them");
    r.api_tool_calls = calls
        .iter()
        .map(|c| ApiToolCall {
            id: format!("text_call_{}", NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
            call_type: "function".into(),
            function: ApiToolCallFunction { name: c.name.clone(), arguments: c.arguments.to_string() },
        })
        .collect();
    r.tool_calls = calls;
}

/// Each `open ... close` block in `text`, as (the opening tag's attributes, the body).
fn bodies<'a>(text: &'a str, open: &'a str, close: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let start = tag_starts(rest, open)? + open.len();
        let end = start + rest[start..].find(close)?;
        let block = &rest[start..end];
        rest = &rest[end + close.len()..];
        Some(match block.find('>') {
            Some(gt) if !open.ends_with('>') => (&block[..gt], &block[gt + 1..]),
            _ => ("", block),
        })
    })
}

fn call(name: &str, body: &str) -> ToolCall {
    let arguments = bodies(body, "<parameter", "</parameter>")
        .filter_map(|(head, value)| Some((attr(head, "name")?.to_string(), value_of(value))))
        .collect::<serde_json::Map<_, _>>();
    ToolCall { name: name.to_string(), arguments: arguments.into() }
}

/// A parameter's value: numbers, booleans, lists and objects as JSON, anything else as text.
fn value_of(raw: &str) -> serde_json::Value {
    let raw = decode(raw.trim());
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) if !v.is_string() && !v.is_null() => v,
        _ => serde_json::Value::String(raw),
    }
}

/// XML's five entities, as a model writes them inside markup (#830 review, item 4): the value
/// the tool gets is the text the model meant. `&amp;` last, so `&amp;lt;` stays `&lt;`.
fn decode(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    raw.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
        .replace("&#39;", "'").replace("&amp;", "&")
}

/// `key="value"` (or single-quoted) in a tag's attributes. The key must start the attributes
/// or follow a space: `data-name="recall" name="run_command"` is `run_command`, so the tool a
/// log shows is the tool that runs (#830 review, item 4).
fn attr<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{key}=");
    let mut from = 0;
    let at = loop {
        let i = from + head[from..].find(&pat)?;
        if head[..i].chars().next_back().map_or(true, char::is_whitespace) {
            break i + pat.len();
        }
        from = i + pat.len();
    };
    let quote = head[at..].chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &head[at + 1..];
    Some(&value[..value.find(quote)?])
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    Some(&text[start..start + text[start..].find(close)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    // The three replies VM 542 showed the person on 16 Oct 2026, verbatim.
    const CALENDAR: &str =
        "<tool_call>\n<name>discover_tools</name>\n<parameter name=\"keyword\">calendar</parameter>\n</tool_call>";
    const REMEMBER: &str = "<invoke name=\"remember\">\n<parameter name=\"text\">Maya's friend Dana's housewarming is on 8 November (2026).</parameter>\n<parameter name=\"domain\">identity</parameter>\n<parameter name=\"importance\">0.8</parameter>\n</invoke>\n<invoke name=\"recall\">\n<parameter name=\"query\">Dana</parameter>\n</invoke>";
    const WEATHER: &str =
        "<invoke name=\"recall\"> <parameter name=\"query\">Maya's location city</parameter> </invoke>";

    #[test]
    fn the_vms_replies_are_read_as_calls() {
        let c = calls_in(CALENDAR);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "discover_tools");
        assert_eq!(c[0].arguments["keyword"], "calendar");

        let c = calls_in(REMEMBER);
        assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["remember", "recall"]);
        assert_eq!(c[0].arguments["text"], "Maya's friend Dana's housewarming is on 8 November (2026).");
        assert_eq!(c[0].arguments["domain"], "identity");
        assert_eq!(c[0].arguments["importance"], 0.8);

        let c = calls_in(WEATHER);
        assert_eq!(c[0].name, "recall");
        assert_eq!(c[0].arguments["query"], "Maya's location city");
    }

    #[test]
    fn markup_only_replies_are_never_words() {
        for reply in [CALENDAR, REMEMBER, WEATHER, "<function_calls>\n<invoke name=\"x\">", "<tool_call>\n<function=recall>"] {
            assert!(is_markup_only(reply), "{reply:?}");
            assert_eq!(text_outside(reply), "", "{reply:?}");
        }
    }

    #[test]
    fn words_around_markup_are_kept_and_plain_words_pass() {
        assert_eq!(text_outside(&format!("Let me check.\n{WEATHER}")), "Let me check.");
        assert!(!is_markup_only(&format!("Let me check.\n{WEATHER}")));
        assert!(!is_markup_only("Your disk is 41% full."));
        assert!(!is_markup_only(""));
        assert_eq!(text_outside("Use <b>bold</b> here."), "Use <b>bold</b> here.");
    }

    /// Security review of 181c5881, #830, item 4.
    #[test]
    fn markup_that_only_looks_like_a_call_is_not_one() {
        assert!(calls_in("<invoked name=\"recall\"><parameter name=\"query\">x</parameter></invoked></invoke>").is_empty());
        assert!(calls_in("<invoke_x name=\"recall\"><parameter name=\"query\">x</parameter></invoke>").is_empty());
        let c = calls_in("<invoke data-name=\"recall\" name=\"calculate\"><parameter name=\"expression\">1+1</parameter></invoke>");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "calculate", "the name attribute, not data-name");
        let c = calls_in("<invoke data-name=\"recall\"><parameter name=\"query\">x</parameter></invoke>");
        assert!(c.is_empty(), "data-name is not a name: {c:?}");
        let c = calls_in("<invoke name=\"recall\"><parameter data-name=\"x\" name=\"query\">a</parameter></invoke>");
        assert_eq!(c[0].arguments, serde_json::json!({"query": "a"}));
    }

    #[test]
    fn a_block_inside_another_is_no_call() {
        let nested = "<tool_call><name>recall</name><parameter name=\"query\">a</parameter>\
            <invoke name=\"run_command\"><parameter name=\"command\">ls</parameter></invoke></tool_call>";
        assert!(calls_in(nested).is_empty());
        let nested = "<invoke name=\"recall\"><tool_call><name>run_command</name></tool_call></invoke>";
        assert!(calls_in(nested).is_empty());
        // Claude's own wrapper is not nesting.
        let wrapped = format!("<function_calls>\n{WEATHER}\n</function_calls>");
        assert_eq!(calls_in(&wrapped)[0].name, "recall");
    }

    #[test]
    fn parameter_values_are_entity_decoded() {
        let c = calls_in("<invoke name=\"calculate\"><parameter name=\"expression\">2 &lt; 3 &amp;&amp; &quot;a&quot; &amp;lt;</parameter></invoke>");
        assert_eq!(c[0].arguments["expression"], "2 < 3 && \"a\" &lt;");
    }

    #[test]
    fn qwen_json_calls_are_read_as_before() {
        let c = calls_in("<tool_call>\n{\"name\": \"calculate\", \"arguments\": {\"expression\": \"1+1\"}}\n</tool_call>");
        assert_eq!(c[0].name, "calculate");
        assert_eq!(c[0].arguments["expression"], "1+1");
    }

    #[test]
    fn text_calls_on_the_native_path_get_ids_the_results_can_answer() {
        let mut r = LLMResponse {
            text: WEATHER.into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            tool_calls: Vec::new(),
            api_tool_calls: Vec::new(),
            stop_reason: "stop".into(),
        };
        adopt_text_calls(&mut r, None);
        assert_eq!(r.tool_calls[0].name, "recall");
        assert!(r.api_tool_calls[0].id.starts_with("text_call_"));
        let first = r.api_tool_calls[0].id.clone();
        let mut again = LLMResponse { tool_calls: Vec::new(), api_tool_calls: Vec::new(), ..r.clone() };
        adopt_text_calls(&mut again, None);
        assert_ne!(again.api_tool_calls[0].id, first, "no two calls share an id");
        assert_eq!(ToolCall::from_api(&r.api_tool_calls[0]).unwrap().arguments["query"], "Maya's location city");
    }
}
