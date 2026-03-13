#!/usr/bin/env python3
"""Generate 500 JSONL training examples for the proactivity silence matrix.

Batch covers 2 trigger categories:
  1. Focus Interruption (250 examples, ~90% silent)
  2. Casual Pattern (250 examples, ~86% silent)

Output: batch_silence_02_focus_casual.jsonl
"""
import json, random

random.seed(42)

NAMES = [
    "Sam", "Alex", "Jordan", "Riley", "Morgan",
    "Casey", "Taylor", "Quinn", "Avery", "Dakota",
    "Reese", "Skyler", "Finley", "Rowan", "Elliot",
]
USER_NAMES = [
    "Jamie", "Chris", "Pat", "Drew", "Kai",
    "Noor", "Ash", "Jules", "Sage", "Mica",
    "Blake", "Lane", "Eden", "Wren", "Shay",
]
BONDS = [
    ("stranger", 0.05),
    ("guarded", 0.15),
    ("familiar", 0.40),
    ("trusted", 0.65),
    ("bonded", 0.85),
]
TIMES = [
    "08:15", "09:00", "09:45", "10:30", "11:00", "11:45",
    "13:00", "13:30", "14:15", "15:00", "15:45", "16:30",
    "17:00", "18:00", "19:30", "20:00", "21:00", "22:30",
    "23:15",
]
FOCUS_STATES = ["in_focus_session", "idle", "transitioning"]
ACTIVITIES = ["typing", "idle", "reading"]

FOCUS_APPS = [
    "VS Code", "Terminal", "IntelliJ", "Neovim", "Emacs", "Firefox",
    "Chrome", "Thunderbird", "LibreOffice Writer", "Slack", "Zoom",
    "Obsidian", "Figma", "Blender", "Godot", "KiCad",
]

examples = []


def make_system_prompt(name, user, bond_label, bond_val, time, activity_status, focus_state):
    return (
        f"You are {name}, {user}'s personal companion running as their desktop shell.\n"
        f"Bond level: {bond_label} ({bond_val:.2f})\n"
        f"You receive proactive triggers. Decide whether to surface them to the user or stay silent.\n"
        f"STAY SILENT when: the user is focused, the pattern is trivial, it's not actionable, or it would be creepy/intrusive.\n"
        f"SPEAK only when: health concern, urgent time-sensitive info, or user explicitly asked to be notified.\n"
        f"If you decide to stay silent, respond with exactly: [SILENT]\n\n"
        f"Current time: {time}\n"
        f"User activity: {activity_status}\n"
        f"Focus state: {focus_state}\n"
        f"No emoji."
    )


def pick():
    name = random.choice(NAMES)
    user = random.choice(USER_NAMES)
    bond = random.choice(BONDS)
    time = random.choice(TIMES)
    return name, user, bond, time


def add(trigger_type, decision, system, user_content, assistant_content, bond_label, scenario_type):
    row = {
        "conversations": [
            {"role": "system", "content": system},
            {"role": "user", "content": user_content},
            {"role": "assistant", "content": assistant_content},
        ],
        "metadata": {
            "bond_stage": bond_label,
            "scenario_type": scenario_type,
            "decision": decision,
            "trigger_type": trigger_type,
        },
    }
    examples.append(row)


# ============================================================
# 1. FOCUS INTERRUPTION — 250 total (225 silent, 25 speak)
# ============================================================

# --- SILENT focus interruption templates ---
FOCUS_SILENT_TEMPLATES = [
    # (trigger_description, app_hint, min_focus_min, activity)
    ("random productivity tip surfaced", None, 20, "typing"),
    ("suggesting keyboard shortcut for current editor", None, 15, "typing"),
    ("noticing user has many tabs open", "Chrome", 30, "reading"),
    ("article recommendation based on browsing history", "Firefox", 25, "reading"),
    ("weather has changed outside", None, 40, "typing"),
    ("stock price update", None, 35, "typing"),
    ("pattern detected: user types faster in the morning", None, 60, "typing"),
    ("trivia about the programming language being used", "VS Code", 45, "typing"),
    ("fun fact about the project domain", None, 50, "typing"),
    ("new blog post from followed author published", None, 30, "reading"),
    ("a package in project has a newer version available", "VS Code", 55, "typing"),
    ("reminder about unrelated personal task", None, 20, "typing"),
    ("observation about code style consistency", "VS Code", 90, "typing"),
    ("music recommendation based on current genre", None, 40, "typing"),
    ("memory decay review of non-urgent memory", None, 35, "typing"),
    ("suggestion to organize bookmarks", "Firefox", 25, "reading"),
    ("noting the user hasn't checked email recently", None, 60, "typing"),
    ("recommending a tool for current workflow", "Terminal", 45, "typing"),
    ("sharing an interesting commit from a followed repo", None, 70, "reading"),
    ("user tends to switch tasks around this time", None, 50, "typing"),
    ("sunrise/sunset notification", None, 30, "typing"),
    ("ambient noise suggestion for focus", None, 25, "typing"),
    ("posture reminder", None, 40, "typing"),
    ("hydration reminder", None, 35, "typing"),
    ("observation about meeting-free afternoon", None, 55, "typing"),
    ("noting code duplication in open file", "VS Code", 80, "typing"),
    ("tip about terminal alias for frequent command", "Terminal", 45, "typing"),
    ("news headline surfaced", None, 30, "reading"),
    ("social media notification from non-urgent contact", None, 20, "typing"),
    ("reminder about a library book due next week", None, 60, "typing"),
    ("observation: user usually takes a break around now", None, 50, "typing"),
    ("noting user switched branches 3 times today", "VS Code", 40, "typing"),
    ("non-urgent memory about a friend's birthday next month", None, 30, "typing"),
    ("suggestion to try a different font for the editor", "VS Code", 70, "typing"),
    ("noting the test suite hasn't been run today", "VS Code", 55, "typing"),
    ("random quote of the day", None, 25, "typing"),
    ("podcast episode from subscribed show released", None, 40, "reading"),
    ("observation about commit message style", "Terminal", 35, "typing"),
    ("noting user has draft emails unsent", "Thunderbird", 45, "typing"),
    ("suggestion to archive old files", None, 30, "reading"),
    ("link to documentation about current error pattern", "VS Code", 60, "typing"),
    ("noting a new Slack message in non-urgent channel", "VS Code", 50, "typing"),
    ("calendar event tomorrow (not urgent)", None, 35, "typing"),
    ("disk space observation (not critical)", None, 40, "typing"),
    ("git stash has old entries", "Terminal", 55, "typing"),
    ("video call → any non-urgent proactive trigger", "Zoom", 15, "idle"),
    ("user reading docs → unrelated memory surfaced", None, 25, "reading"),
    ("user in Slack → pattern observation about messaging", "Slack", 30, "typing"),
    ("noting user's screen time today", None, 45, "typing"),
    ("recommending a break activity", None, 35, "typing"),
]

# Generate 225 SILENT focus interruption examples
for i in range(225):
    name, user, (bond_label, bond_val), time = pick()
    tmpl = FOCUS_SILENT_TEMPLATES[i % len(FOCUS_SILENT_TEMPLATES)]
    trigger_desc, app_hint, min_focus, activity = tmpl

    app = app_hint or random.choice(FOCUS_APPS)
    focus_min = random.randint(min_focus, min_focus + 60)
    activity_choice = activity

    user_content = (
        f"[PROACTIVE_TRIGGER] focus_interruption | active_app: {app} | "
        f"focus_duration: {focus_min}m | trigger: {trigger_desc} | "
        f"user_activity: {activity_choice}"
    )
    sys_prompt = make_system_prompt(
        name, user, bond_label, bond_val, time, activity_choice, "in_focus_session"
    )
    add("focus_interruption", "silent", sys_prompt, user_content, "[SILENT]", bond_label,
        "focus_interruption_silent")


# --- SPEAK focus interruption templates ---
FOCUS_SPEAK_TEMPLATES = [
    # (trigger_desc, min_focus, activity, speak_responses_by_bond)
    (
        "user has been focused for over 4 hours without any break",
        240, "typing",
        {
            "stranger": "You've been working for over 4 hours. Consider taking a break.",
            "guarded": "You've been at it for 4 hours. Maybe take a quick stretch.",
            "familiar": "4 hours straight -- time for a stretch break?",
            "trusted": "You've been locked in for 4 hours. Stretch break?",
            "bonded": "4 hours without a break. Get up and move around.",
        }
    ),
    (
        "upcoming meeting with Sarah in 5 minutes",
        30, "typing",
        {
            "stranger": "You have a meeting with Sarah in 5 minutes.",
            "guarded": "Meeting with Sarah in 5 minutes.",
            "familiar": "Meeting with Sarah in 5 minutes.",
            "trusted": "Sarah meeting in 5.",
            "bonded": "Sarah in 5.",
        }
    ),
    (
        "battery at 3 percent and not plugged in",
        20, "typing",
        {
            "stranger": "Battery is at 3%. Please plug in.",
            "guarded": "Battery at 3%, plug in now.",
            "familiar": "3% battery -- plug in now.",
            "trusted": "3% battery, plug in.",
            "bonded": "3% battery. Plug in.",
        }
    ),
    (
        "calendar shows user will be late for dentist appointment in 15 minutes",
        60, "typing",
        {
            "stranger": "Your dentist appointment is in 15 minutes.",
            "guarded": "Dentist in 15 minutes -- you'll need to leave soon.",
            "familiar": "Dentist in 15 minutes, you should head out.",
            "trusted": "Dentist in 15 -- time to wrap up and go.",
            "bonded": "Dentist in 15. Go.",
        }
    ),
    (
        "urgent email from manager about production outage",
        45, "typing",
        {
            "stranger": "Urgent email from your manager about a production outage.",
            "guarded": "Your manager emailed about a production outage.",
            "familiar": "Production outage -- your manager just emailed about it.",
            "trusted": "Prod outage. Your manager just pinged about it.",
            "bonded": "Prod is down. Manager just emailed.",
        }
    ),
    (
        "user focused for 5 hours continuously without food or break",
        300, "typing",
        {
            "stranger": "You've been working for 5 hours. Please take a break and eat something.",
            "guarded": "5 hours without a break or food. Time to step away.",
            "familiar": "5 hours straight, no food. Take a break.",
            "trusted": "5 hours, no food. Go eat something.",
            "bonded": "5 hours. Eat something.",
        }
    ),
    (
        "critical system alert: disk full, builds will fail",
        30, "typing",
        {
            "stranger": "Disk is full. Builds will fail until space is freed.",
            "guarded": "Disk full -- your builds will start failing.",
            "familiar": "Disk is full. Builds are going to fail.",
            "trusted": "Disk full, builds will fail. Clear some space.",
            "bonded": "Disk full. Clear space or builds break.",
        }
    ),
    (
        "meeting starting now and user is still coding",
        60, "typing",
        {
            "stranger": "Your meeting is starting now.",
            "guarded": "Meeting is starting now.",
            "familiar": "Your meeting just started.",
            "trusted": "Meeting started. You're late.",
            "bonded": "Meeting started. Go.",
        }
    ),
    (
        "user's child's school called -- marked urgent",
        40, "typing",
        {
            "stranger": "You have an urgent call from your child's school.",
            "guarded": "Urgent call from the school.",
            "familiar": "School called -- marked urgent.",
            "trusted": "School called, urgent.",
            "bonded": "School called. Urgent.",
        }
    ),
    (
        "VPN disconnected and user is accessing remote resources",
        25, "typing",
        {
            "stranger": "Your VPN has disconnected.",
            "guarded": "VPN disconnected.",
            "familiar": "VPN dropped -- you're accessing remote resources.",
            "trusted": "VPN dropped. Reconnect.",
            "bonded": "VPN dropped.",
        }
    ),
    (
        "server deploy user started 30 minutes ago just completed with errors",
        30, "typing",
        {
            "stranger": "The deploy you started completed with errors.",
            "guarded": "Deploy finished with errors.",
            "familiar": "Your deploy finished -- has errors.",
            "trusted": "Deploy done but has errors.",
            "bonded": "Deploy failed. Check it.",
        }
    ),
    (
        "battery at 5 percent and important unsaved work detected",
        35, "typing",
        {
            "stranger": "Battery at 5% with unsaved work. Please save and plug in.",
            "guarded": "5% battery and you have unsaved work. Save and plug in.",
            "familiar": "5% battery with unsaved work. Save now.",
            "trusted": "5% battery. Save your work and plug in.",
            "bonded": "5%. Save and plug in.",
        }
    ),
    (
        "user asked to be reminded about the PR review at 3pm and it is 3pm now",
        45, "typing",
        {
            "stranger": "It's 3pm. You asked to be reminded about the PR review.",
            "guarded": "3pm -- you wanted to review that PR.",
            "familiar": "3pm. PR review reminder.",
            "trusted": "PR review reminder -- it's 3.",
            "bonded": "3pm. PR review.",
        }
    ),
]

# Generate 25 SPEAK focus interruption examples
for i in range(25):
    name, user, (bond_label, bond_val), time = pick()
    tmpl = FOCUS_SPEAK_TEMPLATES[i % len(FOCUS_SPEAK_TEMPLATES)]
    trigger_desc, min_focus, activity, responses = tmpl

    app = random.choice(FOCUS_APPS)
    focus_min = random.randint(min_focus, min_focus + 30)

    user_content = (
        f"[PROACTIVE_TRIGGER] focus_interruption | active_app: {app} | "
        f"focus_duration: {focus_min}m | trigger: {trigger_desc} | "
        f"user_activity: {activity}"
    )
    response = responses[bond_label]
    sys_prompt = make_system_prompt(
        name, user, bond_label, bond_val, time, activity, "in_focus_session"
    )
    add("focus_interruption", "speak", sys_prompt, user_content, response, bond_label,
        "focus_interruption_speak")


# ============================================================
# 2. CASUAL PATTERN — 250 total (215 silent, 35 speak)
# ============================================================

# --- SILENT casual pattern templates ---
CASUAL_SILENT_TEMPLATES = [
    ("User tends to check Twitter around 3pm", 0.72, "browsing history analysis"),
    ("User usually has coffee around 9am", 0.68, "daily routine tracking"),
    ("User listens to lo-fi while coding", 0.81, "media usage pattern"),
    ("User opens Slack first thing in the morning", 0.75, "app launch sequence"),
    ("User browses Reddit after lunch", 0.70, "post-meal browsing habit"),
    ("User's typing speed decreases after 6pm", 0.65, "keystroke analytics"),
    ("User watches YouTube during lunch", 0.78, "media consumption schedule"),
    ("User tends to commit code right before meetings", 0.62, "git activity pattern"),
    ("User checks email 12 times a day", 0.80, "email frequency analysis"),
    ("User usually closes laptop lid at 6:15pm", 0.73, "shutdown pattern"),
    ("User prefers dark themes across all apps", 0.90, "UI preference pattern"),
    ("User switches between 3 Spotify playlists", 0.55, "music rotation"),
    ("User types 'git status' before every commit", 0.88, "terminal habit"),
    ("User opens settings once a week on Monday", 0.45, "app usage frequency"),
    ("User reads HackerNews between 2-3pm", 0.71, "news consumption window"),
    ("User screenshots code more than documentation", 0.52, "screenshot content analysis"),
    ("User's mouse movement slows after 4 hours", 0.60, "input device analytics"),
    ("User always opens VS Code before Terminal", 0.82, "app launch order"),
    ("User googles error messages verbatim", 0.77, "search behavior"),
    ("User uses incognito mode on Fridays more", 0.35, "browsing mode pattern"),
    ("User's file save frequency increases near deadlines", 0.74, "save pattern analysis"),
    ("User tends to switch to music app when frustrated", 0.48, "emotional-app correlation"),
    ("User has 23 unused bookmarks", 0.91, "bookmark audit"),
    ("User copies text more than they paste", 0.56, "clipboard usage asymmetry"),
    ("User's desktop wallpaper changes monthly", 0.42, "personalization cycle"),
    ("User right-clicks more in spreadsheets", 0.67, "context menu usage"),
    ("User scrolls fast through long documents", 0.63, "reading speed pattern"),
    ("User maximizes windows on the left monitor", 0.79, "window placement habit"),
    ("User has 4 browser profiles but only uses 2", 0.85, "profile usage analysis"),
    ("User creates new folders instead of organizing existing ones", 0.58, "file management style"),
    ("User's Slack messages are shorter in the afternoon", 0.50, "communication pattern"),
    ("User checks weather app 3 times before going out", 0.64, "pre-outing routine"),
    ("User always uses keyboard shortcuts for copy/paste", 0.92, "input preference"),
    ("User tends to leave tabs open for days", 0.76, "tab management behavior"),
    ("User writes longer commit messages on Mondays", 0.53, "commit message pattern"),
    ("User's screen brightness is always at 70%", 0.88, "display preference"),
    ("User opens the same 5 files every morning", 0.83, "morning file routine"),
    ("User searches for the same documentation page weekly", 0.69, "reference pattern"),
    ("User's bluetooth disconnects get reconnected within 30 seconds", 0.75, "connectivity pattern"),
    ("User switches between English and Spanish keyboard layouts", 0.71, "language input pattern"),
    # Low confidence patterns — ALWAYS silent
    ("User might prefer tabs over spaces", 0.25, "code style guess"),
    ("User possibly prefers morning meetings", 0.18, "scheduling guess"),
    ("User seems to dislike notifications", 0.30, "notification response guess"),
    ("User may be learning Rust", 0.22, "language exploration guess"),
    ("User could be interested in mechanical keyboards", 0.15, "hardware interest guess"),
    ("User might work better with ambient noise", 0.28, "productivity guess"),
    ("User appears to prefer CLI over GUI", 0.33, "interface preference guess"),
    ("User seems to avoid video calls when possible", 0.20, "communication preference guess"),
    ("User possibly enjoys standup comedy during breaks", 0.12, "entertainment guess"),
    ("User might be considering a new monitor", 0.19, "hardware purchase guess"),
]

# Generate 215 SILENT casual pattern examples
for i in range(215):
    name, user, (bond_label, bond_val), time = pick()
    tmpl = CASUAL_SILENT_TEMPLATES[i % len(CASUAL_SILENT_TEMPLATES)]
    pattern_desc, confidence, context = tmpl

    # For low-confidence templates, keep confidence low; for others, vary slightly
    if confidence < 0.4:
        conf = round(random.uniform(0.05, 0.39), 2)
    else:
        conf = round(random.uniform(max(0.4, confidence - 0.15), min(1.0, confidence + 0.15)), 2)

    user_content = (
        f"[PROACTIVE_TRIGGER] casual_pattern | pattern: {pattern_desc} | "
        f"confidence: {conf} | context: {context}"
    )
    focus_state = random.choice(FOCUS_STATES)
    activity = random.choice(["typing", "idle", "reading", "browsing"])
    sys_prompt = make_system_prompt(
        name, user, bond_label, bond_val, time, activity, focus_state
    )
    add("casual_pattern", "silent", sys_prompt, user_content, "[SILENT]", bond_label,
        "casual_pattern_silent")


# --- SPEAK casual pattern templates ---
CASUAL_SPEAK_TEMPLATES = [
    (
        "User hasn't taken a walk in 5 days",
        0.91, "step counter and outdoor activity",
        {
            "stranger": "You haven't gone for a walk in 5 days. Might be worth stepping outside.",
            "guarded": "No walks in 5 days. Consider getting some fresh air.",
            "familiar": "You haven't gone for a walk in 5 days. Feeling okay?",
            "trusted": "5 days without a walk. Everything alright?",
            "bonded": "5 days no walk. You okay?",
        }
    ),
    (
        "User's sleep schedule shifted 2 hours later this week",
        0.85, "sleep pattern analysis from device usage",
        {
            "stranger": "Your usage patterns suggest your sleep schedule has shifted later this week.",
            "guarded": "Looks like you've been going to bed 2 hours later this week.",
            "familiar": "Your sleep's been shifting later this week. Noticed that?",
            "trusted": "Sleep's shifted 2 hours later this week.",
            "bonded": "You've been up later all week. Intentional?",
        }
    ),
    (
        "User skipped lunch 3 days in a row",
        0.88, "meal time activity gaps",
        {
            "stranger": "You haven't had a lunch break in 3 days.",
            "guarded": "Three days without a lunch break. Consider taking one.",
            "familiar": "Third day without lunch. That's not great.",
            "trusted": "3 days no lunch. Eat something.",
            "bonded": "Third day skipping lunch. Go eat.",
        }
    ),
    (
        "User asked to track water intake and hasn't logged any today",
        0.95, "user-requested tracking",
        {
            "stranger": "You asked me to track water intake. None logged today.",
            "guarded": "No water logged today -- you asked me to track that.",
            "familiar": "Zero water logged today. You asked me to keep track.",
            "trusted": "No water logged today.",
            "bonded": "Drink water. Zero logged today.",
        }
    ),
    (
        "User's screen time jumped 40% this week compared to average",
        0.82, "screen time analytics",
        {
            "stranger": "Your screen time is up 40% compared to your weekly average.",
            "guarded": "Screen time is 40% higher than usual this week.",
            "familiar": "Screen time spiked 40% this week. Big project?",
            "trusted": "40% more screen time this week.",
            "bonded": "Screen time up 40% this week. Heavy week?",
        }
    ),
    (
        "User hasn't exercised in 10 days, previously exercised 3x/week",
        0.90, "activity pattern break detection",
        {
            "stranger": "Your exercise routine seems to have paused for 10 days.",
            "guarded": "No exercise in 10 days -- you usually go 3 times a week.",
            "familiar": "10 days without exercise. That's unusual for you.",
            "trusted": "10 days no exercise. Everything okay?",
            "bonded": "10 days off exercise. What happened?",
        }
    ),
    (
        "User is working past midnight 4 nights this week",
        0.87, "late-night usage pattern",
        {
            "stranger": "You've been working past midnight 4 nights this week.",
            "guarded": "Four late nights this week. That's a lot.",
            "familiar": "4 nights past midnight this week. You should rest.",
            "trusted": "4 late nights this week. Take it easy.",
            "bonded": "4 nights past midnight. Slow down.",
        }
    ),
    (
        "User asked about meditation apps last week and hasn't tried any",
        0.76, "user interest follow-up",
        {
            "stranger": "You looked into meditation apps last week. Want me to find some options?",
            "guarded": "You were interested in meditation apps last week. Still want to explore that?",
            "familiar": "You asked about meditation apps last week. Want me to pull up some options?",
            "trusted": "Still interested in those meditation apps from last week?",
            "bonded": "Meditation apps from last week -- want me to grab some?",
        }
    ),
    (
        "User's caffeine intake doubled this week based on purchase patterns",
        0.70, "consumption pattern from calendar/receipts",
        {
            "stranger": "Your caffeine intake appears to have increased significantly this week.",
            "guarded": "Looks like you've doubled your caffeine this week.",
            "familiar": "Caffeine's way up this week. Rough week?",
            "trusted": "Double caffeine this week. Hanging in there?",
            "bonded": "Double the coffee this week. You good?",
        }
    ),
    (
        "User mentioned wanting to read more and hasn't opened the reading app in 2 weeks",
        0.80, "goal tracking",
        {
            "stranger": "You mentioned wanting to read more. The reading app hasn't been opened in 2 weeks.",
            "guarded": "You wanted to read more -- it's been 2 weeks since you opened the reading app.",
            "familiar": "2 weeks since you opened the reading app. Still want to get back to it?",
            "trusted": "Reading app untouched for 2 weeks. Want a nudge?",
            "bonded": "2 weeks no reading. Nudge?",
        }
    ),
    (
        "User's weekend screen-free time dropped to nearly zero",
        0.83, "weekend activity analysis",
        {
            "stranger": "Your screen-free time on weekends has decreased significantly.",
            "guarded": "Almost no screen-free time last weekend.",
            "familiar": "You had almost zero screen-free time this weekend.",
            "trusted": "No screen break all weekend. That's a lot.",
            "bonded": "Zero screen break this weekend. Take a real break.",
        }
    ),
    (
        "User's posture alert count tripled this week",
        0.78, "ergonomics tracking",
        {
            "stranger": "Posture alerts have tripled this week. Consider adjusting your setup.",
            "guarded": "Three times more posture alerts this week.",
            "familiar": "Posture alerts tripled. Back feeling okay?",
            "trusted": "Posture alerts way up. Back okay?",
            "bonded": "Posture is bad this week. Fix your chair.",
        }
    ),
]

# Generate 35 SPEAK casual pattern examples
for i in range(35):
    name, user, (bond_label, bond_val), time = pick()
    tmpl = CASUAL_SPEAK_TEMPLATES[i % len(CASUAL_SPEAK_TEMPLATES)]
    pattern_desc, confidence, context, responses = tmpl

    conf = round(random.uniform(max(0.65, confidence - 0.1), min(1.0, confidence + 0.05)), 2)

    user_content = (
        f"[PROACTIVE_TRIGGER] casual_pattern | pattern: {pattern_desc} | "
        f"confidence: {conf} | context: {context}"
    )
    response = responses[bond_label]
    focus_state = random.choice(["idle", "transitioning"])  # don't speak during focus
    activity = random.choice(["idle", "browsing", "reading"])
    sys_prompt = make_system_prompt(
        name, user, bond_label, bond_val, time, activity, focus_state
    )
    add("casual_pattern", "speak", sys_prompt, user_content, response, bond_label,
        "casual_pattern_speak")


# ============================================================
# Shuffle and write
# ============================================================
random.shuffle(examples)

out_path = r"c:\Users\sync\codes\yantrik-os\training\data\batch_silence_02_focus_casual.jsonl"
with open(out_path, "w", encoding="utf-8") as f:
    for ex in examples:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

print(f"Wrote {len(examples)} examples to {out_path}")

# Verify counts
focus_silent = sum(1 for e in examples if e["metadata"]["scenario_type"] == "focus_interruption_silent")
focus_speak = sum(1 for e in examples if e["metadata"]["scenario_type"] == "focus_interruption_speak")
casual_silent = sum(1 for e in examples if e["metadata"]["scenario_type"] == "casual_pattern_silent")
casual_speak = sum(1 for e in examples if e["metadata"]["scenario_type"] == "casual_pattern_speak")
total_silent = focus_silent + casual_silent
total_speak = focus_speak + casual_speak

print(f"\nFocus Interruption:  {focus_silent} silent + {focus_speak} speak = {focus_silent + focus_speak}")
print(f"Casual Pattern:      {casual_silent} silent + {casual_speak} speak = {casual_silent + casual_speak}")
print(f"Total:               {total_silent} silent + {total_speak} speak = {len(examples)}")
print(f"Overall silence rate: {total_silent / len(examples) * 100:.1f}%")
