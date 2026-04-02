//! Communication and email recipe templates.

use super::RecipeTemplate;
use crate::recipe::RecipeStep;

fn tool(name: &str, args: serde_json::Value, store: &str) -> RecipeStep {
    RecipeStep::Tool {
        tool_name: name.to_string(),
        args,
        store_as: store.to_string(),
        on_error: Default::default(),
    }
}

fn think(prompt: &str, store: &str) -> RecipeStep {
    RecipeStep::Think {
        prompt: prompt.to_string(),
        store_as: store.to_string(),
        fallback_template: None,
    }
}

fn notify(msg: &str) -> RecipeStep {
    RecipeStep::Notify {
        message: msg.to_string(),
    }
}

fn ask(question: &str, store: &str) -> RecipeStep {
    RecipeStep::AskUser {
        question: question.to_string(),
        store_as: store.to_string(),
        choices: None,
    }
}

pub fn templates() -> Vec<RecipeTemplate> {
    vec![
        // 10. Email Triage
        RecipeTemplate {
            id: "builtin_email_triage",
            name: "Email Triage",
            description: "Check unread emails and categorize by priority",
            category: "communication",
            keywords: &[
                "email", "triage", "inbox", "unread", "check email",
                "email priority", "sort email", "inbox zero",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "email_list",
                        serde_json::json!({"limit": 20, "unread": true}),
                        "emails",
                    ),
                    think(
                        "Triage these unread emails: {{emails}}\n\n\
                         Categorize each as:\n\
                         - URGENT: needs response today\n\
                         - IMPORTANT: needs response this week\n\
                         - LOW: informational, no response needed\n\n\
                         For each email: one-line summary + category. \
                         If no unread emails, say 'Inbox clear!'",
                        "triage",
                    ),
                    notify("Email Triage:\n\n{{triage}}"),
                ]
            },
            trigger: None,
        },
        // 11. Email Draft Reply
        RecipeTemplate {
            id: "builtin_email_draft_reply",
            name: "Draft Email Reply",
            description: "Read a specific email and draft a contextual reply",
            category: "communication",
            keywords: &[
                "reply", "respond", "email reply", "draft reply",
                "write back", "respond to email", "answer email",
            ],
            required_vars: &[("email_id", "ID of the email to reply to")],
            steps: || {
                vec![
                    tool(
                        "email_read",
                        serde_json::json!({"id": "{{email_id}}"}),
                        "email",
                    ),
                    tool(
                        "recall",
                        serde_json::json!({"query": "email context {{email_id}}"}),
                        "context",
                    ),
                    think(
                        "Draft a reply to this email:\n\
                         Email: {{email}}\n\
                         Context from memory: {{context}}\n\n\
                         Write a professional, concise reply that addresses \
                         the key points. Match the sender's tone.",
                        "draft",
                    ),
                    notify("Draft reply:\n\n{{draft}}"),
                ]
            },
            trigger: None,
        },
        // 12. Email Digest
        RecipeTemplate {
            id: "builtin_email_digest",
            name: "Email Digest",
            description: "Summarize recent emails into a quick digest",
            category: "communication",
            keywords: &[
                "email digest", "email summary", "summarize emails",
                "email overview", "what emails", "catch up on email",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "email_list",
                        serde_json::json!({"limit": 30}),
                        "emails",
                    ),
                    think(
                        "Create a concise email digest from: {{emails}}\n\n\
                         Group by: action needed, FYI/newsletters, automated/notifications.\n\
                         For action items: sender, subject, what's needed.\n\
                         For FYI: one-line summary each.\n\
                         Skip automated notifications unless important.",
                        "digest",
                    ),
                    notify("Email Digest:\n\n{{digest}}"),
                ]
            },
            trigger: None,
        },
        // 13. Email Cleanup
        RecipeTemplate {
            id: "builtin_email_cleanup",
            name: "Email Cleanup",
            description: "Archive old read emails to clean up inbox",
            category: "communication",
            keywords: &[
                "clean inbox", "email cleanup", "archive emails",
                "clear inbox", "inbox cleanup", "tidy email",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "email_list",
                        serde_json::json!({"limit": 50}),
                        "emails",
                    ),
                    think(
                        "From these emails: {{emails}}\n\n\
                         Identify emails safe to archive (read, older than a week, \
                         no action needed). List their IDs and subjects. \
                         Do NOT suggest archiving unread or flagged emails.",
                        "to_archive",
                    ),
                    notify("Cleanup suggestions:\n\n{{to_archive}}\n\nSay 'go ahead' to archive these."),
                ]
            },
            trigger: None,
        },
        // 14. Draft Message
        RecipeTemplate {
            id: "builtin_draft_message",
            name: "Draft Message",
            description: "Draft a message with the right tone for any context",
            category: "communication",
            keywords: &[
                "draft", "write message", "compose", "help me write",
                "message", "text", "word it",
            ],
            required_vars: &[
                ("recipient", "Who the message is for"),
                ("purpose", "What the message is about"),
            ],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{recipient}} communication preferences tone"}),
                        "context",
                    ),
                    think(
                        "Draft a message:\n\
                         To: {{recipient}}\n\
                         Purpose: {{purpose}}\n\
                         Context from memory: {{context}}\n\n\
                         Write a clear, appropriate message. Match the \
                         formality level to the relationship.",
                        "draft",
                    ),
                    notify("Draft message to {{recipient}}:\n\n{{draft}}"),
                ]
            },
            trigger: None,
        },
        // 15. Thank You Note
        RecipeTemplate {
            id: "builtin_thank_you_note",
            name: "Thank You Note",
            description: "Draft a personalized thank you message",
            category: "communication",
            keywords: &[
                "thank you", "thanks", "grateful", "appreciation",
                "thank you note", "thank someone",
            ],
            required_vars: &[
                ("person", "Who to thank"),
                ("reason", "What to thank them for"),
            ],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{person}} relationship interactions"}),
                        "context",
                    ),
                    think(
                        "Write a heartfelt thank you note:\n\
                         To: {{person}}\n\
                         For: {{reason}}\n\
                         Relationship context: {{context}}\n\n\
                         Make it personal, specific, and genuine. \
                         Reference shared experiences if available. 3-5 sentences.",
                        "note",
                    ),
                    notify("Thank you note for {{person}}:\n\n{{note}}"),
                ]
            },
            trigger: None,
        },
        // 16. Meeting Prep
        RecipeTemplate {
            id: "builtin_meeting_prep",
            name: "Meeting Preparation",
            description: "Prepare for a meeting with context, agenda, and talking points",
            category: "communication",
            keywords: &[
                "meeting", "prep", "prepare", "meeting prep",
                "talking points", "agenda", "before meeting",
            ],
            required_vars: &[("meeting_topic", "What the meeting is about")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{meeting_topic}} history context decisions"}),
                        "memory_context",
                    ),
                    tool(
                        "calendar_list_events",
                        serde_json::json!({}),
                        "calendar",
                    ),
                    think(
                        "Prepare for a meeting about: {{meeting_topic}}\n\
                         Past context: {{memory_context}}\n\
                         Calendar: {{calendar}}\n\n\
                         Create:\n\
                         1. Key context (what's been discussed before)\n\
                         2. Suggested agenda (3-5 items)\n\
                         3. Talking points (key things to raise)\n\
                         4. Questions to ask\n\
                         Keep it concise and actionable.",
                        "prep",
                    ),
                    notify("Meeting Prep — {{meeting_topic}}:\n\n{{prep}}"),
                ]
            },
            trigger: None,
        },
        // 17. Birthday Reminder Setup
        RecipeTemplate {
            id: "builtin_birthday_reminder",
            name: "Birthday Reminder",
            description: "Save a birthday and set up annual reminders",
            category: "communication",
            keywords: &[
                "birthday", "anniversary", "remind birthday",
                "save birthday", "remember birthday",
            ],
            required_vars: &[
                ("person", "Whose birthday"),
                ("date", "Birthday date (e.g., March 15)"),
            ],
            steps: || {
                vec![
                    tool(
                        "remember",
                        serde_json::json!({"content": "{{person}}'s birthday is on {{date}}"}),
                        "_saved",
                    ),
                    tool(
                        "set_reminder",
                        serde_json::json!({
                            "message": "{{person}}'s birthday is coming up on {{date}}! Time to plan something.",
                            "when": "{{date}}"
                        }),
                        "_reminder",
                    ),
                    notify("Saved {{person}}'s birthday ({{date}}) and set up a reminder."),
                ]
            },
            trigger: None,
        },
    ]
}
