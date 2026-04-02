//! Daily routine and productivity recipe templates.

use super::RecipeTemplate;
use crate::recipe::RecipeStep;

/// Helper to build a Tool step concisely.
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

pub fn templates() -> Vec<RecipeTemplate> {
    vec![
        // 1. Morning Briefing
        RecipeTemplate {
            id: "builtin_morning_briefing",
            name: "Morning Briefing",
            description: "Get weather, calendar, emails, and tasks summary to start your day",
            category: "routines",
            keywords: &[
                "morning", "briefing", "brief", "start day", "good morning",
                "wake up", "daily summary", "morning report",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("get_weather", serde_json::json!({}), "weather"),
                    tool(
                        "calendar_today",
                        serde_json::json!({}),
                        "calendar",
                    ),
                    tool(
                        "email_list",
                        serde_json::json!({"limit": 10, "unread": true}),
                        "emails",
                    ),
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "pending", "limit": 10}),
                        "tasks",
                    ),
                    think(
                        "Create a concise morning briefing from:\n\
                         Weather: {{weather}}\n\
                         Calendar: {{calendar}}\n\
                         Unread emails: {{emails}}\n\
                         Pending tasks: {{tasks}}\n\n\
                         Format: Start with weather, then today's schedule, \
                         important emails, and top tasks. Keep it under 200 words.",
                        "briefing",
                    ),
                    notify("Good morning! Here's your briefing:\n\n{{briefing}}"),
                ]
            },
            trigger: None,
        },
        // 2. Evening Reflection
        RecipeTemplate {
            id: "builtin_evening_reflection",
            name: "Evening Reflection",
            description: "Reflect on the day's activities and save insights to memory",
            category: "routines",
            keywords: &[
                "evening", "reflection", "end of day", "day review",
                "what did i do", "reflect", "journal", "wind down",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "today activities conversations topics"}),
                        "today_memories",
                    ),
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "completed", "limit": 10}),
                        "completed_tasks",
                    ),
                    think(
                        "Based on today's activities and completed tasks, write a brief \
                         evening reflection:\n\
                         Memories: {{today_memories}}\n\
                         Completed: {{completed_tasks}}\n\n\
                         Include: what went well, what could improve, and one thing to \
                         be grateful for. Keep it personal and warm. 3-4 sentences.",
                        "reflection",
                    ),
                    tool(
                        "remember",
                        serde_json::json!({"content": "Evening reflection: {{reflection}}"}),
                        "_saved",
                    ),
                    notify("Evening reflection:\n\n{{reflection}}"),
                ]
            },
            trigger: None,
        },
        // 3. Weekly Review
        RecipeTemplate {
            id: "builtin_weekly_review",
            name: "Weekly Review",
            description: "Review the week's accomplishments and plan for next week",
            category: "routines",
            keywords: &[
                "weekly", "review", "week summary", "week recap",
                "plan next week", "weekly planning",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "this week activities accomplishments progress"}),
                        "week_memories",
                    ),
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "completed", "limit": 20}),
                        "completed",
                    ),
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "pending", "limit": 20}),
                        "pending",
                    ),
                    think(
                        "Create a weekly review:\n\
                         Week's activities: {{week_memories}}\n\
                         Completed tasks: {{completed}}\n\
                         Still pending: {{pending}}\n\n\
                         Format:\n\
                         - Wins this week (2-3 highlights)\n\
                         - Carried over (tasks that need attention)\n\
                         - Focus for next week (2-3 priorities)\n\
                         Keep it actionable.",
                        "review",
                    ),
                    notify("Weekly Review:\n\n{{review}}"),
                ]
            },
            trigger: None,
        },
        // 4. Daily Standup
        RecipeTemplate {
            id: "builtin_daily_standup",
            name: "Daily Standup",
            description: "Generate a daily standup summary: done, doing, blockers",
            category: "routines",
            keywords: &[
                "standup", "stand up", "scrum", "daily update",
                "status update", "what am i working on",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "completed", "limit": 5}),
                        "done",
                    ),
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "in_progress", "limit": 5}),
                        "in_progress",
                    ),
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "pending", "limit": 5}),
                        "pending",
                    ),
                    think(
                        "Format a daily standup update:\n\
                         Done: {{done}}\n\
                         In progress: {{in_progress}}\n\
                         Pending: {{pending}}\n\n\
                         Format as:\n\
                         Yesterday: (completed items)\n\
                         Today: (in-progress + top pending)\n\
                         Blockers: (any obvious blockers, or 'None')\n\
                         Keep each section to 1-2 bullet points.",
                        "standup",
                    ),
                    notify("Daily Standup:\n\n{{standup}}"),
                ]
            },
            trigger: None,
        },
        // 5. Focus Session
        RecipeTemplate {
            id: "builtin_focus_session",
            name: "Focus Session",
            description: "Start a focused work session with timer and notification",
            category: "routines",
            keywords: &[
                "focus", "deep work", "concentration", "do not disturb",
                "focus mode", "work session", "pomodoro",
            ],
            required_vars: &[("duration_minutes", "Focus session length in minutes (default: 25)")],
            steps: || {
                vec![
                    notify("Focus session started. I'll minimize interruptions for {{duration_minutes}} minutes."),
                    tool(
                        "timer",
                        serde_json::json!({"minutes": "{{duration_minutes}}"}),
                        "timer_result",
                    ),
                    notify("Focus session complete! Time for a break."),
                ]
            },
            trigger: None,
        },
        // 6. End of Day
        RecipeTemplate {
            id: "builtin_end_of_day",
            name: "End of Day Wrap-up",
            description: "Summarize open tasks and set reminders for tomorrow",
            category: "routines",
            keywords: &[
                "end of day", "wrap up", "close out", "sign off",
                "leaving", "heading out", "done for today",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "list_tasks",
                        serde_json::json!({"status": "in_progress", "limit": 10}),
                        "open_tasks",
                    ),
                    tool(
                        "show_open_loops",
                        serde_json::json!({}),
                        "open_loops",
                    ),
                    think(
                        "Create an end-of-day summary:\n\
                         Open tasks: {{open_tasks}}\n\
                         Open loops: {{open_loops}}\n\n\
                         List what needs attention tomorrow. Identify the single \
                         most important thing to tackle first. Keep it brief.",
                        "eod_summary",
                    ),
                    notify("End of day wrap-up:\n\n{{eod_summary}}"),
                ]
            },
            trigger: None,
        },
        // 7. Habit Check-in
        RecipeTemplate {
            id: "builtin_habit_checkin",
            name: "Habit Check-in",
            description: "Review daily habits and track completion",
            category: "routines",
            keywords: &[
                "habit", "habits", "check in", "routine", "tracker",
                "daily habits", "habit check",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "habits goals daily routine tracking"}),
                        "habits",
                    ),
                    think(
                        "Based on known habits and routines: {{habits}}\n\n\
                         Create a simple checklist of the user's tracked habits. \
                         If no habits are stored, suggest starting with 3 common ones \
                         (exercise, reading, hydration). Format as a checkbox list.",
                        "checklist",
                    ),
                    notify("Habit Check-in:\n\n{{checklist}}"),
                ]
            },
            trigger: None,
        },
        // 8. Goal Review
        RecipeTemplate {
            id: "builtin_goal_review",
            name: "Goal Review",
            description: "Review progress on goals and suggest next actions",
            category: "routines",
            keywords: &[
                "goal", "goals", "progress", "objective", "target",
                "milestone", "goal review", "how am i doing",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "goals objectives targets milestones progress"}),
                        "goals",
                    ),
                    tool(
                        "list_tasks",
                        serde_json::json!({"limit": 20}),
                        "tasks",
                    ),
                    think(
                        "Review goal progress:\n\
                         Known goals: {{goals}}\n\
                         Current tasks: {{tasks}}\n\n\
                         For each goal: assess progress (on track / needs attention / at risk), \
                         and suggest one concrete next action. If no goals are stored, \
                         suggest the user set 2-3 goals.",
                        "review",
                    ),
                    notify("Goal Review:\n\n{{review}}"),
                ]
            },
            trigger: None,
        },
        // 9. Quick Capture
        RecipeTemplate {
            id: "builtin_quick_capture",
            name: "Quick Capture",
            description: "Quickly capture a thought, idea, or note to memory",
            category: "routines",
            keywords: &[
                "capture", "note", "jot down", "save thought",
                "remember this", "quick note", "idea",
            ],
            required_vars: &[("content", "The thought or idea to capture")],
            steps: || {
                vec![
                    tool(
                        "remember",
                        serde_json::json!({"content": "{{content}}"}),
                        "saved",
                    ),
                    notify("Captured: {{content}}"),
                ]
            },
            trigger: None,
        },
    ]
}
