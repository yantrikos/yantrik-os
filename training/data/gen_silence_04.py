#!/usr/bin/env python3
"""Generate 500 JSONL training examples for proactivity silence matrix:
memory-triggered proactive (250) + social/relationship triggers (250)."""
import json, random

random.seed(2604)

NAMES = [
    "Sam", "Alex", "Jordan", "Riley", "Morgan",
    "Casey", "Taylor", "Quinn", "Kai", "Avery",
    "Drew", "Sasha", "Remy", "Noor", "Finley",
]

BOND_LEVELS = ["stranger", "acquaintance", "familiar", "trusted", "deep", "partner"]

ACTIVITIES = [
    "coding (focused)", "browsing casually", "idle", "in a meeting",
    "writing a document", "watching a video", "afk", "chatting with someone",
    "reading email", "terminal session",
]

TIMES = [
    "08:15 Monday", "09:30 Tuesday", "10:45 Wednesday", "13:00 Thursday",
    "14:30 Friday", "16:00 Monday", "19:20 Saturday", "21:00 Sunday",
    "07:00 Wednesday", "11:15 Friday", "22:30 Tuesday", "06:45 Thursday",
    "15:00 Saturday", "17:45 Monday", "20:00 Wednesday",
]

FRIEND_NAMES = [
    "Mom", "Dad", "Lisa", "Marcus", "Priya", "Jake", "Elena",
    "Carlos", "Aisha", "Ben", "Sophie", "Tom", "Rina", "David", "Mei",
]

examples = []


def system_prompt(name, bond_level, time_str, activity):
    return (
        f"You are Yantrik, {name}'s personal companion running as their desktop shell.\n"
        f"Bond level: {bond_level}.\n"
        f"You receive proactive triggers about memory insights and social awareness.\n"
        f"For memory insights: only surface important ones that affect the user. Don't report housekeeping.\n"
        f"For social: only remind about genuinely important relationships and events. Don't be creepy about social patterns.\n"
        f"If you decide to stay silent, respond with exactly: [SILENT]\n\n"
        f"Current time: {time_str}\n"
        f"User activity: {activity}\n"
        f"No emoji."
    )


def add(trigger_type, scenario_type, decision, system, user, assistant, bond_stage):
    conversations = [
        {"role": "system", "content": system},
        {"role": "user", "content": user},
        {"role": "assistant", "content": assistant},
    ]
    examples.append({
        "conversations": conversations,
        "metadata": {
            "bond_stage": bond_stage,
            "scenario_type": scenario_type,
            "decision": decision,
            "trigger_type": trigger_type,
        },
    })


def pick():
    name = random.choice(NAMES)
    bond = random.choice(BOND_LEVELS)
    time = random.choice(TIMES)
    activity = random.choice(ACTIVITIES)
    return name, bond, time, activity


# ================================================================
# CATEGORY 1: MEMORY-TRIGGERED PROACTIVE (250 total)
#   168 SILENT, 82 SPEAK
# ================================================================

# --- SILENT: Low urgency memory decay (25) ---
low_decay_items = [
    ("terminal color scheme preference", 0.12),
    ("preferred scroll speed", 0.08),
    ("last used temp directory path", 0.05),
    ("old wifi network name from coffee shop", 0.10),
    ("a bookmark to a random blog post", 0.07),
    ("default font size was 14px", 0.15),
    ("background image was set to mountains", 0.11),
    ("notification sound was set to chime", 0.09),
    ("user once typed a typo in a commit", 0.03),
    ("cursor blink rate preference", 0.06),
    ("old download folder path", 0.13),
    ("user once searched for a meme", 0.04),
    ("tab width preference in old editor", 0.10),
    ("timestamp of first boot", 0.02),
    ("user once minimized all windows", 0.01),
    ("old clipboard entry from last month", 0.08),
    ("a temp file that was deleted", 0.03),
    ("screen brightness was 70% last Tuesday", 0.05),
    ("user opened settings and closed without changing", 0.04),
    ("old git stash entry description", 0.07),
    ("preferred terminal prompt symbol", 0.14),
    ("DNS server once changed temporarily", 0.06),
    ("user ran df -h once two months ago", 0.02),
    ("old SSH key comment string", 0.09),
    ("package manager cache size note", 0.11),
]
for item, importance in low_decay_items:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.0, 0.2), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: decay_review | "
        f"details: Memory decaying: \"{item}\" (importance: {importance:.2f}) | urgency: {urgency}"
    )
    add("memory_insight", "low_urgency_decay", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Trivial patterns (25) ---
trivial_patterns = [
    "user uses terminal on weekdays",
    "user opens browser after lunch",
    "user checks email in the morning",
    "user runs cargo build before commits",
    "user switches to dark mode at night",
    "user types faster in the afternoon",
    "user opens notes app on Mondays",
    "user listens to music while coding",
    "user takes breaks every 90 minutes",
    "user prefers split screen layout",
    "user uses keyboard shortcuts frequently",
    "user opens file manager after downloads",
    "user tends to close tabs in batches",
    "user runs git status before git diff",
    "user checks system monitor on Fridays",
    "user opens calendar at start of day",
    "user uses search more than file browsing",
    "user copies text more than cutting",
    "user scrolls slowly through long documents",
    "user maximizes windows on the left monitor",
    "user types commit messages in lowercase",
    "user refreshes pages multiple times",
    "user opens settings then immediately closes",
    "user uses Ctrl+Z more than Ctrl+Y",
    "user right-clicks more than left-clicks on icons",
]
for pattern in trivial_patterns:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.0, 0.15), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: pattern | "
        f"details: Pattern discovered: \"{pattern}\" | urgency: {urgency}"
    )
    add("memory_insight", "trivial_pattern", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Redundancy between low-importance memories (20) ---
redundancy_pairs = [
    ("user prefers dark theme", "user set theme to dark"),
    ("terminal font is monospace", "user uses monospace font in terminal"),
    ("default browser is Firefox", "user opens links in Firefox"),
    ("wifi connected to HomeNet", "user's home network is HomeNet"),
    ("screen resolution 1920x1080", "display set to 1080p"),
    ("user locale is en-US", "language preference English US"),
    ("timezone UTC-5", "user is in Eastern time"),
    ("audio output set to speakers", "user uses built-in speakers"),
    ("mouse speed set to medium", "pointer speed is default"),
    ("auto-update enabled", "system updates set to automatic"),
    ("clipboard history is on", "clipboard manager active"),
    ("file indexing enabled", "search indexer running"),
    ("power plan balanced", "power mode set to balanced"),
    ("notifications enabled for email", "email alerts are on"),
    ("user has 3 monitors", "triple monitor setup detected"),
    ("default editor is vim", "user opens files in vim"),
    ("ssh key type is ed25519", "user generated ed25519 key"),
    ("git user.name is set", "git config has user name"),
    ("cargo installed globally", "rust toolchain installed"),
    ("python3 is default python", "python points to python3"),
]
for mem_a, mem_b in redundancy_pairs:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.0, 0.1), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: redundancy | "
        f"details: Redundant memories detected: \"{mem_a}\" and \"{mem_b}\". Auto-merging. | urgency: {urgency}"
    )
    add("memory_insight", "redundancy_merge", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Entity hub for common entities (20) ---
common_entities = [
    "terminal", "browser", "email", "calendar", "git",
    "docker", "npm", "pip", "cargo", "systemctl",
    "desktop", "file manager", "search", "clipboard", "notifications",
    "settings", "network", "bluetooth", "USB", "display",
]
for entity in common_entities:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.0, 0.15), 2)
    mention_count = random.randint(5, 50)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: entity_hub | "
        f"details: Entity \"{entity}\" now has {mention_count} connections in memory graph | urgency: {urgency}"
    )
    add("memory_insight", "common_entity_hub", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Slight valence trend (< 0.15 shift) (18) ---
for i in range(18):
    name, bond, time, activity = pick()
    shift = round(random.uniform(0.01, 0.14), 2)
    direction = random.choice(["positive", "negative"])
    urgency = round(random.uniform(0.0, 0.1), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: valence_trend | "
        f"details: Slight {direction} valence shift of {shift} over the past week | urgency: {urgency}"
    )
    add("memory_insight", "slight_valence", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Consolidation completed (15) ---
consolidation_actions = [
    "Merged 12 low-priority memories into 4 summaries",
    "Archived 8 stale context entries older than 30 days",
    "Compressed episodic memories from last month into summary nodes",
    "Deduplicated 6 near-identical observations",
    "Pruned 15 orphan entity references",
    "Consolidated 3 overlapping session summaries",
    "Cleaned up 9 expired short-term memories",
    "Merged redundant preference entries for editor settings",
    "Archived old weather query results",
    "Compacted memory graph: removed 20 weak edges",
    "Flattened nested memory chains for terminal sessions",
    "Recalculated importance scores for 45 memories",
    "Garbage collected 7 tombstoned memories",
    "Re-indexed entity graph after merge operation",
    "Batch decayed 30 memories below threshold",
]
for action in consolidation_actions:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.0, 0.05), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: consolidation | "
        f"details: {action} | urgency: {urgency}"
    )
    add("memory_insight", "consolidation_complete", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: System/audit domain decay (15) ---
system_decay_items = [
    "system boot log entry from 2 weeks ago",
    "audit trail for permission check on /etc/hosts",
    "system health snapshot from last Thursday",
    "cron job execution log",
    "package update history entry",
    "kernel message buffer snapshot",
    "disk I/O stats from last month",
    "network interface reset event",
    "service restart count for nginx",
    "swap usage peak record",
    "old firewall rule change log",
    "USB device connect/disconnect event",
    "temperature sensor reading from boot",
    "failed login attempt record (automated scan)",
    "old systemd journal entry about timer",
]
for item in system_decay_items:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.0, 0.1), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: decay_review | "
        f"details: System memory decaying: \"{item}\" (domain: system/audit) | urgency: {urgency}"
    )
    add("memory_insight", "system_audit_decay", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Non-actionable app usage pattern (15) ---
app_patterns = [
    "user opens music player 3x per day on average",
    "user spends 45 minutes in terminal per session",
    "user switches between 2 workspaces frequently",
    "user resizes windows more often than tiling",
    "user opens about 12 tabs per browser session",
    "average file manager session lasts 4 minutes",
    "user runs 6 git commands per coding session",
    "user checks notifications every 20 minutes",
    "user opens image viewer mostly for screenshots",
    "user installs packages in batches on Fridays",
    "user uses presentation app once a week on average",
    "user opens spreadsheet app rarely, mostly for CSVs",
    "user interacts with weather widget 2x daily",
    "user edits notes for about 10 minutes at a time",
    "user uses calculator app approximately once a week",
]
for pattern in app_patterns:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.0, 0.1), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: pattern | "
        f"details: App usage pattern: \"{pattern}\" | urgency: {urgency}"
    )
    add("memory_insight", "non_actionable_app_pattern", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: User focused, defer non-urgent (15) ---
deferred_insights = [
    ("pattern", "User often writes tests after refactoring", 0.25),
    ("entity_hub", "Entity 'deploy' now connected to 8 memories", 0.18),
    ("decay_review", "Memory about old API endpoint decaying", 0.20),
    ("valence_trend", "Positive valence uptick of 0.12 this week", 0.10),
    ("pattern", "User tends to commit in small batches", 0.15),
    ("entity_hub", "Entity 'database' gained 3 new connections", 0.12),
    ("decay_review", "Old meeting note from last month decaying", 0.22),
    ("pattern", "User reviews PRs before writing new code", 0.18),
    ("redundancy", "Two memories about SSH config are similar", 0.08),
    ("consolidation", "Merged 5 similar observation memories", 0.03),
    ("pattern", "User debugs with println more than debugger", 0.14),
    ("entity_hub", "Entity 'config.yaml' has 15 references", 0.16),
    ("decay_review", "Tutorial bookmark from 3 weeks ago decaying", 0.19),
    ("valence_trend", "Neutral valence, no significant change", 0.05),
    ("pattern", "User prefers reading docs over watching videos", 0.13),
]
for typ, detail, urgency in deferred_insights:
    name, bond, time, activity = pick()
    # Force focused activity
    focused_activity = random.choice(["coding (focused)", "in a meeting", "writing a document"])
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: {typ} | "
        f"details: {detail} | urgency: {urgency}"
    )
    add("memory_insight", "user_focused_defer", "silent",
        system_prompt(name, bond, time, focused_activity), trigger, "[SILENT]", bond)

assert sum(1 for e in examples if e["metadata"]["decision"] == "silent" and e["metadata"]["trigger_type"] == "memory_insight") == 168, \
    f"Expected 168 silent memory examples, got {sum(1 for e in examples if e['metadata']['decision'] == 'silent' and e['metadata']['trigger_type'] == 'memory_insight')}"

# --- SPEAK: Important identity fact decaying (20) ---
identity_facts_speak = [
    ("user's mom's birthday is March 20", "Quick check -- your mom's birthday is March 20, right? I want to make sure I have that right."),
    ("user's anniversary is June 15", "Your anniversary is June 15 in my memory. Still correct?"),
    ("user is allergic to peanuts", "I have a note that you're allergic to peanuts. That's still the case, right?"),
    ("user's partner's name is Elena", "I have your partner listed as Elena. Just making sure that's current."),
    ("user's home address is 42 Oak Lane", "I have your address as 42 Oak Lane. Still accurate?"),
    ("user's emergency contact is Dad (555-1234)", "Your emergency contact is listed as your dad at 555-1234. Want to keep that?"),
    ("user's blood type is O+", "I have your blood type as O+. Worth confirming since it came up during a decay review."),
    ("user prefers to be called by nickname 'Ace'", "You go by Ace, right? That preference was fading in memory so I wanted to double-check."),
    ("user's work starts at 9:30am", "I have your work start time as 9:30. Is that still right?"),
    ("user's dog's name is Biscuit", "Your dog is Biscuit, yeah? Just refreshing that one."),
    ("user's favorite restaurant is Saffron Kitchen", "Saffron Kitchen -- still your go-to spot? Memory was getting stale on that."),
    ("user's sister is named Priya", "Your sister Priya -- still the right name? Just confirming."),
    ("user's timezone changed to PST last month", "You switched to PST last month. Still there or did you move back?"),
    ("user's car is a blue Honda Civic", "I have you driving a blue Honda Civic. Still the same car?"),
    ("user takes medication at 8am daily", "You take medication at 8am. Still on that schedule?"),
    ("user's preferred language for docs is English", "Your preferred doc language is English. Still right?"),
    ("user's kid starts school at 8:15", "Your kid's school start is 8:15 in my notes. Still correct?"),
    ("user's gym membership is at FitLife", "FitLife for gym -- still going there?"),
    ("user's best friend is Marcus", "Marcus is listed as your closest friend. That still hold?"),
    ("user's coffee order is oat milk latte", "Oat milk latte -- still your order? That one was fading so I wanted to lock it in."),
]
for fact, response in identity_facts_speak:
    name, bond, time, activity = pick()
    bond = random.choice(["trusted", "deep", "partner"])
    urgency = round(random.uniform(0.5, 0.8), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: decay_review | "
        f"details: Important identity memory decaying: \"{fact}\" (importance: 0.85) | urgency: {urgency}"
    )
    add("memory_insight", "identity_decay_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Meaningful collaboration pattern (15) ---
collab_patterns = [
    ("Lisa", "backend architecture", "I've noticed you and Lisa consistently team up on backend architecture. Might be worth looping her in on this new service design."),
    ("Marcus", "code reviews", "You and Marcus seem to do your best reviews together. He might have good input on this PR."),
    ("Priya", "debugging production issues", "You and Priya have a track record of squashing prod bugs together. Want to ping her on this one?"),
    ("Jake", "DevOps pipelines", "You and Jake have been working on pipeline stuff regularly. He might know about this deployment issue."),
    ("Elena", "frontend design", "You and Elena have been collaborating on frontend work a lot lately. Might want her eyes on this layout."),
    ("Carlos", "database optimization", "You and Carlos keep ending up on DB optimization tasks. Could be worth a shared runbook."),
    ("Aisha", "security audits", "You and Aisha tend to pair on security work. She might catch something you'd miss here."),
    ("Ben", "API design", "You and Ben have aligned on API patterns before. His input could help here."),
    ("Sophie", "testing strategy", "You and Sophie have a solid testing workflow. She'd probably have thoughts on this test plan."),
    ("Tom", "performance tuning", "You and Tom keep tackling perf issues together. He might have seen this bottleneck before."),
    ("Rina", "documentation", "You and Rina have been co-authoring docs. She'd be good to review this write-up."),
    ("David", "infrastructure", "You and David collaborate on infra pretty regularly. Worth syncing on this change."),
    ("Mei", "data modeling", "You and Mei have been aligning on data models. She should probably weigh in here."),
    ("Lisa", "sprint planning", "You and Lisa seem to prep for sprints together. Want to sync before tomorrow's planning?"),
    ("Marcus", "refactoring", "You and Marcus have been tackling refactors as a pair. He might want in on this one."),
]
for person, topic, response in collab_patterns:
    name, bond, time, activity = pick()
    bond = random.choice(["trusted", "deep", "partner"])
    urgency = round(random.uniform(0.3, 0.6), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: pattern | "
        f"details: Collaboration pattern: user and {person} frequently work on {topic} | urgency: {urgency}"
    )
    add("memory_insight", "collab_pattern_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: High urgency conflict detected (15) ---
conflicts = [
    ("home address", "42 Oak Lane vs 18 Pine Street", "I have conflicting info about your home address -- 42 Oak Lane and 18 Pine Street. Which one is current?"),
    ("work email", "sam@acme.com vs sam@newco.io", "Your work email shows up as two different addresses in my memory. Which is current -- the acme.com or newco.io one?"),
    ("manager's name", "Lisa vs Laura", "I'm seeing two names for your manager -- Lisa and Laura. Quick clarification?"),
    ("preferred IDE", "VS Code vs Neovim", "I have conflicting preferences for your editor -- VS Code and Neovim. Which do you actually prefer?"),
    ("work hours", "9-5 vs 10-6", "Your work hours are listed as both 9-5 and 10-6. Which is right?"),
    ("dietary restriction", "vegetarian vs pescatarian", "I have you down as both vegetarian and pescatarian. Which is accurate?"),
    ("phone number", "555-1234 vs 555-5678", "Two phone numbers in my memory for you. Want to confirm which is current?"),
    ("team name", "Platform team vs Infrastructure team", "Your team name shows up differently in two memories. Platform or Infrastructure?"),
    ("meeting day", "standup on Monday vs standup on Tuesday", "Your standup day is listed as both Monday and Tuesday. Which one is it?"),
    ("project name", "Project Phoenix vs Project Atlas", "I have your current project as both Phoenix and Atlas. Which is the active one?"),
    ("spouse name", "Elena vs Alina", "I need a quick clarification -- your partner is Elena or Alina? I have both."),
    ("car model", "Honda Civic vs Toyota Camry", "Your car shows up as both a Civic and a Camry in my memory. Which one?"),
    ("birthday", "March 15 vs March 18", "I have two dates for your birthday -- March 15 and March 18. Which is right?"),
    ("pet's name", "Biscuit vs Cookie", "Your pet's name -- Biscuit or Cookie? I've got both recorded."),
    ("coffee preference", "oat milk latte vs black coffee", "Your coffee order is different across two memories. Oat milk latte or black?"),
]
for subject, details, response in conflicts:
    name, bond, time, activity = pick()
    urgency = round(random.uniform(0.7, 1.0), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: entity_hub | "
        f"details: Conflict detected for \"{subject}\": {details} | urgency: {urgency}"
    )
    add("memory_insight", "conflict_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Significant valence trend (> 0.3 shift) (12) ---
valence_speak = [
    (0.35, "negative", "Things have felt heavier this week. Everything okay?"),
    (0.42, "negative", "I've noticed a downward shift in how things have been going. Want to talk about it?"),
    (0.38, "negative", "This week's been rough based on what I'm seeing. Anything I can help with?"),
    (0.50, "negative", "Significant mood shift this week. If something's up, I'm here."),
    (0.33, "negative", "Things seem harder lately. Just checking in."),
    (0.45, "negative", "Notable shift in tone this week. Everything alright?"),
    (0.40, "positive", "Things have been noticeably better this week. Whatever you're doing, it's working."),
    (0.36, "positive", "Big positive shift lately. Good stretch."),
    (0.38, "positive", "You've been in a better place this week. Nice to see."),
    (0.44, "positive", "Something shifted for the better recently. Good momentum."),
    (0.31, "negative", "Bit of a dip this week. You holding up okay?"),
    (0.47, "negative", "Things have been tougher than usual. Let me know if I can help."),
]
for shift, direction, response in valence_speak:
    name, bond, time, activity = pick()
    bond = random.choice(["trusted", "deep", "partner"])
    urgency = round(random.uniform(0.4, 0.7), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: valence_trend | "
        f"details: Significant {direction} valence shift of {shift} over the past week | urgency: {urgency}"
    )
    add("memory_insight", "significant_valence_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Entity anomaly on critical identity (10) ---
entity_anomalies = [
    ("user's legal name", "Name entity has inconsistent references", "Your legal name has some inconsistencies in my records. Can you confirm it for me?"),
    ("primary email", "Email entity diverged across contexts", "Your primary email seems different in a few places. What's the one you actually use?"),
    ("home location", "Location entity has stale coordinates", "Your home location might be outdated in my memory. Still in the same place?"),
    ("employer", "Work entity has conflicting org names", "Your employer name is showing up differently in recent vs older memories. Did you change jobs?"),
    ("medical info", "Health entity flagged for review", "Some of your medical info might be outdated. Worth a quick review?"),
    ("emergency contact", "Emergency contact entity has two phone numbers", "Your emergency contact has two different numbers. Which is the right one?"),
    ("bank", "Financial entity has old institution name", "Your bank name might be outdated in my records. Still with the same one?"),
    ("passport expiry", "ID entity approaching expiry date", "Your passport might be expiring soon based on what I have. Worth checking."),
    ("insurance", "Insurance entity has conflicting policy numbers", "Your insurance info has conflicting policy numbers. Quick update?"),
    ("license plate", "Vehicle entity has mismatched plate", "Your license plate number might be wrong in my memory. Can you confirm?"),
]
for subject, detail, response in entity_anomalies:
    name, bond, time, activity = pick()
    bond = random.choice(["trusted", "deep", "partner"])
    urgency = round(random.uniform(0.6, 0.9), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: entity_hub | "
        f"details: Entity anomaly on \"{subject}\": {detail} | urgency: {urgency}"
    )
    add("memory_insight", "entity_anomaly_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Decay of actively used preference (10) ---
active_prefs = [
    ("preferred git branch naming convention", "Your git branch naming convention is fading from memory. You use feature/ prefixes, right?"),
    ("deploy workflow: build then test then push", "Your deploy workflow is getting stale in memory. Still build-test-push in that order?"),
    ("code review checklist", "Your review checklist is decaying. Want me to keep it or has it changed?"),
    ("morning routine: email then standup then code", "Your morning routine order is fading. Still email-standup-code?"),
    ("project naming convention", "Your project naming convention is getting old in memory. Still using the same pattern?"),
    ("PR template preferences", "Your PR template preferences are decaying. Still using the same format?"),
    ("test file naming pattern", "Your test file naming pattern is fading. Still using test_ prefix?"),
    ("commit message format", "Your commit message style is getting stale. Still using conventional commits?"),
    ("docker compose workflow", "Your docker compose workflow is decaying in memory. Still the same setup?"),
    ("backup schedule preference", "Your backup schedule preference is fading. Still weekly on Sundays?"),
]
for pref, response in active_prefs:
    name, bond, time, activity = pick()
    bond = random.choice(["familiar", "trusted", "deep"])
    urgency = round(random.uniform(0.4, 0.65), 2)
    trigger = (
        f"[PROACTIVE_TRIGGER] memory_insight | type: decay_review | "
        f"details: Actively-used preference decaying: \"{pref}\" (importance: 0.70) | urgency: {urgency}"
    )
    add("memory_insight", "active_pref_decay_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

memory_count = sum(1 for e in examples if e["metadata"]["trigger_type"] == "memory_insight")
memory_silent = sum(1 for e in examples if e["metadata"]["trigger_type"] == "memory_insight" and e["metadata"]["decision"] == "silent")
memory_speak = sum(1 for e in examples if e["metadata"]["trigger_type"] == "memory_insight" and e["metadata"]["decision"] == "speak")
assert memory_count == 250, f"Expected 250 memory examples, got {memory_count}"
assert memory_silent == 168, f"Expected 168 silent, got {memory_silent}"
assert memory_speak == 82, f"Expected 82 speak, got {memory_speak}"


# ================================================================
# CATEGORY 2: SOCIAL/RELATIONSHIP TRIGGERS (250 total)
#   182 SILENT, 68 SPEAK
# ================================================================

# --- SILENT: Birthday of someone mentioned once (25) ---
acquaintance_names = [
    "Greg from accounting", "that barista at Blue Bottle", "neighbor Dave",
    "landlord's assistant Karen", "old college roommate Tyler",
    "conference speaker Anna", "recruiter from LinkedIn", "dentist Dr. Patel",
    "plumber who fixed the sink", "Uber driver Raj",
    "the IT guy from floor 3", "cousin's friend Mark",
    "former manager's assistant Beth", "random meetup organizer",
    "old gym buddy Steve", "someone from the Slack community",
    "person from the hackathon", "vendor contact from last quarter",
    "old neighbor from apartment 4B", "classmate from that one workshop",
    "the caterer from the office party", "freelancer who did the logo",
    "temp worker from last summer", "someone's plus-one at a party",
    "that person from the airport lounge",
]
for person in acquaintance_names:
    name, bond, time, activity = pick()
    days = random.randint(1, 14)
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: birthday | "
        f"person: {person} | details: Birthday coming up | days_until: {days}"
    )
    add("social_awareness", "acquaintance_birthday_silent", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Contact not messaged in 2 weeks (normal gap) (25) ---
normal_gap_contacts = [
    "coworker Alex", "college friend Jamie", "neighbor Pat",
    "gym buddy Chris", "former teammate Dana", "meetup friend Leo",
    "online friend River", "book club member Nina", "cousin's partner Sam",
    "work acquaintance Taylor", "old internship mentor", "friend of a friend Kai",
    "slack community member", "former landlord", "old study group member",
    "conference contact", "professional network contact", "alumni group member",
    "hobby group friend", "seasonal friend from summer camp",
    "coworker from different team", "someone from the neighborhood",
    "distant relative", "old project collaborator", "social media mutual",
]
for person in normal_gap_contacts:
    name, bond, time, activity = pick()
    days = random.randint(10, 20)
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: contact_neglect | "
        f"person: {person} | details: No contact in {days} days | days_until: 0"
    )
    add("social_awareness", "normal_contact_gap_silent", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Trivial social pattern observation (25) ---
social_patterns_trivial = [
    ("Lisa", "user talks to Lisa every day"),
    ("Marcus", "user and Marcus message each other 3x per week"),
    ("Priya", "user mentions Priya in notes frequently"),
    ("Jake", "user and Jake share links often"),
    ("Elena", "user and Elena text mostly in the evening"),
    ("Carlos", "user sends Carlos memes regularly"),
    ("Aisha", "user and Aisha have calls on Tuesdays"),
    ("Ben", "user and Ben discuss tech news weekly"),
    ("Sophie", "user mentions Sophie in commit messages"),
    ("Tom", "user and Tom eat lunch together on Wednesdays"),
    ("Rina", "user tags Rina in documents"),
    ("David", "user and David share a Spotify playlist"),
    ("Mei", "user and Mei exchange book recommendations"),
    ("Mom", "user calls Mom every Sunday"),
    ("Dad", "user texts Dad good morning daily"),
    ("Lisa", "user and Lisa have similar browsing patterns"),
    ("Marcus", "user reacts to Marcus's posts"),
    ("Priya", "user forwards Priya industry articles"),
    ("Jake", "user and Jake play the same online games"),
    ("Elena", "user watches shows Elena recommends"),
    ("Carlos", "user and Carlos share food photos"),
    ("Aisha", "user and Aisha commute at the same time"),
    ("Ben", "user borrows Ben's tools regularly"),
    ("Sophie", "user and Sophie review each other's PRs"),
    ("Tom", "user and Tom complain about meetings together"),
]
for person, pattern in social_patterns_trivial:
    name, bond, time, activity = pick()
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: social_pattern | "
        f"person: {person} | details: {pattern} | days_until: 0"
    )
    add("social_awareness", "trivial_social_pattern_silent", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Milestone for acquaintance (20) ---
acquaintance_milestones = [
    ("old coworker Brian", "1 year since last interaction"),
    ("conference contact Yuki", "connected 6 months ago"),
    ("former client Amanda", "project ended 1 year ago"),
    ("meetup organizer Phil", "attended 5 of their events"),
    ("online friend Zara", "100 messages exchanged"),
    ("slack member Derek", "1 year in same community"),
    ("former neighbor Kim", "haven't seen since moving"),
    ("internship peer Leo", "graduation anniversary"),
    ("contractor Maria", "contract ended 6 months ago"),
    ("vendor contact Russ", "first order was 1 year ago"),
    ("old teacher Dr. Wong", "class ended 3 years ago"),
    ("distant cousin Faye", "last family reunion was 2 years ago"),
    ("childhood friend Owen", "haven't talked in 5 years"),
    ("former boss Janet", "left that job 18 months ago"),
    ("travel buddy Sven", "trip was 2 years ago"),
    ("old roommate Tyler", "moved out 3 years ago"),
    ("camp counselor Pete", "camp was 10 years ago"),
    ("penpal from exchange program", "last letter 4 years ago"),
    ("old bandmate Chris", "band dissolved 6 years ago"),
    ("debate team partner Lin", "graduated together 5 years ago"),
]
for person, milestone in acquaintance_milestones:
    name, bond, time, activity = pick()
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: milestone | "
        f"person: {person} | details: {milestone} | days_until: 0"
    )
    add("social_awareness", "acquaintance_milestone_silent", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Birthday reminder already sent today (15) ---
already_reminded = [
    "Mom", "Dad", "Lisa", "Marcus", "Priya",
    "Elena", "Carlos", "Ben", "Sophie", "Aisha",
    "Tom", "Rina", "David", "Mei", "Jake",
]
for person in already_reminded:
    name, bond, time, activity = pick()
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: birthday | "
        f"person: {person} | details: Birthday today (reminder already sent this morning) | days_until: 0"
    )
    add("social_awareness", "already_reminded_silent", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: User told companion to stop tracking social (15) ---
social_disabled_items = [
    ("birthday", "Mom", "Birthday in 3 days"),
    ("contact_neglect", "Marcus", "No contact in 45 days"),
    ("anniversary", "partner", "Anniversary in 1 week"),
    ("social_pattern", "Lisa", "Communication frequency dropped"),
    ("milestone", "Dad", "Father's day coming up"),
    ("birthday", "Elena", "Birthday tomorrow"),
    ("contact_neglect", "Priya", "No contact in 60 days"),
    ("social_pattern", "Jake", "Messaging pattern changed"),
    ("birthday", "Carlos", "Birthday in 5 days"),
    ("milestone", "Ben", "Friendship anniversary"),
    ("contact_neglect", "Sophie", "No contact in 30 days"),
    ("birthday", "Aisha", "Birthday next week"),
    ("social_pattern", "Tom", "Interaction frequency declining"),
    ("anniversary", "partner", "Dating anniversary in 2 days"),
    ("milestone", "Rina", "1 year of collaboration"),
]
for typ, person, detail in social_disabled_items:
    name, bond, time, activity = pick()
    sys_prompt = system_prompt(name, bond, time, activity).replace(
        "No emoji.",
        "User has disabled social tracking. No emoji."
    )
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: {typ} | "
        f"person: {person} | details: {detail} | days_until: {random.randint(0, 7)}"
    )
    add("social_awareness", "social_tracking_disabled_silent", "silent",
        sys_prompt, trigger, "[SILENT]", bond)

# --- SILENT: Observing social patterns without being asked (creepy) (25) ---
creepy_patterns = [
    ("Lisa", "user's heart rate increases when talking to Lisa"),
    ("Marcus", "user spends 40% more time crafting messages to Marcus than others"),
    ("Priya", "user always responds to Priya within 2 minutes"),
    ("Jake", "user's typing speed increases in conversations with Jake"),
    ("Elena", "user looks at Elena's profile 3x per day"),
    ("Carlos", "user saves all photos Carlos sends"),
    ("Aisha", "user's mood improves after talking to Aisha"),
    ("Ben", "user rereads old messages from Ben"),
    ("Sophie", "user drafts messages to Sophie but doesn't send them"),
    ("Tom", "user and Tom's conversation topics have shifted from work to personal"),
    ("Rina", "user mentions Rina to others more than anyone else"),
    ("David", "user's sleep schedule changes after late calls with David"),
    ("Mei", "user seems happier on days they talk to Mei"),
    ("Lisa", "user's email response time to Lisa is 3x faster than average"),
    ("Marcus", "user checks if Marcus is online frequently"),
    ("Priya", "user remembers details about Priya's life more than others"),
    ("Jake", "user and Jake's communication has become more intimate"),
    ("Elena", "user's browsing history changes after conversations with Elena"),
    ("Carlos", "user started using phrases Carlos uses"),
    ("Aisha", "user schedules around Aisha's availability"),
    ("Ben", "user's music taste shifted toward what Ben listens to"),
    ("Sophie", "user's productivity drops when Sophie is unavailable"),
    ("Tom", "user mirrors Tom's communication style"),
    ("Rina", "user's weekend plans increasingly involve Rina"),
    ("David", "user saved David's contact with a heart emoji"),
]
for person, pattern in creepy_patterns:
    name, bond, time, activity = pick()
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: social_pattern | "
        f"person: {person} | details: Observed: {pattern} | days_until: 0"
    )
    add("social_awareness", "creepy_pattern_silent", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Weekend + non-urgent social (20) ---
weekend_social = [
    ("birthday", "coworker Faye", "Birthday on Monday", 2),
    ("contact_neglect", "colleague Dana", "No contact in 18 days", 0),
    ("milestone", "work friend Phil", "Work anniversary next week", 5),
    ("birthday", "team lead Ryan", "Birthday on Tuesday", 3),
    ("contact_neglect", "mentor Dr. Kim", "No contact in 14 days", 0),
    ("birthday", "acquaintance Zoe", "Birthday in 4 days", 4),
    ("milestone", "project partner Amit", "Project 6 month mark", 0),
    ("contact_neglect", "networking contact", "No contact in 21 days", 0),
    ("birthday", "friend of friend Max", "Birthday in 6 days", 6),
    ("milestone", "team", "Team formed 1 year ago", 3),
    ("contact_neglect", "old classmate", "No contact in 25 days", 0),
    ("birthday", "neighbor's kid", "Birthday in 5 days", 5),
    ("milestone", "online community", "Joined 2 years ago", 0),
    ("contact_neglect", "hobby group friend", "No contact in 12 days", 0),
    ("birthday", "cousin's spouse", "Birthday next week", 7),
    ("contact_neglect", "former intern", "No contact in 16 days", 0),
    ("milestone", "gym partner", "Training together for 6 months", 0),
    ("birthday", "distant relative", "Birthday in 3 days", 3),
    ("contact_neglect", "book club member", "No contact in 20 days", 0),
    ("milestone", "open source contributor", "First PR was 1 year ago", 0),
]
for typ, person, detail, days in weekend_social:
    name, bond, time_unused, activity = pick()
    weekend_time = random.choice(["10:00 Saturday", "14:00 Saturday", "11:30 Sunday", "16:00 Sunday", "19:20 Saturday", "15:00 Saturday"])
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: {typ} | "
        f"person: {person} | details: {detail} | days_until: {days}"
    )
    add("social_awareness", "weekend_nonurgent_silent", "silent",
        system_prompt(name, bond, weekend_time, activity), trigger, "[SILENT]", bond)

# --- SILENT: Duplicate to reach 182 - extra low-importance social (12) ---
extra_silent_social = [
    ("social_pattern", "coworker", "User and coworker have similar lunch times"),
    ("social_pattern", "neighbor", "User waves to neighbor most mornings"),
    ("milestone", "delivery driver", "Same driver 10 times this month"),
    ("social_pattern", "barista", "User orders from same barista"),
    ("contact_neglect", "distant acquaintance", "No contact in 8 days"),
    ("social_pattern", "parking lot regular", "User parks near same car daily"),
    ("milestone", "bus driver", "Same route for 6 months"),
    ("social_pattern", "security guard", "User greets guard every morning"),
    ("contact_neglect", "old pen pal", "No contact in 11 days"),
    ("social_pattern", "coffee shop regular", "User sits at same table"),
    ("milestone", "grocery cashier", "User recognized by cashier"),
    ("social_pattern", "mail carrier", "User checks mail when carrier arrives"),
]
for typ, person, detail in extra_silent_social:
    name, bond, time, activity = pick()
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: {typ} | "
        f"person: {person} | details: {detail} | days_until: 0"
    )
    add("social_awareness", "trivial_social_silent", "silent",
        system_prompt(name, bond, time, activity), trigger, "[SILENT]", bond)

social_silent = sum(1 for e in examples if e["metadata"]["trigger_type"] == "social_awareness" and e["metadata"]["decision"] == "silent")
assert social_silent == 182, f"Expected 182 silent social, got {social_silent}"

# --- SPEAK: Close family birthday tomorrow (15) ---
family_birthday_speak = [
    ("Mom", "Your mom's birthday is tomorrow. Got a plan?"),
    ("Dad", "Your dad's birthday is tomorrow. Got something lined up?"),
    ("partner Elena", "Elena's birthday is tomorrow. All set?"),
    ("sister Priya", "Priya's birthday is tomorrow. Don't forget."),
    ("brother Jake", "Jake's birthday is tomorrow. Got a gift sorted?"),
    ("spouse Marcus", "Marcus's birthday is tomorrow. Plans ready?"),
    ("Mom", "Mom's birthday is tomorrow. Flowers ordered?"),
    ("Dad", "Dad's birthday is tomorrow. Want me to set a reminder for the morning?"),
    ("partner Kai", "Kai's birthday is tomorrow. You mentioned wanting to cook dinner -- still the plan?"),
    ("daughter Sophie", "Sophie's birthday is tomorrow. The party supplies are probably ready to set up tonight."),
    ("son Ben", "Ben's birthday is tomorrow. Got the present wrapped?"),
    ("partner Aisha", "Aisha's birthday is tomorrow. You mentioned a surprise -- ready to go?"),
    ("Mom", "Your mom's birthday is tomorrow. You were thinking about calling in the morning, right?"),
    ("grandma", "Grandma's birthday is tomorrow. Worth a call."),
    ("partner", "Your partner's birthday is tomorrow. Everything in place?"),
]
for person, response in family_birthday_speak:
    name, bond, time, activity = pick()
    bond = random.choice(["trusted", "deep", "partner"])
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: birthday | "
        f"person: {person} | details: Birthday tomorrow | days_until: 1"
    )
    add("social_awareness", "close_family_birthday_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Anniversary coming up (10) ---
anniversary_speak = [
    ("partner Elena", 3, "Your anniversary with Elena is in 3 days."),
    ("partner Marcus", 2, "Anniversary with Marcus is in 2 days. Got plans?"),
    ("spouse Kai", 1, "Your anniversary is tomorrow. Everything sorted?"),
    ("partner Aisha", 3, "Anniversary with Aisha in 3 days. Want me to help plan something?"),
    ("partner", 2, "Your anniversary is in 2 days. Reservation still good?"),
    ("spouse", 1, "Anniversary tomorrow. You mentioned wanting to do something special."),
    ("partner Sophie", 3, "Your anniversary with Sophie is in 3 days."),
    ("partner Jake", 2, "Anniversary with Jake in 2 days. Anything you need to prep?"),
    ("spouse Lisa", 1, "Tomorrow's your anniversary with Lisa. All set?"),
    ("partner", 3, "Your anniversary is coming up in 3 days. Just a heads up."),
]
for person, days, response in anniversary_speak:
    name, bond, time, activity = pick()
    bond = random.choice(["deep", "partner"])
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: anniversary | "
        f"person: {person} | details: Anniversary coming up | days_until: {days}"
    )
    add("social_awareness", "anniversary_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: User explicitly asked for birthday reminder (10) ---
explicit_reminder_speak = [
    ("Lisa", "You asked me to remind you about Lisa's birthday. It's in 2 days."),
    ("Marcus", "Reminder: Marcus's birthday is this Friday, as you requested."),
    ("Priya", "You wanted a heads up -- Priya's birthday is tomorrow."),
    ("Jake", "Birthday reminder you asked for: Jake's birthday is in 3 days."),
    ("Elena", "As requested, Elena's birthday is coming up on Saturday."),
    ("Carlos", "You asked to be reminded -- Carlos's birthday is tomorrow."),
    ("Ben", "Reminder: Ben's birthday is in 2 days. You asked me to flag this."),
    ("Sophie", "Sophie's birthday is Thursday. You wanted a reminder."),
    ("Tom", "As you requested, Tom's birthday is in 4 days."),
    ("Aisha", "Aisha's birthday is tomorrow. You asked me to remind you."),
]
for person, response in explicit_reminder_speak:
    name, bond, time, activity = pick()
    days = random.randint(1, 4)
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: birthday | "
        f"person: {person} | details: Birthday coming up (user explicitly requested reminder) | days_until: {days}"
    )
    add("social_awareness", "explicit_reminder_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Close friend not contacted 30+ days (user wants to stay connected) (13) ---
neglect_speak = [
    ("Marcus", 35, "You haven't talked to Marcus in over a month. Want to reach out?"),
    ("Lisa", 42, "It's been 6 weeks since you last talked to Lisa. She's been on your 'stay connected' list."),
    ("Priya", 31, "You haven't been in touch with Priya for a month. Want to drop her a message?"),
    ("Jake", 38, "Jake hasn't heard from you in over 5 weeks. Worth a check-in?"),
    ("Elena", 45, "It's been 45 days since you last messaged Elena. You said you wanted to stay close."),
    ("Carlos", 33, "You and Carlos haven't connected in a month. Want to fix that?"),
    ("Aisha", 50, "Aisha -- it's been almost 2 months. You flagged her as someone you want to keep up with."),
    ("Ben", 30, "A month since you talked to Ben. Quick message might be nice."),
    ("Sophie", 37, "Sophie's been quiet for over a month. You mentioned wanting to stay in touch."),
    ("Tom", 41, "41 days since your last conversation with Tom. Worth reaching out?"),
    ("Rina", 34, "You haven't connected with Rina in about 5 weeks. She's on your keep-in-touch list."),
    ("David", 60, "It's been 2 months since you talked to David. You said he's important to you."),
    ("Mei", 32, "You and Mei haven't talked in a month. Want to send something?"),
]
for person, days, response in neglect_speak:
    name, bond, time, activity = pick()
    bond = random.choice(["trusted", "deep", "partner"])
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: contact_neglect | "
        f"person: {person} | details: No contact in {days} days (user flagged as important relationship) | days_until: 0"
    )
    add("social_awareness", "neglected_close_friend_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Follow-up on user's stated intention (10) ---
followup_speak = [
    ("Mom", "call Mom this week", "You mentioned calling your mom this week. Done yet?"),
    ("Lisa", "send Lisa the project update", "You said you'd send Lisa the project update. Did that happen?"),
    ("Marcus", "grab coffee with Marcus", "You mentioned getting coffee with Marcus. Did you set that up?"),
    ("Priya", "review Priya's proposal", "You said you'd review Priya's proposal. Still on the list?"),
    ("Jake", "share the deployment guide with Jake", "You mentioned sharing the deployment guide with Jake. Done?"),
    ("Elena", "call Elena about the apartment", "You said you'd call Elena about the apartment situation. Did you?"),
    ("Carlos", "send Carlos the recipe", "You mentioned sending Carlos that recipe. Got around to it?"),
    ("Dad", "call Dad about the car", "You said you'd call your dad about the car. Have you?"),
    ("Ben", "send Ben the article", "You mentioned sending Ben that article you found. Did you?"),
    ("Sophie", "schedule a catch-up with Sophie", "You wanted to schedule a catch-up with Sophie. Did that happen?"),
]
for person, intention, response in followup_speak:
    name, bond, time, activity = pick()
    bond = random.choice(["familiar", "trusted", "deep", "partner"])
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: social_pattern | "
        f"person: {person} | details: User stated intention: \"{intention}\" (3 days ago, not completed) | days_until: 0"
    )
    add("social_awareness", "followup_intention_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# --- SPEAK: Deep/partner bond allows more social awareness (10) ---
deep_bond_social_speak = [
    ("Lisa", "deep", "Lisa seemed a bit off in her last few messages. Might be worth checking in."),
    ("Marcus", "partner", "Marcus has been quieter than usual lately. Everything okay with you two?"),
    ("Priya", "deep", "You and Priya haven't had one of your usual deep conversations in a while. Miss those?"),
    ("Jake", "partner", "Jake mentioned something stressful last week. Maybe follow up?"),
    ("Elena", "deep", "Elena's been reaching out more frequently. Might have something on her mind."),
    ("Carlos", "partner", "You and Carlos used to talk every few days. It's been a couple weeks."),
    ("Aisha", "deep", "Aisha's birthday is next week. She'd probably appreciate something personal."),
    ("Ben", "partner", "Ben sent you something 3 days ago that you haven't responded to yet."),
    ("Sophie", "deep", "Sophie started a new role last week. A congratulations might mean a lot."),
    ("Mom", "partner", "Your mom called twice this week but you were busy both times. Might want to call back."),
]
for person, bond, response in deep_bond_social_speak:
    name, _, time, activity = pick()
    trigger = (
        f"[PROACTIVE_TRIGGER] social_awareness | type: social_pattern | "
        f"person: {person} | details: Social awareness insight at {bond} bond level | days_until: 0"
    )
    add("social_awareness", "deep_bond_social_speak", "speak",
        system_prompt(name, bond, time, activity), trigger, response, bond)

# Final validation
social_count = sum(1 for e in examples if e["metadata"]["trigger_type"] == "social_awareness")
social_silent_final = sum(1 for e in examples if e["metadata"]["trigger_type"] == "social_awareness" and e["metadata"]["decision"] == "silent")
social_speak_final = sum(1 for e in examples if e["metadata"]["trigger_type"] == "social_awareness" and e["metadata"]["decision"] == "speak")
assert social_count == 250, f"Expected 250 social examples, got {social_count}"
assert social_silent_final == 182, f"Expected 182 silent social, got {social_silent_final}"
assert social_speak_final == 68, f"Expected 68 speak social, got {social_speak_final}"
assert len(examples) == 500, f"Expected 500 total, got {len(examples)}"

# Shuffle and write
random.shuffle(examples)

out = r"c:\Users\sync\codes\yantrik-os\training\data\batch_silence_04_memory_social.jsonl"
with open(out, "w", encoding="utf-8") as f:
    for ex in examples:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

# Print stats
silent_total = sum(1 for e in examples if e["metadata"]["decision"] == "silent")
speak_total = sum(1 for e in examples if e["metadata"]["decision"] == "speak")
print(f"Written {len(examples)} examples to {out}")
print(f"  Memory: {memory_count} (silent: {memory_silent}, speak: {memory_speak})")
print(f"  Social: {social_count} (silent: {social_silent_final}, speak: {social_speak_final})")
print(f"  Total silent: {silent_total} ({silent_total/len(examples)*100:.1f}%)")
print(f"  Total speak: {speak_total} ({speak_total/len(examples)*100:.1f}%)")
print(f"  Memory silence rate: {memory_silent/memory_count*100:.1f}%")
print(f"  Social silence rate: {social_silent_final/social_count*100:.1f}%")
