//! The Lens's conversation, put back when the Lens opens empty (#246).
//!
//! The Lens used to start every shell blank: `messages` is a fresh model at startup and nothing
//! filled it, so an update, a crash or a deploy wiped the chat a person was in the middle of. The
//! conversation was never lost. The Agents store keeps every turn of it, the Lens's own included
//! (`<mind>:main`, one line per turn in `~/.local/share/yantrik/agents`), and the Agents screen
//! shows it. The Lens just never read it. This reads it: the last turns of the answering mind's
//! conversation, as the bubbles the Lens draws. The request goes in a bubble of its own; the
//! reply carries its text as markdown blocks and each tool call as the card it was.

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::agents::model::{Item, Turn};
use crate::{App, ContentBlock, MessageData, ToolCallData};

/// How many of the latest turns the Lens puts back: enough to see where the conversation was,
/// few enough that the Lens opens at the bottom of it rather than a day ago.
pub const RESTORED_TURNS: usize = 12;

/// The bubbles for the last `limit` turns of a conversation, oldest first.
pub fn bubbles(turns: &[Turn], limit: usize) -> Vec<MessageData> {
    let start = turns.len().saturating_sub(limit);
    let mut out = Vec::new();
    for turn in &turns[start..] {
        if !turn.prompt.trim().is_empty() {
            out.push(MessageData {
                run: Default::default(),
                role: "user".into(),
                content: turn.prompt.as_str().into(),
                is_streaming: false,
                blocks: ModelRc::default(),
            });
        }
        let (content, blocks) = reply(turn);
        if !blocks.is_empty() {
            out.push(MessageData {
                run: Default::default(),
                role: "assistant".into(),
                content: content.into(),
                is_streaming: false,
                blocks: ModelRc::new(VecModel::from(blocks)),
            });
        }
    }
    out
}

/// A turn's reply as the Lens draws one: its text, and the blocks the bubble renders.
fn reply(turn: &Turn) -> (String, Vec<ContentBlock>) {
    let mut text = Vec::new();
    let mut blocks = Vec::new();
    for item in &turn.items {
        match item {
            Item::Text(capped) => {
                let said = capped.text();
                if said.trim().is_empty() {
                    continue;
                }
                blocks.extend(crate::markdown::parse_blocks(&said).into_iter().map(|b| ContentBlock {
                    block_type: SharedString::from(b.block_type),
                    text: b.text.as_str().into(),
                    call: b.call.as_ref().map(crate::trail::ToolCall::to_card).unwrap_or_default(),
                }));
                text.push(said);
            }
            Item::Card(card) => {
                let card = card.shown();
                let call = ToolCallData { status: card.state.status().into(), ..card.as_call().to_card() };
                blocks.push(ContentBlock { block_type: "tool".into(), text: call.summary.clone(), call });
            }
            Item::Note(note) => blocks.push(line(note)),
            // Never the question's words: a bubble is a copy the Lens keeps on screen, which an
            // erasure of them could not reach. The question itself is in Agents.
            Item::Question(q) => blocks.push(line(&match (q.answer.as_str(), q.closed.as_str()) {
                ("", "") => "Asked you a question (in Agents)".to_string(),
                ("", why) => format!("Asked you a question (in Agents; not answered: {why})"),
                (answer, _) => format!("Asked you a question (in Agents; you answered: {answer})"),
            })),
            Item::Approval(approval) => {
                let record = approval.record.as_str();
                let record = if record.trim().is_empty() { approval.what.as_str() } else { record };
                blocks.push(line(record));
            }
            // Folded in the Agents pane, and not part of what was said.
            Item::Thinking(_) => {}
        }
    }
    // A turn still open in the saved session did not finish while the shell was there to see
    // it: the shell stopped mid-answer. Say so rather than leave a reply that just ends.
    if turn.open() && !turn.prompt.trim().is_empty() {
        blocks.push(line("This had not finished when the desktop last stopped; what came of it is in Agents."));
    }
    (text.join("\n\n"), blocks)
}

fn line(text: &str) -> ContentBlock {
    ContentBlock { block_type: "text".into(), text: text.into(), call: ToolCallData::default() }
}

/// Fill an empty Lens with the answering mind's conversation. Says how many bubbles it put back:
/// none when the Lens already holds a conversation, when the built-in companion is answering (its
/// conversation is not an agent's), or when the mind has said nothing yet.
pub fn restore_if_empty(ui: &App) -> usize {
    // Once per shell start: the empty Lens this is for is the one a start leaves. An empty Lens
    // later is one the person emptied with New chat (#246), and filling it again would undo the
    // press the next time the Lens opened.
    static TRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if TRIED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return 0;
    }
    let messages = ui.get_messages();
    let Some(model) = messages.as_any().downcast_ref::<VecModel<MessageData>>() else {
        return 0;
    };
    if model.row_count() > 0 {
        return 0;
    }
    let Some(host) = crate::wire::harness::host() else {
        return 0;
    };
    let agent = crate::agents::feed::main_agent(&host.active_id());
    let restored =
        crate::agents::store().read(|s| s.agent(&agent).map(|a| bubbles(&a.turns, RESTORED_TURNS))).unwrap_or_default();
    for bubble in &restored {
        model.push(bubble.clone());
    }
    if !restored.is_empty() {
        tracing::info!(agent = %agent.0, bubbles = restored.len(), "The Lens opened on its saved conversation");
    }
    restored.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model::{Capped, Card, Provenance};

    fn capped(text: &str) -> Capped {
        let mut c = Capped::new(4096, 65536);
        c.push(text.as_bytes());
        c
    }

    fn turn(n: u64, prompt: &str, items: Vec<Item>, ended: bool) -> Turn {
        Turn {
            n,
            prompt: prompt.into(),
            started: 100 + n,
            ended: ended.then_some(200 + n),
            ok: ended.then_some(true),
            lost: false, origin: Default::default(),
            items,
            events: false,
            trail_seq: 0,
        }
    }

    fn kinds(m: &MessageData) -> Vec<String> {
        m.blocks.iter().map(|b| b.block_type.to_string()).collect()
    }

    #[test]
    fn a_turn_becomes_the_request_and_the_reply_with_its_calls_as_cards() {
        let card = Card::new(
            "c1",
            "os_act",
            "notes.create",
            serde_json::json!({"app": "notes", "action": "create", "title": "Market status"}),
            Provenance::Reported,
            150,
        );
        let turns = vec![turn(
            1,
            "Create a note for tomorrow's market status meeting",
            vec![Item::Card(card), Item::Text(capped("Done. **Market status** is in Notes:\n\n- agenda\n- owners"))],
            true,
        )];
        let b = bubbles(&turns, RESTORED_TURNS);
        assert_eq!(b.len(), 2, "the request and the reply");
        assert_eq!(b[0].role, "user");
        assert_eq!(b[0].content, "Create a note for tomorrow's market status meeting");
        assert_eq!(b[1].role, "assistant");
        assert!(!b[1].is_streaming, "a restored reply is finished, not streaming");
        assert_eq!(kinds(&b[1]), vec!["tool", "text", "bullet"], "the call as a card, then the text as markdown");
        let call = b[1].blocks.row_data(0).unwrap().call;
        assert_eq!(call.name, "os_act");
        assert!(call.summary.contains("notes.create"), "the card's line names what it touched: {}", call.summary);
    }

    #[test]
    fn only_the_latest_turns_come_back_oldest_first() {
        let turns: Vec<Turn> = (1..=20).map(|n| turn(n, &format!("question {n}"), vec![Item::Text(capped("answer"))], true)).collect();
        let b = bubbles(&turns, 3);
        let asked: Vec<String> = b.iter().filter(|m| m.role == "user").map(|m| m.content.to_string()).collect();
        assert_eq!(asked, vec!["question 18", "question 19", "question 20"]);
    }

    #[test]
    fn a_turn_the_shell_stopped_in_the_middle_of_says_so() {
        let turns = vec![turn(1, "create a small town model", vec![Item::Text(capped("Starting on the terrain."))], false)];
        let b = bubbles(&turns, RESTORED_TURNS);
        let last = b[1].blocks.row_data(b[1].blocks.row_count() - 1).unwrap();
        assert!(last.text.contains("had not finished"), "the reply ends by saying it was cut off: {}", last.text);
    }

    #[test]
    fn a_turn_with_nothing_said_draws_no_empty_reply() {
        let turns = vec![turn(1, "hello", vec![Item::Thinking(capped("hmm"))], true)];
        let b = bubbles(&turns, RESTORED_TURNS);
        assert_eq!(b.len(), 1, "the request alone; thinking is not a reply");
    }
}
