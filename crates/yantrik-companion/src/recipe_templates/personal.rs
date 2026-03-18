//! Personal, home automation, health, and finance recipe templates.

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
    }
}

fn notify(msg: &str) -> RecipeStep {
    RecipeStep::Notify {
        message: msg.to_string(),
    }
}

pub fn templates() -> Vec<RecipeTemplate> {
    vec![
        // 42. Memory Review & Cleanup
        RecipeTemplate {
            id: "builtin_memory_review",
            name: "Memory Review",
            description: "Review stored memories, find conflicts, and clean up",
            category: "personal",
            keywords: &[
                "memory", "memories", "review memories", "memory cleanup",
                "what do you know", "clean memory", "memory audit",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("memory_stats", serde_json::json!({}), "stats"),
                    tool(
                        "review_memories",
                        serde_json::json!({"limit": 20}),
                        "memories",
                    ),
                    tool(
                        "resolve_conflicts",
                        serde_json::json!({}),
                        "conflicts",
                    ),
                    think(
                        "Memory review:\n\
                         Stats: {{stats}}\n\
                         Recent memories: {{memories}}\n\
                         Conflicts: {{conflicts}}\n\n\
                         Report:\n\
                         - Total memories stored\n\
                         - Any contradictions or conflicts found\n\
                         - Outdated memories that should be updated\n\
                         - Memory health assessment",
                        "review",
                    ),
                    notify("Memory Review:\n\n{{review}}"),
                ]
            },
            trigger: None,
        },
        // 43. Knowledge Summary
        RecipeTemplate {
            id: "builtin_knowledge_summary",
            name: "Knowledge Summary",
            description: "Summarize everything known about a specific topic",
            category: "personal",
            keywords: &[
                "what do you know about", "knowledge", "summary",
                "tell me what you know", "recall everything",
            ],
            required_vars: &[("topic", "Topic to summarize knowledge about")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{topic}}"}),
                        "memories",
                    ),
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{topic}} preferences opinions facts"}),
                        "deep_recall",
                    ),
                    think(
                        "Synthesize everything known about '{{topic}}':\n\
                         Direct memories: {{memories}}\n\
                         Deep recall: {{deep_recall}}\n\n\
                         Provide a comprehensive summary of what is stored \
                         about this topic. Organize by: facts, preferences, \
                         history/timeline, and open questions. \
                         If little is known, say so honestly.",
                        "summary",
                    ),
                    notify("Knowledge Summary — {{topic}}:\n\n{{summary}}"),
                ]
            },
            trigger: None,
        },
        // 44. Smart Home Scene
        RecipeTemplate {
            id: "builtin_smart_home_scene",
            name: "Smart Home Scene",
            description: "Configure smart home devices for a scene (movie, sleep, etc.)",
            category: "personal",
            keywords: &[
                "smart home", "scene", "home assistant", "lights",
                "thermostat", "movie mode", "sleep mode", "bedtime",
            ],
            required_vars: &[("scene", "Scene name (e.g., 'movie night', 'bedtime', 'working from home')")],
            steps: || {
                vec![
                    tool(
                        "ha_list_entities",
                        serde_json::json!({}),
                        "entities",
                    ),
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{scene}} smart home preferences settings"}),
                        "preferences",
                    ),
                    think(
                        "Configure smart home for '{{scene}}' scene:\n\
                         Available entities: {{entities}}\n\
                         User preferences: {{preferences}}\n\n\
                         Determine appropriate settings for each device \
                         (lights, thermostat, speakers, etc.) for this scene. \
                         List the specific HA service calls needed.",
                        "plan",
                    ),
                    notify("Smart Home — {{scene}}:\n\n{{plan}}\n\nSay 'apply' to set these."),
                ]
            },
            trigger: None,
        },
        // 45. Energy Check
        RecipeTemplate {
            id: "builtin_energy_check",
            name: "Energy Check",
            description: "Check smart home device states and suggest energy savings",
            category: "personal",
            keywords: &[
                "energy", "power", "electricity", "save energy",
                "what's on", "devices on", "energy usage",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "ha_list_entities",
                        serde_json::json!({}),
                        "entities",
                    ),
                    think(
                        "Energy usage check:\n\
                         Device states: {{entities}}\n\n\
                         Report:\n\
                         - Devices currently on\n\
                         - Devices that could be turned off to save energy\n\
                         - Any devices left on that are unusual\n\
                         - Energy-saving suggestions",
                        "report",
                    ),
                    notify("Energy Check:\n\n{{report}}"),
                ]
            },
            trigger: None,
        },
        // 46. Budget Review
        RecipeTemplate {
            id: "builtin_budget_review",
            name: "Budget Review",
            description: "Review spending notes and provide a budget overview",
            category: "personal",
            keywords: &[
                "budget", "spending", "money", "finances",
                "expenses", "how much spent", "financial review",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "spending budget expenses purchases money"}),
                        "financial_memories",
                    ),
                    think(
                        "Budget review based on stored information:\n\
                         Financial memories: {{financial_memories}}\n\n\
                         If financial data is available:\n\
                         - Spending summary by category\n\
                         - Notable expenses\n\
                         - Trends or patterns\n\
                         - Suggestions for saving\n\n\
                         If no financial data stored, suggest the user start \
                         logging expenses with 'remember I spent X on Y'.",
                        "review",
                    ),
                    notify("Budget Review:\n\n{{review}}"),
                ]
            },
            trigger: None,
        },
        // 47. Subscription Audit
        RecipeTemplate {
            id: "builtin_subscription_audit",
            name: "Subscription Audit",
            description: "List and review active subscriptions for potential savings",
            category: "personal",
            keywords: &[
                "subscription", "subscriptions", "recurring", "monthly charges",
                "cancel subscription", "what am i paying for",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "subscriptions services paying monthly yearly"}),
                        "subs",
                    ),
                    tool(
                        "email_search",
                        serde_json::json!({"query": "subscription receipt renewal billing", "limit": 20}),
                        "billing_emails",
                    ),
                    think(
                        "Audit subscriptions:\n\
                         Known subscriptions: {{subs}}\n\
                         Billing emails: {{billing_emails}}\n\n\
                         Create a list of all identified subscriptions with:\n\
                         - Service name\n\
                         - Estimated cost (if known)\n\
                         - Usage assessment (essential / nice-to-have / unused)\n\
                         - Total estimated monthly spend\n\
                         Suggest which could be cancelled or downgraded.",
                        "audit",
                    ),
                    notify("Subscription Audit:\n\n{{audit}}"),
                ]
            },
            trigger: None,
        },
        // 48. Workout Log
        RecipeTemplate {
            id: "builtin_workout_log",
            name: "Workout Log",
            description: "Log a workout and track exercise progress",
            category: "personal",
            keywords: &[
                "workout", "exercise", "gym", "run", "training",
                "log workout", "fitness", "worked out",
            ],
            required_vars: &[("workout", "Description of the workout (e.g., '30 min run, 5K')")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "workout exercise fitness progress recent"}),
                        "history",
                    ),
                    tool(
                        "remember",
                        serde_json::json!({"content": "Workout logged: {{workout}}"}),
                        "_saved",
                    ),
                    think(
                        "Workout logged: {{workout}}\n\
                         Previous workouts: {{history}}\n\n\
                         Provide:\n\
                         - Confirmation of logged workout\n\
                         - Comparison to previous similar workouts (if any)\n\
                         - Streak or consistency note\n\
                         - Brief encouragement\n\
                         Keep it short and motivating.",
                        "summary",
                    ),
                    notify("Workout Logged:\n\n{{summary}}"),
                ]
            },
            trigger: None,
        },
        // 49. Journal Entry
        RecipeTemplate {
            id: "builtin_journal_entry",
            name: "Journal Entry",
            description: "Write a journal entry with guided prompts",
            category: "personal",
            keywords: &[
                "journal", "diary", "write journal", "journal entry",
                "daily journal", "reflection", "gratitude",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "today activities events mood feelings"}),
                        "today",
                    ),
                    think(
                        "Create journal prompts based on today's context:\n\
                         Today's events: {{today}}\n\n\
                         Generate 3 thoughtful prompts:\n\
                         1. About something specific that happened today\n\
                         2. About feelings or reactions\n\
                         3. About gratitude or looking forward\n\
                         Make them personal based on the day's events.",
                        "prompts",
                    ),
                    notify("Journal Prompts:\n\n{{prompts}}\n\nShare your thoughts and I'll save your journal entry."),
                ]
            },
            trigger: None,
        },
        // 50. Decision Helper
        RecipeTemplate {
            id: "builtin_decision_helper",
            name: "Decision Helper",
            description: "Help make a decision with structured pros/cons analysis",
            category: "personal",
            keywords: &[
                "decide", "decision", "should i", "pros cons",
                "help me choose", "dilemma", "options",
                "what should i do", "which one",
            ],
            required_vars: &[("decision", "The decision or options you're considering")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{decision}} preferences values priorities"}),
                        "context",
                    ),
                    think(
                        "Help with decision: {{decision}}\n\
                         User context/values: {{context}}\n\n\
                         Structure the analysis:\n\
                         1. Clarify the options\n\
                         2. Pros and cons of each option\n\
                         3. What matters most (based on known preferences)\n\
                         4. Potential regret analysis (which choice would you regret more?)\n\
                         5. Recommendation with reasoning\n\
                         Be balanced but give a clear recommendation.",
                        "analysis",
                    ),
                    notify("Decision Analysis — {{decision}}:\n\n{{analysis}}"),
                ]
            },
            trigger: None,
        },
    ]
}
