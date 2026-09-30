//! Formations — recipes whose steps are catalog agents (design/desk-and-mind-2026-09-23.md,
//! section 6).
//!
//! Each is built from Agent steps: a turn handed to a role from the agent catalog, its answer
//! kept for the steps after it. Agent steps that do not read each other's answers work at the same
//! time — at most three at once — and a step that reads one waits for it, so the Council's three
//! seats and the Writers' room's three writers work in parallel, and the rest go in order.
//!
//! Each takes one input a run must be given (`required_vars`) and names its roles through
//! variables with defaults ([`defaults`]), so a run can seat other roles: `run_recipe` with
//! `{"question": "…", "seat_2": "reviewer"}`.
//!
//! Each tells the person one thing, once: the recipe's completion message, which carries what its
//! last step kept. No Notify step and no question on the way — a formation's messages would
//! otherwise overwrite each other in the companion's one message slot (#187).

use super::RecipeTemplate;
use crate::recipe::{Condition, RecipeStep};

pub const COUNCIL: &str = "builtin_formation_council";
pub const RED_TEAM: &str = "builtin_formation_red_team";
pub const BUILD: &str = "builtin_formation_build";
pub const WRITERS_ROOM: &str = "builtin_formation_writers_room";

fn agent(role: &str, prompt: &str, store_as: &str, context: Option<&str>) -> RecipeStep {
    RecipeStep::Agent {
        role: role.to_string(),
        prompt: prompt.to_string(),
        store_as: store_as.to_string(),
        context: context.map(str::to_string),
        title: None,
    }
}

/// An Agent step named on screen by `title` rather than by its role alone.
fn titled(title: &str, step: RecipeStep) -> RecipeStep {
    match step {
        RecipeStep::Agent { role, prompt, store_as, context, .. } => {
            RecipeStep::Agent { role, prompt, store_as, context, title: Some(title.to_string()) }
        }
        other => other,
    }
}

fn format(template: &str, store_as: &str) -> RecipeStep {
    RecipeStep::Format { input_vars: Vec::new(), template: template.to_string(), store_as: store_as.to_string() }
}

/// The inputs each formation fills in by itself when a run is not given them: name, value, and
/// what it is for.
pub fn defaults(template_id: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    match template_id {
        COUNCIL => &[
            ("seat_1", "researcher", "the first seat's role"),
            ("seat_2", "red-team", "the second seat's role"),
            ("seat_3", "planner", "the third seat's role"),
            ("chair", "chair", "the role that weighs the three answers"),
        ],
        RED_TEAM => &[
            ("author", "planner", "the role that proposes and revises"),
            ("attacker", "red-team", "the role that attacks"),
        ],
        WRITERS_ROOM => &[
            ("voice_1", "Voice A", "the first writer's voice"),
            ("voice_2", "Voice B", "the second writer's voice"),
            ("voice_3", "Narrator", "the third writer's voice"),
        ],
        _ => &[],
    }
}

pub fn templates() -> Vec<RecipeTemplate> {
    vec![
        RecipeTemplate {
            id: COUNCIL,
            name: "Council",
            description: "Three roles answer one question independently, at the same time; the Chair \
                          weighs the three and gives a verdict with its confidence.",
            category: "formations",
            keywords: &["council", "second opinion", "several opinions", "weigh the options", "agents debate"],
            required_vars: &[("question", "The question the council is to answer")],
            steps: || {
                let seat = "Answer this question on your own, as well as you can. Two other agents are \
                            answering it too, independently; do not guess what they will say, and do not \
                            hedge toward them.\n\nThe question: {{question}}";
                vec![
                    agent("{{seat_1}}", seat, "answer_1", None),
                    agent("{{seat_2}}", seat, "answer_2", None),
                    agent("{{seat_3}}", seat, "answer_3", None),
                    agent(
                        "{{chair}}",
                        "Three agents answered this question independently: {{question}}\n\nWeigh their \
                         answers, which follow. Name where they agree and where they disagree, whose \
                         argument wins each disagreement, and give a verdict with how confident you are.",
                        "verdict",
                        Some("From the {{seat_1}}:\n{{answer_1}}\n\nFrom the {{seat_2}}:\n{{answer_2}}\n\nFrom the {{seat_3}}:\n{{answer_3}}"),
                    ),
                ]
            },
            trigger: None,
        },
        RecipeTemplate {
            id: RED_TEAM,
            name: "Red team",
            description: "The author proposes, the Red team attacks, the author revises — two rounds — \
                          and the revised proposal is the result.",
            category: "formations",
            keywords: &["red team", "attack my plan", "stress test", "poke holes", "devil's advocate"],
            required_vars: &[("brief", "What the author is to propose: the goal, and anything it must meet")],
            steps: || {
                vec![
                    agent(
                        "{{author}}",
                        "Propose: {{brief}}\n\nWrite the proposal in full. It will be attacked by someone \
                         who wants it to fail, and then you will revise it.",
                        "proposal_1",
                        None,
                    ),
                    agent(
                        "{{attacker}}",
                        "Attack this proposal, written for: {{brief}}",
                        "attack_1",
                        Some("The proposal:\n{{proposal_1}}"),
                    ),
                    agent(
                        "{{author}}",
                        "Revise your proposal for: {{brief}}\n\nAnswer every attack below: fix what it \
                         found, or say why it does not hold. Give the revised proposal in full.",
                        "proposal_2",
                        Some("Your proposal:\n{{proposal_1}}\n\nThe attack:\n{{attack_1}}"),
                    ),
                    agent(
                        "{{attacker}}",
                        "Attack the revised proposal, written for: {{brief}}\n\nSay first which of your \
                         earlier attacks it answered, then what is still wrong with it.",
                        "attack_2",
                        Some("Your earlier attack:\n{{attack_1}}\n\nThe revised proposal:\n{{proposal_2}}"),
                    ),
                    agent(
                        "{{author}}",
                        "Revise your proposal for: {{brief}} one last time. Answer every attack below, \
                         give the final proposal in full, and end with what is still open.",
                        "proposal",
                        Some("Your revised proposal:\n{{proposal_2}}\n\nThe second attack:\n{{attack_2}}"),
                    ),
                ]
            },
            trigger: None,
        },
        RecipeTemplate {
            id: BUILD,
            name: "Build",
            description: "The Planner plans a change, the Coder makes it, the Reviewer reviews it; the \
                          Reviewer's findings go back to the Coder once, then it stops.",
            category: "formations",
            keywords: &["build", "make the change", "implement", "plan code review", "code it"],
            required_vars: &[("goal", "The change to make, and where")],
            steps: || {
                vec![
                    // 0: what the Coder's first round has to answer.
                    format("None yet: this is the first version.", "findings"),
                    // 1
                    agent(
                        "planner",
                        "Plan this change: {{goal}}\n\nName the files to touch, the steps in order, and \
                         how to prove it works.",
                        "plan",
                        None,
                    ),
                    // 2: the Coder, and the loop's way back.
                    agent(
                        "coder",
                        "Make this change: {{goal}}\n\nFollow the plan below, and address the reviewer's \
                         findings if there are any.",
                        "change",
                        Some("The plan:\n{{plan}}\n\nThe reviewer's findings to address:\n{{findings}}"),
                    ),
                    // 3
                    agent(
                        "reviewer",
                        "Review the change just made for: {{goal}}\n\nAfter your answer, end with one line \
                         on its own: `ROUND: fix` if the Coder must change anything, or `ROUND: ship` if not.",
                        "review",
                        Some("The plan:\n{{plan}}\n\nWhat the Coder did:\n{{change}}"),
                    ),
                    // 4: shipped, or the Coder has had its second round — to the result.
                    RecipeStep::JumpIf {
                        condition: Condition::Or {
                            conditions: vec![
                                Condition::Not {
                                    inner: Box::new(Condition::VarContains { var: "review".into(), substring: "ROUND: fix".into() }),
                                },
                                Condition::VarContains { var: "findings".into(), substring: "ROUND:".into() },
                            ],
                        },
                        target_step: 7,
                    },
                    // 5, 6: the findings go back to the Coder, once.
                    format("{{review}}", "findings"),
                    RecipeStep::JumpIf { condition: Condition::VarExists { var: "findings".into() }, target_step: 2 },
                    // 7
                    format("The change:\n{{change}}\n\nThe review:\n{{review}}", "result"),
                ]
            },
            trigger: None,
        },
        RecipeTemplate {
            id: WRITERS_ROOM,
            name: "Writers' room",
            description: "A showrunner's beats; three writers each write one voice's lines for every \
                          beat, at the same time; the Scribe assembles the scene.",
            category: "formations",
            keywords: &["writers room", "writers' room", "write a scene", "script", "dialogue from beats"],
            required_vars: &[("beats", "The showrunner's beats, in order — separate them with ; or new lines")],
            steps: || {
                // Each writer is named by its voice, on screen and in its brief (#194): three stages
                // that all read "Writer · deepseek" could not be told apart.
                let writer = |n: u8| {
                    let prompt = format!(
                        "You are one of three writers in a writers' room. The cast, in order: {{{{voice_1}}}}, \
                         {{{{voice_2}}}}, {{{{voice_3}}}}. You write only for {{{{voice_{n}}}}}. For every beat \
                         below, in order, write that voice's lines — none if it would not speak — each marked \
                         with the beat it belongs to. The other writers write the other voices; do not write \
                         theirs."
                    );
                    titled(
                        &format!("Writer ({{{{voice_{n}}}}})"),
                        agent("writer", &prompt, &format!("lines_{n}"), Some("The beats:\n{{beats}}")),
                    )
                };
                vec![
                    writer(1),
                    writer(2),
                    writer(3),
                    agent(
                        "scribe",
                        "Do not summarise this time: assemble the scene. Three writers each wrote one \
                         voice's lines for the beats below (the cast, in order: {{voice_1}}, {{voice_2}}, {{voice_3}}). Put the lines \
                         together beat by beat, in the beats' order, each under the voice that speaks it. \
                         Keep every line exactly as written, and mark a beat nobody wrote for.",
                        "script",
                        Some("The beats:\n{{beats}}\n\nThe first voice's lines:\n{{lines_1}}\n\nThe second voice's lines:\n{{lines_2}}\n\nThe third voice's lines:\n{{lines_3}}"),
                    ),
                ]
            },
            trigger: None,
        },
    ]
}
