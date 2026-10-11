//! Erasing words from one agent's session at the person's request (the harness `redact` event).
//!
//! The host decides whether an erasure may happen at all (yantrik-harness, `host::erase` and
//! `run_store::erase`) and erases the run store; this is the shell's own copy, the session the
//! Agents pane draws and `~/.local/share/yantrik/agents/<agent>.jsonl` keeps.
//!
//! # What is erased, and what is only shown erased
//!
//! The words the person and the agent said are replaced with the marker: each prompt, the
//! agent's text and its thinking (each joined across the turn's blocks before matching, so words
//! split across chunks or around a card are still found), the questions it asked — the Keep/Erase
//! question's own prompt included, since it quotes the words —, the shell's notes, the title and
//! the status line.
//!
//! The record of what happened is not touched, and never hidden: an erasure must not be a way to
//! make an action disappear. An approval (its `app.action`, what it was for and how it came out)
//! is shown exactly as it was. A tool call keeps all its words, and only its free text is drawn
//! with the marker ([`Card::shown`]): its preview and summary, the strings under its free-text
//! arguments ([`FREE_TEXT_ARGS`]; none for a command, whose arguments are the action itself), and
//! its output when that is no longer than [`MASK_OUTPUT_MAX`]. Its name, its target and every
//! other argument are drawn as they are. The
//! person — and only the person, in the pane — can show a masked card as it was. The refusal lines
//! are masked the same way (`Agent::shown_refusals`). A mask keeps no words and no digest; it is
//! keyed by the request the person answered.
//!
//! # Searched off the lock
//!
//! [`Store::erasure_size`] says how much there is to search from lengths alone;
//! [`Store::erasure_texts`] copies the texts out under the lock; an [`ErasurePlan`] measures and
//! searches the copies with no lock held; [`Store::apply_erasure`] takes the lock again only to
//! replace what was found, checking each match against the text as it is by then.

use std::collections::HashMap;

use yantrik_harness::host::{ShellErased, ShellErasure};
use yantrik_harness::redact::{apply_json_under, json_strings_under, Found, Prepared, Search};

use super::*;

/// The arguments of a tool call that are free text — what was said, not what was done — and so
/// are drawn masked when they hold erased words. Anything under one of these keys, at any depth.
pub const FREE_TEXT_ARGS: [&str; 14] = [
    "text", "content", "body", "message", "note", "title", "subject", "prompt", "query", "description",
    "summary", "comment", "reply", "answer",
];

/// The argument keys of `card` whose strings are free text: none for a command, whose arguments —
/// the text typed into a terminal, a query, an instruction handed on — are the action itself.
fn free_args(card: &Card) -> &'static [&'static str] {
    if card.is_command() {
        &[]
    } else {
        &FREE_TEXT_ARGS
    }
}

/// The longest output of a tool call that is searched and drawn masked. A longer one is neither
/// copied nor searched, and is drawn as it is (docs/harness.md, Limits).
pub const MASK_OUTPUT_MAX: usize = 64 * 1024;

/// Where one text of an agent's session is, to find it again under the lock.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    Title,
    Status,
    /// A refusal line, as shown.
    Refusal(usize),
    /// Turn `n`'s prompt, its text blocks joined, its thinking joined.
    Prompt(usize),
    Text(usize),
    Thinking(usize),
    /// One field of item `i` of turn `n`.
    Item(usize, usize, Field),
}

/// A field of one item of a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Question,
    Note,
    /// A card's free text, as its mask starts from; `Arg(k)` is the `k`th free-text string in
    /// its arguments.
    Arg(usize),
    Preview,
    Summary,
    Output,
}

/// One agent's texts, copied out of the store, to be measured and searched with no lock held.
pub struct ErasurePlan {
    pub id: AgentId,
    texts: Vec<(Place, Prepared)>,
    found: HashMap<Place, Found>,
}

impl ErasurePlan {
    /// The copies (from [`Store::erasure_texts`]) put in canonical form. No lock is needed.
    pub fn new(id: AgentId, texts: Vec<(Place, Vec<String>)>) -> ErasurePlan {
        let texts = texts.into_iter().map(|(place, pieces)| (place, Prepared::new(&pieces))).collect();
        ErasurePlan { id, texts, found: HashMap::new() }
    }

    /// What searching it will cost (`redact::MAX_WORK`'s units).
    pub fn cost(&self, search: &Search) -> u64 {
        self.texts.iter().fold(0u64, |sum, (_, text)| sum.saturating_add(search.work(text)))
    }

    /// Search the copies.
    pub fn find(&mut self, search: &Search) {
        for (place, text) in &self.texts {
            let found = search.find(text);
            if !found.is_empty() {
                self.found.insert(place.clone(), found);
            }
        }
    }

    fn get(&self, place: Place) -> Option<&Found> {
        self.found.get(&place)
    }
}

impl Store {
    /// How many bytes of text [`Store::erasure_texts`] would copy for agent `id`, worked out from
    /// lengths alone: nothing is copied.
    pub fn erasure_size(&self, id: &AgentId) -> u64 {
        let Some(agent) = self.agent(id) else { return 0 };
        let refusals = agent.refusals_shown.as_ref().unwrap_or(&agent.refusals);
        let mut bytes = agent.meta.title.len() + agent.status.len() + refusals.iter().map(String::len).sum::<usize>();
        for turn in &agent.turns {
            bytes += turn.prompt.len();
            for item in &turn.items {
                bytes += match item {
                    Item::Text(buffer) | Item::Thinking(buffer) => buffer.kept(),
                    Item::Question(q) => q.prompt.len(),
                    Item::Note(note) => note.len(),
                    Item::Card(card) => card_size(card),
                    Item::Approval(_) => 0,
                };
            }
        }
        bytes as u64
    }

    /// Copies of every text of agent `id`'s session an erasure reaches (see the module docs).
    /// Empty when the store does not hold it.
    pub fn erasure_texts(&self, id: &AgentId) -> Vec<(Place, Vec<String>)> {
        let Some(agent) = self.agent(id) else { return Vec::new() };
        let mut out = vec![(Place::Title, vec![agent.meta.title.clone()]), (Place::Status, vec![agent.status.clone()])];
        let refusals = agent.refusals_shown.as_ref().unwrap_or(&agent.refusals);
        out.extend(refusals.iter().enumerate().map(|(n, line)| (Place::Refusal(n), vec![line.clone()])));
        for (t, turn) in agent.turns.iter().enumerate() {
            out.push((Place::Prompt(t), vec![turn.prompt.clone()]));
            out.push((Place::Text(t), block_texts(&turn.items, false).into_iter().map(|(_, text)| text).collect()));
            out.push((Place::Thinking(t), block_texts(&turn.items, true).into_iter().map(|(_, text)| text).collect()));
            for (i, item) in turn.items.iter().enumerate() {
                let at = |field| Place::Item(t, i, field);
                match item {
                    Item::Question(q) => out.push((at(Field::Question), vec![q.prompt.clone()])),
                    Item::Note(note) => out.push((at(Field::Note), vec![note.clone()])),
                    Item::Card(card) => {
                        let base = card_mask(card);
                        let args = json_strings_under(&base.args, Some(free_args(card)));
                        out.extend(args.into_iter().enumerate().map(|(k, s)| (at(Field::Arg(k)), vec![s])));
                        out.push((at(Field::Preview), vec![base.preview]));
                        out.push((at(Field::Summary), vec![base.summary]));
                        if let Some(output) = base.output {
                            out.push((at(Field::Output), vec![output]));
                        }
                    }
                    Item::Approval(_) | Item::Text(_) | Item::Thinking(_) => {}
                }
            }
        }
        out
    }

    /// Apply what `plan` found to agent `id`'s session in memory, mask the free text of the cards
    /// that keep the words, and record the erasure. Each match is checked again against the text
    /// as it is now. Writing it to disk is the caller's (`Agents::redact`).
    pub fn apply_erasure(&mut self, id: &AgentId, e: &ShellErasure<'_>, plan: &ErasurePlan) -> ShellErased {
        let Some(i) = self.index(id) else { return ShellErased::default() };
        let now = self.now();
        let agent = &mut self.agents[i];
        let mut done = ShellErased::default();

        done.places += in_place(&mut agent.meta.title, plan.get(Place::Title));
        done.places += in_place(&mut agent.status, plan.get(Place::Status));
        for (t, turn) in agent.turns.iter_mut().enumerate() {
            done.places += in_place(&mut turn.prompt, plan.get(Place::Prompt(t)));
            done.places += blocks(&mut turn.items, plan.get(Place::Text(t)), false);
            done.places += blocks(&mut turn.items, plan.get(Place::Thinking(t)), true);
            for (n, item) in turn.items.iter_mut().enumerate() {
                let at = |field| Place::Item(t, n, field);
                match item {
                    Item::Question(q) => done.places += in_place(&mut q.prompt, plan.get(at(Field::Question))),
                    Item::Note(note) => done.places += in_place(note, plan.get(at(Field::Note))),
                    Item::Card(card) => done.masked += mask_card(card, e.request_id, plan, t, n),
                    // An approval is what was asked and how it came out: never masked.
                    Item::Approval(_) | Item::Text(_) | Item::Thinking(_) => {}
                }
            }
        }
        let mut shown = agent.refusals_shown.clone().unwrap_or_else(|| agent.refusals.clone());
        let masked_lines: usize =
            shown.iter_mut().enumerate().map(|(n, line)| in_place(line, plan.get(Place::Refusal(n)))).sum();
        if masked_lines > 0 {
            agent.refusals_shown = Some(shown);
            done.masked += masked_lines;
        }

        let erasure = Erasure { request: e.request_id.to_string(), places: e.places_in_runs + done.places, at: now };
        if let Some(turn) = agent.turns.last_mut() {
            turn.items.push(Item::Note(erasure.line()));
        }
        agent.erasures.push(erasure);
        self.mark(i);
        done
    }

    /// Copy, search and apply at once, under whatever lock the caller holds: for tests. The shell
    /// erases through `Agents::redact`, which searches off the lock.
    pub fn redact(&mut self, id: &AgentId, e: &ShellErasure<'_>) -> ShellErased {
        let mut plan = ErasurePlan::new(id.clone(), self.erasure_texts(id));
        plan.find(&Search::new(e.needles));
        self.apply_erasure(id, e, &plan)
    }
}

/// Apply `found` to one string in place. How many places.
fn in_place(text: &mut String, found: Option<&Found>) -> usize {
    match found.and_then(|f| f.apply(&[text.as_str()])) {
        Some((mut erased, n)) => {
            *text = erased.remove(0);
            n
        }
        None => 0,
    }
}

/// A turn's text blocks (or its thinking), in order: where each is, and its text.
fn block_texts(items: &[Item], thinking: bool) -> Vec<(usize, String)> {
    let mut kept = Vec::new();
    for (at, item) in items.iter().enumerate() {
        match item {
            Item::Text(buffer) if !thinking => kept.push((at, buffer.text())),
            Item::Thinking(buffer) if thinking => kept.push((at, buffer.text())),
            _ => {}
        }
    }
    kept
}

/// Apply `found` to a turn's text blocks (or its thinking), joined as one text. How many places.
fn blocks(items: &mut [Item], found: Option<&Found>, thinking: bool) -> usize {
    let Some(found) = found else { return 0 };
    let kept = block_texts(items, thinking);
    let pieces: Vec<&str> = kept.iter().map(|(_, t)| t.as_str()).collect();
    let Some((erased, n)) = found.apply(&pieces) else { return 0 };
    for ((at, before), after) in kept.iter().zip(erased) {
        if *before == after {
            continue;
        }
        let mut buffer = Capped::new(TEXT_HEAD, TEXT_CAP);
        buffer.push(after.as_bytes());
        items[*at] = if thinking { Item::Thinking(buffer) } else { Item::Text(buffer) };
    }
    n
}

/// What a card's mask starts from: its mask so far, or its own free text. The output only when it
/// is no longer than [`MASK_OUTPUT_MAX`].
struct CardBase {
    args: serde_json::Value,
    preview: String,
    summary: String,
    output: Option<String>,
}

fn card_mask(card: &Card) -> CardBase {
    match &card.mask {
        Some(mask) => CardBase {
            args: mask.args.clone(),
            preview: mask.preview.clone(),
            summary: mask.summary.clone(),
            output: (mask.output.len() <= MASK_OUTPUT_MAX).then(|| mask.output.clone()),
        },
        None => CardBase {
            args: card.args.clone(),
            preview: card.preview.clone(),
            summary: card.summary.clone(),
            output: (card.output.bytes.kept() <= MASK_OUTPUT_MAX).then(|| card.output.bytes.text()),
        },
    }
}

/// [`card_mask`]'s size in bytes, without copying it.
fn card_size(card: &Card) -> usize {
    let (args, preview, summary, output) = match &card.mask {
        Some(mask) => (&mask.args, mask.preview.len(), mask.summary.len(), mask.output.len()),
        None => (&card.args, card.preview.len(), card.summary.len(), card.output.bytes.kept()),
    };
    let args: usize = json_strings_under(args, Some(free_args(card))).iter().map(String::len).sum();
    args + preview + summary + if output <= MASK_OUTPUT_MAX { output } else { 0 }
}

/// Mask a card whose free text holds what was found; the card keeps its own. Its name, target and
/// other arguments are never masked. How many places masked.
fn mask_card(card: &mut Card, request: &str, plan: &ErasurePlan, t: usize, i: usize) -> usize {
    let at = |field| Place::Item(t, i, field);
    let mut base = card_mask(card);
    let args: Vec<Found> = (0..json_strings_under(&base.args, Some(free_args(card))).len())
        .map(|k| plan.get(at(Field::Arg(k))).cloned().unwrap_or_default())
        .collect();
    let mut output = base.output.take();
    let n = apply_json_under(&mut base.args, &args, Some(free_args(card)))
        + in_place(&mut base.preview, plan.get(at(Field::Preview)))
        + in_place(&mut base.summary, plan.get(at(Field::Summary)))
        + output.as_mut().map_or(0, |o| in_place(o, plan.get(at(Field::Output))));
    if n > 0 {
        let previous = card.mask.as_ref().map(|m| m.output.clone());
        card.mask = Some(Box::new(CardMask {
            request: request.to_string(),
            target: card.target.clone(),
            args: base.args,
            preview: base.preview,
            summary: base.summary,
            output: output.or(previous).unwrap_or_else(|| card.output.bytes.text()),
        }));
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use yantrik_harness::redact::{Needle, MARKER};

    const SECRET: &str = "Priya";

    fn erasure(needles: &[Needle]) -> ShellErasure<'_> {
        ShellErasure { request_id: "forget-1", needles, places_in_runs: 4 }
    }

    /// An agent whose session holds the secret everywhere it can: the prompt, the reply split
    /// across two chunks, its thinking, a tool call's target, arguments and output, and the
    /// question quoting it; and an approval, which is only ever `app.action`.
    fn session() -> (Store, AgentId) {
        let mut s = Store::with_clock(Box::new(|| 1_000));
        let pi = AgentId::new("pi", "c-1");
        s.open_turn(&pi, "my sister is Priya, remember that");
        s.text(&pi, "Got it: Pri");
        s.text(&pi, "ya is your sister.");
        s.event(&pi, &Event::Thinking { delta: "store Priya".into() }, Provenance::Reported);
        s.event(
            &pi,
            &Event::ToolStart {
                call: "t1".into(),
                name: "os_act".into(),
                target: "notes/Priya.md".into(),
                args: json!({"path": "notes/Priya.md", "text": "sister: Priya"}),
            },
            Provenance::Reported,
        );
        s.event(&pi, &Event::ToolOutput { call: "t1".into(), stream: Stream::Stdout, delta: "saved Priya\n".into() }, Provenance::Reported);
        s.event(&pi, &Event::ToolEnd { call: "t1".into(), ok: true, summary: "noted Priya".into(), exit_code: None }, Provenance::Reported);
        s.approval_asked(&pi, "appr-1", "notes.write");
        s.approval_answered(&pi, "appr-1", true);
        s.event(
            &pi,
            &Event::Request { request_id: "forget-1".into(), prompt: "Forget Priya?".into(), options: vec!["Keep".into(), "Erase".into()] },
            Provenance::Reported,
        );
        s.question_answered(&pi, "forget-1", "Erase");
        s.close_turn(&pi, true);
        (s, pi)
    }

    fn card<'a>(s: &'a Store, id: &AgentId) -> &'a Card {
        s.agent(id).unwrap().cards().next().unwrap()
    }

    fn approval<'a>(s: &'a Store, id: &AgentId) -> &'a Approval {
        s.agent(id).unwrap().turns[0].items.iter().find_map(|i| if let Item::Approval(a) = i { Some(a) } else { None }).unwrap()
    }

    #[test]
    fn the_conversation_is_erased_with_the_reply_joined_across_its_chunks() {
        let (mut s, pi) = session();
        let needles = [Needle::of(SECRET)];
        let done = s.redact(&pi, &erasure(&needles));
        let a = s.agent(&pi).unwrap();
        let turn = &a.turns[0];
        assert_eq!(turn.prompt, format!("my sister is {MARKER}, remember that"));
        assert_eq!(a.meta.title, format!("my sister is {MARKER}, remember that"));
        // "Pri" and "ya" came as two chunks; the pane holds them as one block, matched whole.
        let text: String = turn.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t.text()) } else { None }).collect();
        assert_eq!(text, format!("Got it: {MARKER} is your sister."));
        let thinking: String = turn.items.iter().filter_map(|i| if let Item::Thinking(t) = i { Some(t.text()) } else { None }).collect();
        assert_eq!(thinking, format!("store {MARKER}"));
        let question = turn.items.iter().find_map(|i| if let Item::Question(q) = i { Some(q) } else { None }).unwrap();
        assert_eq!((question.prompt.as_str(), question.answer.as_str()), (format!("Forget {MARKER}?").as_str(), "Erase"));
        // prompt, title, reply, thinking, question.
        assert_eq!(done.places, 5);
        assert_eq!(a.erasures, vec![Erasure { request: "forget-1".into(), places: 9, at: 1_000 }]);
        assert!(matches!(turn.items.last(), Some(Item::Note(n)) if n == "Erased 9 places at your request."));

        let transcript = s.transcript(&pi, 5).unwrap();
        // Nothing read_agent shows holds the words, but the line saying what a call did: its
        // target is never hidden.
        for line in transcript.lines().filter(|l| !l.starts_with("[reported call]")) {
            assert!(!line.contains(SECRET), "{line}\n{transcript}");
        }
        assert!(transcript.contains("Erased 9 places at your request."));
    }

    #[test]
    fn a_tool_call_keeps_its_words_and_only_its_free_text_is_shown_masked_and_an_approval_never_is() {
        let (mut s, pi) = session();
        let before_args = card(&s, &pi).args.clone();
        let before_output = card(&s, &pi).output.all();
        let before_what = approval(&s, &pi).what.clone();
        let needles = [Needle::of(SECRET)];
        let done = s.redact(&pi, &erasure(&needles));

        // The records of what happened are untouched.
        let c = card(&s, &pi);
        assert_eq!((c.args.clone(), c.output.all(), c.summary.as_str()), (before_args, before_output, "noted Priya"));
        let ap = approval(&s, &pi);
        assert_eq!((ap.what.as_str(), ap.outcome), (before_what.as_str(), ApprovalOutcome::Allowed));
        // What is drawn of them is masked, keyed by the erasure.
        let shown = c.shown();
        assert_eq!(shown.args, json!({"path": "notes/Priya.md", "text": format!("sister: {MARKER}")}));
        assert_eq!(shown.output.all(), format!("saved {MARKER}\n"));
        assert_eq!(shown.summary, format!("noted {MARKER}"));
        assert_eq!((shown.state, shown.call.as_str()), (CallState::Ok, "t1"), "how it went is not touched");
        assert_eq!(c.mask.as_ref().unwrap().request, "forget-1");
        // What was done is never hidden: the target and the non-free-text arguments are shown as
        // they are, and the approval is not masked at all.
        assert_eq!(shown.target, "notes/Priya.md");
        assert_eq!(shown.args["path"], "notes/Priya.md");
        assert_eq!(ap.what, "notes.write");
        // The free-text argument, the output, the summary.
        assert_eq!(done.masked, 3);
    }

    #[test]
    fn the_saved_session_holds_the_words_only_inside_the_records_that_keep_them() {
        let (mut s, pi) = session();
        let needles = [Needle::of(SECRET)];
        s.redact(&pi, &erasure(&needles));
        let dir = std::env::temp_dir().join(format!("yantrik-erase-ui-{}-{}", std::process::id(), crate::agents::model::now()));
        let (path, contents) = s.file_of(&dir, &pi).unwrap();
        write_durably(&dir, &path, &contents).unwrap();
        assert!(!path.with_extension("jsonl.partial").exists());
        let saved = std::fs::read_to_string(&path).unwrap();
        for line in saved.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            if value["kind"] == "turn" {
                assert!(!value["prompt"].to_string().contains(SECRET));
                for item in value["items"].as_array().unwrap() {
                    if item.get("card").is_none() && item.get("approval").is_none() {
                        assert!(!item.to_string().contains(SECRET), "{item} holds the words");
                    }
                }
            }
        }
        // Read back, it is still shown masked.
        let back = Store::load(&dir, Box::new(|| 2_000));
        assert_eq!(card(&back, &pi).shown().args, json!({"path": "notes/Priya.md", "text": format!("sister: {MARKER}")}));
        assert_eq!(approval(&back, &pi).what, "notes.write");
        assert_eq!(back.agent(&pi).unwrap().erasures.len(), 1);
        let transcript = back.transcript(&pi, 5).unwrap();
        assert!(transcript.lines().filter(|l| !l.starts_with("[reported call]")).all(|l| !l.contains(SECRET)), "{transcript}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn composed_and_decomposed_words_are_erased_alike() {
        let mut s = Store::with_clock(Box::new(|| 1));
        let pi = AgentId::new("pi", "c-2");
        s.open_turn(&pi, "meet me at the Cafe\u{301} Lune");
        s.text(&pi, "The caf\u{e9} it is.");
        let needles = [Needle::of("Caf\u{e9} Lune"), Needle::of("cafe\u{301}")];
        let done = s.redact(&pi, &erasure(&needles));
        assert_eq!(done.places, 3, "the prompt, the title and the reply");
        let a = s.agent(&pi).unwrap();
        assert_eq!(a.turns[0].prompt, format!("meet me at the {MARKER}"));
        assert!(matches!(&a.turns[0].items[0], Item::Text(t) if t.text() == format!("The {MARKER} it is.")));
    }

    #[test]
    fn one_needle_erases_every_case_in_the_transcript_and_the_rest_keeps_its_case() {
        let mut s = Store::with_clock(Box::new(|| 1));
        let pi = AgentId::new("pi", "c-3");
        s.open_turn(&pi, "Remember THROWAWAY-ERASE2 for me");
        s.text(&pi, "Saved Throwaway-Erase2. Also throw");
        s.text(&pi, "away-erase2, OK?");
        s.event(&pi, &Event::Thinking { delta: "Keep THROWAWAY-erase2 SAFE".into() }, Provenance::Reported);
        s.close_turn(&pi, true);
        let needles = [Needle::of("throwaway-erase2")];
        let done = s.redact(&pi, &erasure(&needles));
        let a = s.agent(&pi).unwrap();
        let turn = &a.turns[0];
        assert_eq!(turn.prompt, format!("Remember {MARKER} for me"));
        assert_eq!(a.meta.title, format!("Remember {MARKER} for me"));
        let text: String = turn.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t.text()) } else { None }).collect();
        assert_eq!(text, format!("Saved {MARKER}. Also {MARKER}, OK?"));
        let thinking: String = turn.items.iter().filter_map(|i| if let Item::Thinking(t) = i { Some(t.text()) } else { None }).collect();
        assert_eq!(thinking, format!("Keep {MARKER} SAFE"));
        // prompt, title, two in the reply, thinking.
        assert_eq!(done.places, 5);

        let shown = s.transcript(&pi, 5).unwrap().to_lowercase();
        assert!(!shown.contains("throwaway-erase2"), "no case of the words is shown:\n{shown}");
        let dir = std::env::temp_dir().join(format!("yantrik-erase-ui-case-{}-{}", std::process::id(), crate::agents::model::now()));
        let (path, contents) = s.file_of(&dir, &pi).unwrap();
        write_durably(&dir, &path, &contents).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.to_lowercase().contains("throwaway-erase2"), "no case of the words is saved:\n{saved}");
        assert!(saved.contains("Remember ") && saved.contains(" SAFE"), "the words around keep their case");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_agent_the_store_does_not_hold_is_nothing_to_erase() {
        let mut s = Store::with_clock(Box::new(|| 1));
        let needles = [Needle::of(SECRET)];
        assert_eq!(s.redact(&AgentId::new("pi", "c-9"), &erasure(&needles)), ShellErased::default());
    }

    #[test]
    fn a_redact_from_a_feeder_is_refused_and_counted() {
        let (mut s, pi) = session();
        s.open_turn(&pi, "again");
        s.event(&pi, &Event::Redact { request_id: "forget-1".into(), needles: vec![Needle::of(SECRET)] }, Provenance::Reported);
        let a = s.agent(&pi).unwrap();
        assert!(a.refusals.last().unwrap().contains("only the host applies one"));
        assert!(a.erasures.is_empty());
    }

    #[test]
    fn a_large_transcript_searched_for_long_needles_is_measured_over_the_limit_without_searching() {
        use yantrik_harness::redact::{Search, MAX_WORK};
        let mut s = Store::with_clock(Box::new(|| 1));
        let pi = AgentId::new("pi", "c-big");
        s.open_turn(&pi, "write it all out");
        for _ in 0..100 {
            s.text(&pi, &"lorem ipsum dolor sit amet, Priya. ".repeat(100));
        }
        s.close_turn(&pi, true);
        let long: Vec<Needle> = (0..16).map(|i| Needle::of(&"x".repeat(4081 + i))).collect();
        let started = std::time::Instant::now();
        let plan = ErasurePlan::new(pi.clone(), s.erasure_texts(&pi));
        assert!(plan.cost(&Search::new(&long)) > MAX_WORK, "16 needles of about 4096 over the reply");
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "measured, not searched: {:?}", started.elapsed());
        // One short needle over the same transcript is well within it, and is applied.
        let short = [Needle::of("priya")];
        let mut plan = ErasurePlan::new(pi.clone(), s.erasure_texts(&pi));
        assert!(plan.cost(&Search::new(&short)) < MAX_WORK);
        plan.find(&Search::new(&short));
        let done = s.apply_erasure(&pi, &erasure(&short), &plan);
        assert!(done.places > 0);
        assert!(!s.transcript(&pi, 5).unwrap().contains("Priya"));
    }

    /// A save found stale after an erasure is dropped; the files it would have deleted are deleted
    /// by the next one instead, unless that agent is held again by then.
    #[test]
    fn a_stale_save_gives_back_the_deletes_it_took() {
        let dir = std::env::temp_dir().join("yantrik-erase-stale-save");
        let mut s = Store::with_clock(Box::new(|| 1));
        let gone = AgentId::new("pi", "c-gone");
        let back = AgentId::new("pi", "c-back");
        s.open_turn(&gone, "one");
        s.open_turn(&back, "two");
        let _ = s.take_dirty(&dir);
        assert!(s.remove_agent(&gone) && s.remove_agent(&back));
        let (_, deletes) = s.take_dirty(&dir);
        assert_eq!(deletes.len(), 2);
        // The agent `back` is held again before the next save: its file is written, not deleted.
        s.open_turn(&back, "three");
        s.mark_all_dirty(&deletes, &dir);
        let (writes, again) = s.take_dirty(&dir);
        assert_eq!(again, vec![file_for(&dir, &gone)]);
        assert!(writes.iter().any(|(path, _)| *path == file_for(&dir, &back)));
    }

    /// A command's arguments are the action itself — the text typed into a terminal, a query, an
    /// instruction handed on — so they are never masked; its output may be.
    #[test]
    fn a_commands_arguments_are_never_masked() {
        let mut s = Store::with_clock(Box::new(|| 1));
        let pi = AgentId::new("pi", "c-cmd");
        s.open_turn(&pi, "go");
        s.event(
            &pi,
            &Event::ToolStart { call: "t1".into(), name: "terminal.run".into(), target: String::new(), args: json!({"text": "echo Priya"}) },
            Provenance::Reported,
        );
        s.event(&pi, &Event::ToolOutput { call: "t1".into(), stream: Stream::Stdout, delta: "Priya\n".into() }, Provenance::Reported);
        let needles = [Needle::of(SECRET)];
        let done = s.redact(&pi, &erasure(&needles));
        let shown = card(&s, &pi).shown();
        assert_eq!(shown.args, json!({"text": "echo Priya"}), "what was typed is what was done");
        assert_eq!(shown.output.all(), format!("{MARKER}\n"));
        assert_eq!(done.masked, 1, "the output only");
    }
}
