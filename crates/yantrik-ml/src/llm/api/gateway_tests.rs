//! A backend told it is Yantrik models sends its tools natively: the model's name there is the
//! placeholder `picked` and says nothing about how the model takes tools (#1286).

use super::*;
use crate::thinking::ThoughtWithoutAnswer;

// gpt-oss-120b on OVHcloud's keyless endpoint, 16 Nov 2026: asked to use `calculate` with a
// `tools` field, and with the tools written into the prompt as text.
const NATIVE_TOOL_CALL: &str = include_str!("../testdata/gpt_oss_native_tool_call.sse");
const TOOLS_AS_TEXT: &str = include_str!("../testdata/gpt_oss_tools_as_text_no_answer.sse");

fn calculate() -> Vec<serde_json::Value> {
    vec![serde_json::json!({
        "type": "function",
        "function": {
            "name": "calculate",
            "description": "Work out an expression",
            "parameters": {"type": "object", "properties": {"expression": {"type": "string"}}, "required": ["expression"]}
        }
    })]
}

fn bodies(llm: &ApiLLM) -> Vec<serde_json::Value> {
    let asked = [ChatMessage::system("You are Yantrik."), ChatMessage::user("17 times 23?")];
    let tools = calculate();
    let config = GenerationConfig::default();
    [false, true].map(|stream| llm.build_openai_body(&asked, &config, Some(&tools), stream)).to_vec()
}

fn message_text(body: &serde_json::Value) -> String {
    body["messages"].to_string()
}

#[test]
fn a_gateway_backend_sends_tools_in_the_field_and_none_in_the_messages() {
    let llm = ApiLLM::new("https://gateway.test/v1", Some("fake-token".into()), "picked").for_gateway();
    for body in bodies(&llm) {
        assert_eq!(body["tools"][0]["function"]["name"], "calculate", "{body}");
        assert!(!message_text(&body).contains("<tools>"), "{body}");
    }
}

#[test]
fn a_backend_not_told_it_is_the_gateway_is_as_it_was_for_picked() {
    let llm = ApiLLM::new("https://gateway.test/v1", Some("fake-token".into()), "picked");
    for body in bodies(&llm) {
        assert!(body.get("tools").is_none(), "{body}");
        assert!(message_text(&body).contains("<tools>"), "{body}");
    }
}

#[test]
fn the_native_stream_gives_one_calculate_call() {
    let llm = ApiLLM::new("https://gateway.test/v1", None, "picked").for_gateway();
    let reply = llm.read_openai_stream(NATIVE_TOOL_CALL.as_bytes(), &mut |_| {}).expect("a tool call");
    assert_eq!(reply.api_tool_calls.len(), 1);
    let call = &reply.api_tool_calls[0].function;
    assert_eq!(call.name, "calculate");
    let args: serde_json::Value = serde_json::from_str(&call.arguments).unwrap();
    assert_eq!(args["expression"], "17*23");
    assert_eq!(reply.tool_calls.len(), 1);
}

#[test]
fn the_tools_as_text_stream_is_a_thought_without_an_answer() {
    let llm = ApiLLM::new("https://gateway.test/v1", None, "picked");
    let err = llm.read_openai_stream(TOOLS_AS_TEXT.as_bytes(), &mut |_| {}).expect_err("no answer");
    let failure = err.downcast_ref::<ThoughtWithoutAnswer>().expect("the typed failure");
    assert_eq!(failure.finish_reason, "stop");
}
