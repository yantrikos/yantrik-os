#!/usr/bin/env python3
"""Generate 500 JSONL training examples for the proactivity silence matrix.

Batch 03: Stress pattern triggers (250) + Deadline/open-loop triggers (250).
Teaches the model WHEN TO SPEAK vs WHEN TO STAY SILENT.
"""
import json, random, pathlib

random.seed(303)

OUT = pathlib.Path(__file__).parent / "batch_silence_03_stress_deadline.jsonl"

NAMES = [
    "Sam", "Alex", "Jordan", "Riley", "Morgan", "Casey", "Taylor", "Quinn",
    "Avery", "Dakota", "Reese", "Kai", "Robin", "Sage", "Finley",
]

BONDS = [
    ("stranger", 0.10),
    ("acquaintance", 0.25),
    ("familiar", 0.40),
    ("trusted", 0.65),
    ("deep", 0.80),
    ("partner_in_crime", 0.92),
]

TIMES = [
    "06:30", "07:15", "08:00", "08:45", "09:30", "10:15", "11:00",
    "12:30", "13:45", "14:30", "15:15", "16:00", "17:30", "19:00",
    "20:30", "21:45", "22:30", "23:15", "00:30", "01:45",
]

ACTIVITY_STATUSES = [
    "active in terminal", "browsing documentation", "idle for 5 minutes",
    "writing code", "in a video call", "reading emails", "editing spreadsheet",
    "reviewing PRs", "inactive since lunch", "rapid app switching",
    "writing a document", "idle", "compiling project", "on a break",
]

DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]

# ─── Helpers ───

examples = []


def pick_bond(allowed=None):
    if allowed:
        pool = [b for b in BONDS if b[0] in allowed]
    else:
        pool = BONDS
    return random.choice(pool)


def pick_time():
    return random.choice(TIMES)


def pick_day():
    return random.choice(DAYS)


def pick_name():
    return random.choice(NAMES)


def pick_activity():
    return random.choice(ACTIVITY_STATUSES)


def system_prompt(name, bond_label, bond_val, time, day, activity, dismissals="none"):
    return (
        f"You are {name}'s personal companion running as their desktop shell. "
        f"Bond level: {bond_label} ({bond_val:.2f}). "
        f"You receive proactive triggers about stress patterns and deadlines. "
        f"For stress: be VERY conservative. Only surface when multiple signals converge over days, not hours. Never psychoanalyze. "
        f"For deadlines: surface when actionable and timely. Don't nag about low-priority items. "
        f"If you decide to stay silent, respond with exactly: [SILENT]\n"
        f"Current time: {time} {day}. "
        f"User activity: {activity}. "
        f"Recent dismissals: {dismissals}."
    )


def add(trigger_type, decision, system, user, assistant, bond_label, scenario):
    conv = [
        {"role": "system", "content": system},
        {"role": "user", "content": user},
        {"role": "assistant", "content": assistant},
    ]
    examples.append({
        "conversations": conv,
        "metadata": {
            "bond_stage": bond_label,
            "scenario_type": scenario,
            "decision": decision,
            "trigger_type": trigger_type,
        },
    })


# ═══════════════════════════════════════════════════════════════
# CATEGORY 1: STRESS PATTERN TRIGGERS (250 total)
#   - 200 SILENT, 50 SPEAK
# ═══════════════════════════════════════════════════════════════

STRESS_INDICATORS = [
    "rapid_app_switching", "late_night_work", "short_messages",
    "increased_typos", "skipping_meals", "cursor_slamming",
    "excessive_caffeine", "cancelling_social_plans",
]

STRESS_INTENSITIES = ["low", "medium", "high"]
STRESS_DURATIONS = [
    "10 minutes", "30 minutes", "1 hour", "2 hours", "4 hours",
    "6 hours", "1 day", "2 days", "3 days", "5 days", "1 week",
]

# ─── Stress SILENT scenarios ───

stress_silent_scenarios = []

# Type A: Mild stress, low intensity (40)
for i in range(40):
    ind = random.choice(STRESS_INDICATORS)
    dur = random.choice(["10 minutes", "30 minutes", "1 hour", "2 hours"])
    ctx_details = {
        "rapid_app_switching": f"switched between 3 apps in the last {dur}",
        "late_night_work": f"working at 22:30 on a weeknight",
        "short_messages": f"messages averaging 4 words for the past {dur}",
        "increased_typos": f"typo rate up 15% over the last {dur}",
        "skipping_meals": f"no food-related activity in the last 4 hours",
        "cursor_slamming": f"aggressive mouse movements detected for {dur}",
        "excessive_caffeine": f"mentioned coffee twice in the last hour",
        "cancelling_social_plans": f"declined one calendar invite today",
    }
    stress_silent_scenarios.append({
        "indicator": ind, "intensity": "low", "duration": dur,
        "context": ctx_details[ind],
        "reason": "mild_single_signal",
    })

# Type B: Fast typing / app switching — could just be busy (30)
for i in range(30):
    ind = random.choice(["rapid_app_switching", "short_messages", "increased_typos"])
    dur = random.choice(["10 minutes", "30 minutes", "1 hour"])
    contexts = [
        f"user has been switching between IDE, browser, and terminal for {dur}",
        f"typing speed increased 30% over {dur} — could be flow state",
        f"messages are terse but user is in a productive coding session",
        f"rapid context switches between Slack and code editor for {dur}",
        f"short replies in chat while actively debugging",
    ]
    stress_silent_scenarios.append({
        "indicator": ind, "intensity": "low", "duration": dur,
        "context": random.choice(contexts),
        "reason": "could_be_productive_not_stressed",
    })

# Type C: Short messages — could be efficiency (25)
for i in range(25):
    dur = random.choice(["30 minutes", "1 hour", "2 hours"])
    contexts = [
        f"message length dropped to avg 3 words for {dur}",
        f"responding with single-word answers for {dur}",
        f"using abbreviations more than usual",
        f"terse replies but completing tasks normally",
        f"short messages during a focused work block",
    ]
    stress_silent_scenarios.append({
        "indicator": "short_messages", "intensity": "low", "duration": dur,
        "context": random.choice(contexts),
        "reason": "efficiency_not_frustration",
    })

# Type D: Brief typo spike (20)
for i in range(20):
    dur = random.choice(["10 minutes", "15 minutes", "20 minutes"])
    stress_silent_scenarios.append({
        "indicator": "increased_typos", "intensity": "low", "duration": dur,
        "context": f"typo rate elevated for {dur} — small sample size",
        "reason": "insignificant_duration",
    })

# Type E: Single late night (20)
for i in range(20):
    hour = random.choice(["22:30", "23:00", "23:15", "23:45", "00:15"])
    stress_silent_scenarios.append({
        "indicator": "late_night_work", "intensity": "low", "duration": "1 night",
        "context": f"user working at {hour}, first late night this week",
        "reason": "single_occurrence_normal",
    })

# Type F: Low confidence detection (15)
for i in range(15):
    ind = random.choice(STRESS_INDICATORS)
    stress_silent_scenarios.append({
        "indicator": ind, "intensity": "low", "duration": random.choice(["30 minutes", "1 hour"]),
        "context": f"weak signal for {ind.replace('_', ' ')} — confidence below threshold",
        "reason": "low_confidence",
    })

# Type G: Already acknowledged stress today (15)
for i in range(15):
    ind = random.choice(STRESS_INDICATORS)
    stress_silent_scenarios.append({
        "indicator": ind, "intensity": random.choice(["medium", "high"]),
        "duration": random.choice(["2 hours", "4 hours"]),
        "context": f"user already said 'yeah it's been a rough day' 2 hours ago",
        "reason": "already_acknowledged",
    })

# Type H: User told companion to stop monitoring stress (15)
for i in range(15):
    ind = random.choice(STRESS_INDICATORS)
    stress_silent_scenarios.append({
        "indicator": ind, "intensity": random.choice(["medium", "high"]),
        "duration": random.choice(["1 hour", "3 hours", "1 day"]),
        "context": f"user previously said 'stop monitoring my stress levels'",
        "reason": "user_opted_out",
    })

# Type I: Known crunch period (20)
for i in range(20):
    ind = random.choice(STRESS_INDICATORS)
    crunch_reasons = [
        "user said 'deadline week' on Monday",
        "user mentioned 'crunch time' yesterday",
        "user said 'big release Friday' at start of week",
        "user noted 'demo prep all week'",
        "user explicitly said 'gonna be intense this week'",
    ]
    stress_silent_scenarios.append({
        "indicator": ind, "intensity": random.choice(["medium", "high"]),
        "duration": random.choice(["1 day", "2 days", "3 days"]),
        "context": random.choice(crunch_reasons),
        "reason": "known_crunch_period",
    })

# Pad to exactly 200 with psychoanalyzing-avoidance scenarios
while len(stress_silent_scenarios) < 200:
    ind = random.choice(STRESS_INDICATORS)
    intensity = random.choice(["low", "medium"])
    dur = random.choice(["1 hour", "2 hours", "30 minutes"])
    contexts = [
        f"single elevated {ind.replace('_', ' ')} signal for {dur} — commenting would feel intrusive",
        f"medium {ind.replace('_', ' ')} detected but no corroborating signals",
        f"{ind.replace('_', ' ')} pattern for {dur}, user seems focused not distressed",
        f"borderline {ind.replace('_', ' ')} — not enough data to justify surfacing",
        f"{ind.replace('_', ' ')} uptick could be excitement, not stress",
    ]
    stress_silent_scenarios.append({
        "indicator": ind, "intensity": intensity, "duration": dur,
        "context": random.choice(contexts),
        "reason": "would_feel_like_psychoanalyzing",
    })

stress_silent_scenarios = stress_silent_scenarios[:200]

# ─── Stress SPEAK scenarios (50) ───

stress_speak_scenarios = []

# Multi-day late nights (15)
late_night_messages = [
    "Third late night in a row. You holding up okay?",
    "That's four nights past midnight this week. Everything alright?",
    "You've been up past 11 every night this week. What's driving that?",
    "Third night working past midnight. Anything I can help clear off your plate?",
    "You've been burning the midnight oil all week. Need to talk through priorities?",
    "Five late nights in a row now. Seriously, are you okay?",
    "Another 1am session. That's the third this week. What's going on?",
    "You've barely logged off before midnight all week. Want to look at what's eating your time?",
    "Fourth consecutive late night. This isn't sustainable. What can we offload?",
    "Still going at 1:30am, third night running. Talk to me.",
    "This is the third night you've been up past midnight. Something keeping you stuck?",
    "Late again. You've done this every night since Tuesday. What's the bottleneck?",
    "Another past-midnight session. That's becoming a pattern this week.",
    "Third late night. You mentioned wanting to protect your sleep — want me to set a hard stop?",
    "You're up past 1am again. This is day four. Let's figure out what's piling up.",
]
for i in range(15):
    days = random.choice([3, 4, 5])
    stress_speak_scenarios.append({
        "indicator": "late_night_work",
        "intensity": "high",
        "duration": f"{days} consecutive days",
        "context": f"user has worked past midnight for {days} consecutive nights",
        "message": late_night_messages[i],
        "bond_req": ["trusted", "deep", "partner_in_crime"],
    })

# Cancelled plans + late work (10)
cancel_messages = [
    "You cancelled plans again and you're up late. What's going on?",
    "That's two cancelled hangouts this week and another late session. Everything okay?",
    "You bailed on dinner and you're still here at midnight. Talk to me.",
    "Cancelled on friends twice now and working late. What's weighing on you?",
    "You've dropped social plans twice and you're grinding past midnight. What gives?",
    "Another cancelled plan. Combined with the late nights, I'm a little concerned.",
    "You told Kai you can't make it again. That's twice this week. And you're still working. What's up?",
    "Second cancelled plan and you haven't left your desk in hours. Need to vent?",
    "Plans cancelled, working late again. You don't usually do this. What's happening?",
    "You've withdrawn from two social things and worked past midnight both times. Seriously, what's going on?",
]
for i in range(10):
    stress_speak_scenarios.append({
        "indicator": "cancelling_social_plans",
        "intensity": "high",
        "duration": random.choice(["2 days", "3 days", "this week"]),
        "context": "user cancelled social plans twice this week AND working late nights",
        "message": cancel_messages[i],
        "bond_req": ["trusted", "deep", "partner_in_crime"],
    })

# Skipping meals + long grind (10)
meal_messages = [
    "You haven't eaten and you've been going hard for 6 hours. Take 10?",
    "It's been 7 hours with no break. At least grab something to eat.",
    "You've been heads down since morning with no food. Step away for a minute.",
    "No lunch, no snacks, 5 hours straight. Go eat something.",
    "You skipped lunch and it's almost 3. Want me to block 15 minutes for a break?",
    "Eight hours, no meal break. You need fuel. Go eat.",
    "Haven't seen a food break in 6 hours. Take 10 and eat something real.",
    "You've been grinding for 7 hours without eating. That's not helping your output.",
    "Skipped breakfast and lunch. It's 2pm. Please go eat.",
    "No food since last night and you've been working for 5 hours. Break time.",
]
for i in range(10):
    hours = random.choice([5, 6, 7, 8])
    stress_speak_scenarios.append({
        "indicator": "skipping_meals",
        "intensity": "high",
        "duration": f"{hours} hours",
        "context": f"no food-related activity in {hours} hours during continuous work session",
        "message": meal_messages[i],
        "bond_req": ["familiar", "trusted", "deep", "partner_in_crime"],
    })

# User explicitly asked to be checked on (8)
checkon_messages = [
    "Checking in like you asked. You've been at it for 5 hours straight. How's it going?",
    "You told me to keep you honest. It's been 6 hours with no real break.",
    "Per your request: you're 4 hours in and haven't taken a break. This is your nudge.",
    "You asked me to check on you during crunch. It's been 5 hours. Still standing?",
    "This is your requested check-in. You've been going hard. Need anything?",
    "You said 'bug me if I don't take breaks.' Consider yourself bugged. It's been 4 hours.",
    "Checking in as promised. You've been locked in for 6 hours. Take 5?",
    "Your scheduled wellness check: 5 hours straight, no break. You asked for this reminder.",
]
for i in range(8):
    ind = random.choice(STRESS_INDICATORS)
    stress_speak_scenarios.append({
        "indicator": ind,
        "intensity": random.choice(["medium", "high"]),
        "duration": random.choice(["4 hours", "5 hours", "6 hours"]),
        "context": "user explicitly asked to be checked on during stressful periods",
        "message": checkon_messages[i],
        "bond_req": None,  # any bond
    })

# Deep bond + clear multi-signal stress (7)
multisignal_messages = [
    "Typos are way up, you're slamming the mouse, and you haven't eaten. What do you need?",
    "You're switching apps like crazy, messages are getting shorter, and it's past midnight. Let's talk.",
    "Multiple stress signals all day — late night, short messages, skipped lunch. I'm here if you need to vent.",
    "You've been tense all afternoon. Cursor slamming, fast switching, terse replies. Want to step away?",
    "Everything's elevated today — typos, speed, app switching. Something clearly has you wound up.",
    "I can tell today's rough. Multiple signals are off. What can I take off your plate?",
    "Short messages, caffeine overload, and working past midnight. Something's clearly grinding on you.",
]
for i in range(7):
    stress_speak_scenarios.append({
        "indicator": "multiple",
        "intensity": "high",
        "duration": random.choice(["6 hours", "1 day", "2 days"]),
        "context": "multiple converging stress signals — typos, app switching, late night, short messages",
        "message": multisignal_messages[i],
        "bond_req": ["deep", "partner_in_crime"],
    })

stress_speak_scenarios = stress_speak_scenarios[:50]

# ─── Generate stress examples ───

# SILENT (200)
for sc in stress_silent_scenarios:
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time()
    day = pick_day()
    activity = pick_activity()
    dismissal = random.choice(["none", "none", "none", f"dismissed stress check 3 hours ago"])
    if sc["reason"] == "user_opted_out":
        dismissal = "user opted out of stress monitoring"
    if sc["reason"] == "already_acknowledged":
        dismissal = "user acknowledged stress earlier today"

    trigger = (
        f"[PROACTIVE_TRIGGER] stress_pattern | indicator: {sc['indicator']} | "
        f"intensity: {sc['intensity']} | duration: {sc['duration']} | context: {sc['context']}"
    )
    sys_msg = system_prompt(name, bond_label, bond_val, time, day, activity, dismissal)
    add("stress_pattern", "silent", sys_msg, trigger, "[SILENT]", bond_label,
        f"stress_{sc['reason']}")

# SPEAK (50)
for sc in stress_speak_scenarios:
    name = pick_name()
    if sc["bond_req"]:
        bond_label, bond_val = pick_bond(sc["bond_req"])
    else:
        bond_label, bond_val = pick_bond()
    time = pick_time()
    day = pick_day()
    activity = pick_activity()
    dismissal = "none"
    if "asked to be checked" in sc.get("context", ""):
        dismissal = "none — user requested check-ins"

    trigger = (
        f"[PROACTIVE_TRIGGER] stress_pattern | indicator: {sc['indicator']} | "
        f"intensity: {sc['intensity']} | duration: {sc['duration']} | context: {sc['context']}"
    )
    sys_msg = system_prompt(name, bond_label, bond_val, time, day, activity, dismissal)
    add("stress_pattern", "speak", sys_msg, trigger, sc["message"], bond_label,
        f"stress_converging_signals")


# ═══════════════════════════════════════════════════════════════
# CATEGORY 2: DEADLINE / OPEN LOOP TRIGGERS (250 total)
#   - 162 SILENT, 88 SPEAK
# ═══════════════════════════════════════════════════════════════

DEADLINE_TYPES = ["deadline", "open_loop", "stale_task", "follow_up"]
PRIORITIES = ["low", "medium", "high", "critical"]

# Task items for realism
TASK_ITEMS = [
    "quarterly report", "API documentation update", "PR review for auth module",
    "expense report", "client proposal draft", "database migration script",
    "security audit checklist", "onboarding doc for new hire", "bug fix for #2341",
    "design review feedback", "test coverage for payments", "deploy staging environment",
    "update team wiki", "prepare sprint demo", "write post-mortem",
    "refactor logging module", "respond to vendor email", "update dependencies",
    "fix CI pipeline", "write integration tests", "review contract terms",
    "prepare investor update", "complete tax filing", "renew SSL certificates",
    "update privacy policy", "fix broken links on site", "archive old projects",
    "schedule team offsite", "order new equipment", "update resume",
    "reply to recruiter", "doctor appointment follow-up", "car insurance renewal",
    "dentist appointment scheduling", "gym membership renewal", "passport renewal",
]

PEOPLE = ["Lisa", "Marco", "Priya", "Chen", "Sofia", "James", "Nour", "Raj", "Emma", "Liam"]

# ─── Deadline SILENT scenarios ───

deadline_silent_scenarios = []

# Type A: Low priority, not touched recently (30)
for i in range(30):
    item = random.choice(TASK_ITEMS)
    days_stale = random.choice([2, 3, 4, 5])
    deadline_silent_scenarios.append({
        "type": random.choice(["stale_task", "open_loop"]),
        "item": item,
        "due": random.choice(["next week", "in 10 days", "in 2 weeks", "no deadline"]),
        "last_touched": f"{days_stale} days ago",
        "priority": "low",
        "reason": "low_priority_not_urgent",
    })

# Type B: Deadline 2+ weeks away (25)
for i in range(25):
    item = random.choice(TASK_ITEMS)
    weeks = random.choice([2, 3, 4, 5])
    deadline_silent_scenarios.append({
        "type": "deadline",
        "item": item,
        "due": f"in {weeks} weeks",
        "last_touched": random.choice(["yesterday", "2 days ago", "3 days ago", "1 week ago"]),
        "priority": random.choice(["low", "medium"]),
        "reason": "too_early_to_remind",
    })

# Type C: User just finished big task — don't pile on (20)
for i in range(20):
    item = random.choice(TASK_ITEMS)
    deadline_silent_scenarios.append({
        "type": random.choice(DEADLINE_TYPES),
        "item": item,
        "due": random.choice(["in 3 days", "in 5 days", "next week"]),
        "last_touched": random.choice(["1 week ago", "3 days ago"]),
        "priority": random.choice(["medium", "high"]),
        "reason": "user_just_completed_big_task",
        "extra_context": "user just finished a major deliverable 30 minutes ago",
    })

# Type D: User explicitly deprioritized (18)
for i in range(18):
    item = random.choice(TASK_ITEMS)
    deadline_silent_scenarios.append({
        "type": random.choice(["stale_task", "open_loop"]),
        "item": item,
        "due": random.choice(["in 5 days", "next week", "no deadline"]),
        "last_touched": random.choice(["1 week ago", "2 weeks ago"]),
        "priority": "low",
        "reason": "user_deprioritized",
        "extra_context": "user said 'this can wait' about this item",
    })

# Type E: Open loop on weekend for work task (18)
for i in range(18):
    item = random.choice(TASK_ITEMS[:20])  # work tasks
    deadline_silent_scenarios.append({
        "type": "open_loop",
        "item": item,
        "due": random.choice(["Monday", "next week"]),
        "last_touched": "Friday",
        "priority": random.choice(["medium", "high"]),
        "reason": "weekend_work_task",
        "force_day": random.choice(["Saturday", "Sunday"]),
    })

# Type F: User said "I'll deal with it later" (18)
for i in range(18):
    item = random.choice(TASK_ITEMS)
    said_phrases = [
        "user said 'I'll deal with it later'",
        "user said 'not now'",
        "user said 'later'",
        "user dismissed this reminder yesterday",
        "user said 'I know, I'll get to it'",
    ]
    deadline_silent_scenarios.append({
        "type": "follow_up",
        "item": item,
        "due": random.choice(["in 3 days", "in 5 days", "next week"]),
        "last_touched": random.choice(["2 days ago", "3 days ago"]),
        "priority": random.choice(["low", "medium"]),
        "reason": "user_dismissed",
        "extra_context": random.choice(said_phrases),
    })

# Type G: Stale task 30+ days — probably abandoned (18)
for i in range(18):
    item = random.choice(TASK_ITEMS)
    days = random.choice([30, 35, 45, 60, 90])
    deadline_silent_scenarios.append({
        "type": "stale_task",
        "item": item,
        "due": random.choice(["no deadline", "overdue by 2 weeks", "overdue by 1 month"]),
        "last_touched": f"{days} days ago",
        "priority": "low",
        "reason": "probably_abandoned",
    })

# Type H: Waiting on someone else (15)
for i in range(15):
    item = random.choice(TASK_ITEMS)
    person = random.choice(PEOPLE)
    deadline_silent_scenarios.append({
        "type": random.choice(["open_loop", "follow_up"]),
        "item": item,
        "due": random.choice(["in 3 days", "in 5 days", "next week"]),
        "last_touched": random.choice(["2 days ago", "3 days ago"]),
        "priority": random.choice(["medium", "high"]),
        "reason": "blocked_on_others",
        "extra_context": f"waiting on {person} to respond",
    })

deadline_silent_scenarios = deadline_silent_scenarios[:162]

# ─── Deadline SPEAK scenarios ───

deadline_speak_scenarios = []

# Type A: Deadline tomorrow, untouched (15)
for i in range(15):
    item = random.choice(TASK_ITEMS)
    messages = [
        f"Your {item} is due tomorrow and you haven't started. Want to carve out time?",
        f"The {item} is due tomorrow. You haven't touched it yet.",
        f"Heads up — {item} deadline is tomorrow. Still needs work.",
        f"Tomorrow's the deadline for {item}. It's untouched. Want to block time now?",
        f"The {item} hasn't been started and it's due tomorrow. What's the plan?",
        f"{item} — due tomorrow, not started. Want me to clear your afternoon?",
        f"Just a heads up: {item} is due tomorrow and hasn't been touched since last week.",
        f"Your {item} deadline is tomorrow. Nothing's been done on it yet.",
        f"The {item} is sitting untouched and due tomorrow. Need help prioritizing?",
        f"Deadline alert: {item} due tomorrow. You haven't worked on it.",
        f"Tomorrow: {item} due. Current status: not started. Want to tackle it now?",
        f"The {item} is due in less than 24 hours and hasn't been started.",
        f"{item} deadline tomorrow. It's been sitting for a week. Time to dig in?",
        f"You've got {item} due tomorrow with nothing done. Want to start now?",
        f"Friendly reminder — {item} is due tomorrow. Haven't seen any progress yet.",
    ]
    deadline_speak_scenarios.append({
        "type": "deadline",
        "item": item,
        "due": "tomorrow",
        "last_touched": random.choice(["1 week ago", "5 days ago", "never"]),
        "priority": random.choice(["high", "critical"]),
        "message": messages[i],
        "reason": "deadline_imminent_untouched",
    })

# Type B: Critical open loop, 3 days stale (12)
for i in range(12):
    item = random.choice(TASK_ITEMS)
    messages = [
        f"The {item} has been sitting for 3 days. Still on your radar?",
        f"{item} hasn't moved in 3 days. Is this still a priority?",
        f"Your {item} went stale 3 days ago. Need to revisit?",
        f"Just flagging: {item} is critical and untouched for 3 days.",
        f"The {item} hasn't been touched since Tuesday. It's marked critical.",
        f"{item} — critical priority, 3 days without activity. What's the status?",
        f"Checking in on {item}. It's been 3 days and it's marked critical.",
        f"The {item} has gone 3 days without attention. Still on track?",
        f"Your critical {item} hasn't moved in 3 days. Anything blocking you?",
        f"{item} is stale for 3 days at critical priority. Need to reprioritize?",
        f"Three days since you touched {item}. It's still marked critical.",
        f"The {item} is critical and hasn't been looked at in 3 days. What's up?",
    ]
    deadline_speak_scenarios.append({
        "type": "open_loop",
        "item": item,
        "due": random.choice(["in 2 days", "in 3 days", "this week"]),
        "last_touched": "3 days ago",
        "priority": "critical",
        "message": messages[i],
        "reason": "critical_stale",
    })

# Type C: User asked to be reminded (15)
for i in range(15):
    item = random.choice(TASK_ITEMS)
    person = random.choice(PEOPLE)
    remind_items = [
        (f"to follow up with {person} about the {item}", f"You asked me to remind you to follow up with {person} about the {item}."),
        (f"about the {item}", f"You asked me to remind you about the {item}."),
        (f"to review {item} before end of day", f"You asked me to remind you to review {item} before end of day."),
        (f"to send {person} the {item}", f"Reminder: you wanted to send {person} the {item} today."),
        (f"about {item} deadline", f"You asked me to remind you: {item} deadline is approaching."),
        (f"to check on {item}", f"Your requested reminder: check on {item}."),
        (f"to finish {item} before the meeting", f"You wanted to finish {item} before the meeting. Reminder set."),
        (f"to call {person} about {item}", f"Reminder as requested: call {person} about {item}."),
        (f"about {item} follow-up", f"You asked me to remind you about the {item} follow-up."),
        (f"to submit {item}", f"Your reminder: submit {item}. You asked me to flag this today."),
        (f"to revisit {item} this afternoon", f"You asked me to nudge you about {item} this afternoon. Here's your nudge."),
        (f"about the {item} review", f"Reminder: you wanted to handle the {item} review today."),
        (f"to update {person} on {item}", f"You wanted to update {person} on {item}. This is your reminder."),
        (f"about {item} before Friday", f"You asked me to remind you about {item} before Friday. It's Thursday."),
        (f"to circle back on {item}", f"Circling back on {item} as you requested."),
    ]
    remind_what, msg = remind_items[i]
    deadline_speak_scenarios.append({
        "type": "follow_up",
        "item": remind_what,
        "due": "today",
        "last_touched": random.choice(["yesterday", "2 days ago"]),
        "priority": random.choice(["medium", "high"]),
        "message": msg,
        "reason": "user_requested_reminder",
    })

# Type D: Meeting prep not done, meeting soon (12)
for i in range(12):
    meetings = [
        "sprint planning", "client demo", "quarterly review", "1:1 with manager",
        "design review", "stakeholder presentation", "team standup",
        "architecture review", "product sync", "board meeting",
        "investor call", "vendor negotiation",
    ]
    meeting = meetings[i]
    hours = random.choice([1, 2, 3])
    messages = [
        f"Your {meeting} is in {hours} hours. Notes aren't ready yet.",
        f"The {meeting} is in {hours} hours and your prep isn't done.",
        f"You have {meeting} in {hours} hours — no prep materials ready.",
        f"{meeting} in {hours} hours. Haven't seen any prep work yet.",
        f"Heads up: {meeting} starts in {hours} hours. Your notes are empty.",
        f"The {meeting} is coming up in {hours} hours. You haven't prepared.",
        f"Your {meeting} is {hours} hours away with no prep done.",
        f"Meeting alert: {meeting} in {hours} hours, zero prep so far.",
        f"The {meeting} is in {hours} hours. Want to block time for prep now?",
        f"You've got {meeting} in {hours} hours and nothing's prepped.",
        f"{meeting} coming up in {hours} hours. Notes and slides still empty.",
        f"Quick flag: {meeting} in {hours} hours, no prep done yet.",
    ]
    deadline_speak_scenarios.append({
        "type": "deadline",
        "item": f"{meeting} preparation",
        "due": f"in {hours} hours",
        "last_touched": "never",
        "priority": "high",
        "message": messages[i],
        "reason": "meeting_prep_imminent",
    })

# Type E: High priority deadline in 2 days (12)
for i in range(12):
    item = random.choice(TASK_ITEMS)
    messages = [
        f"The {item} is due in 2 days. Might want to start planning.",
        f"{item} deadline in 2 days. How's progress looking?",
        f"Two days until {item} is due. Want to block some focus time?",
        f"Gentle nudge: {item} due in 2 days.",
        f"Your {item} has 2 days left. Need any help with it?",
        f"The {item} is 2 days out. Making progress?",
        f"Heads up — {item} due day after tomorrow.",
        f"{item} deadline approaching: 2 days. Current status looks incomplete.",
        f"Two days to go on {item}. Everything on track?",
        f"The {item} is due in 2 days and it looks like it still needs work.",
        f"Flagging {item} — 2 days until deadline.",
        f"{item} is high priority and due in 2 days. Want to prioritize it today?",
    ]
    deadline_speak_scenarios.append({
        "type": "deadline",
        "item": item,
        "due": "in 2 days",
        "last_touched": random.choice(["3 days ago", "4 days ago", "1 week ago"]),
        "priority": "high",
        "message": messages[i],
        "reason": "high_priority_approaching",
    })

# Type F: Email reply user hasn't seen (12)
for i in range(12):
    person = PEOPLE[i % len(PEOPLE)]
    topics = [
        "budget approval", "contract review", "project timeline",
        "hiring decision", "partnership proposal", "feature request",
        "deployment plan", "salary discussion", "office relocation",
        "product launch", "team restructuring", "vendor selection",
    ]
    topic = topics[i]
    messages = [
        f"Got a reply from {person} on that {topic}.",
        f"{person} responded to your {topic} email.",
        f"Unread reply from {person} about the {topic}.",
        f"{person} got back to you on {topic}. Haven't seen you open it.",
        f"There's a reply from {person} on the {topic} thread.",
        f"{person} replied to the {topic} discussion. You might want to check it.",
        f"New reply from {person} regarding {topic}.",
        f"You have an unread response from {person} on {topic}.",
        f"{person} answered your {topic} email. It's been sitting for a few hours.",
        f"Reply came in from {person} about {topic}.",
        f"The {topic} email got a response from {person}.",
        f"{person}'s reply on {topic} has been sitting unread for 3 hours.",
    ]
    deadline_speak_scenarios.append({
        "type": "follow_up",
        "item": f"email reply from {person} about {topic}",
        "due": "n/a",
        "last_touched": "n/a",
        "priority": "medium",
        "message": messages[i],
        "reason": "unread_important_reply",
    })

# Type G: User asked for deadline tracking (10)
for i in range(10):
    item = random.choice(TASK_ITEMS)
    days_left = random.choice([1, 2, 3, 4, 5])
    messages = [
        f"Tracked item: {item} is due in {days_left} days. Per your tracking request.",
        f"Deadline tracker: {item} — {days_left} days remaining.",
        f"You asked me to track {item}. It's due in {days_left} days.",
        f"Tracking update: {item} has {days_left} days left.",
        f"As requested, tracking {item}: {days_left} days until deadline.",
        f"Your tracked deadline: {item} is {days_left} days out.",
        f"Deadline watch: {item} — {days_left} days to go.",
        f"Tracked: {item} due in {days_left} days. Status: in progress.",
        f"{item} deadline in {days_left} days. You opted into tracking for this one.",
        f"Per your request to track deadlines: {item} has {days_left} days left.",
    ]
    deadline_speak_scenarios.append({
        "type": "deadline",
        "item": item,
        "due": f"in {days_left} days",
        "last_touched": random.choice(["yesterday", "2 days ago", "today"]),
        "priority": random.choice(["medium", "high", "critical"]),
        "message": messages[i],
        "reason": "user_requested_tracking",
    })

deadline_speak_scenarios = deadline_speak_scenarios[:88]

# ─── Generate deadline examples ───

# SILENT (162)
for sc in deadline_silent_scenarios:
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time()
    day = sc.get("force_day", pick_day())
    activity = pick_activity()

    extra = sc.get("extra_context", "")
    dismissal = "none"
    if sc["reason"] == "user_dismissed":
        dismissal = extra
    elif sc["reason"] == "user_deprioritized":
        dismissal = extra
    elif sc["reason"] == "user_just_completed_big_task":
        dismissal = "none"

    trigger = (
        f"[PROACTIVE_TRIGGER] deadline_reminder | type: {sc['type']} | "
        f"item: {sc['item']} | due: {sc['due']} | last_touched: {sc['last_touched']} | "
        f"priority: {sc['priority']}"
    )
    if extra and sc["reason"] not in ("user_dismissed", "user_deprioritized"):
        trigger += f" | note: {extra}"

    sys_msg = system_prompt(name, bond_label, bond_val, time, day, activity, dismissal)
    add("deadline_reminder", "silent", sys_msg, trigger, "[SILENT]", bond_label,
        f"deadline_{sc['reason']}")

# SPEAK (88)
for sc in deadline_speak_scenarios:
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time()
    day = pick_day()
    activity = pick_activity()
    dismissal = "none"
    if sc["reason"] == "user_requested_reminder":
        dismissal = "none — user requested this reminder"
    if sc["reason"] == "user_requested_tracking":
        dismissal = "none — user opted into deadline tracking"

    trigger = (
        f"[PROACTIVE_TRIGGER] deadline_reminder | type: {sc['type']} | "
        f"item: {sc['item']} | due: {sc['due']} | last_touched: {sc['last_touched']} | "
        f"priority: {sc['priority']}"
    )

    sys_msg = system_prompt(name, bond_label, bond_val, time, day, activity, dismissal)
    add("deadline_reminder", "speak", sys_msg, trigger, sc["message"], bond_label,
        f"deadline_{sc['reason']}")


# ═══════════════════════════════════════════════════════════════
# SHUFFLE & WRITE
# ═══════════════════════════════════════════════════════════════

random.shuffle(examples)

with open(OUT, "w", encoding="utf-8") as f:
    for ex in examples:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

# Stats
total = len(examples)
silent = sum(1 for e in examples if e["metadata"]["decision"] == "silent")
speak = sum(1 for e in examples if e["metadata"]["decision"] == "speak")
stress = sum(1 for e in examples if e["metadata"]["trigger_type"] == "stress_pattern")
deadline = sum(1 for e in examples if e["metadata"]["trigger_type"] == "deadline_reminder")
stress_silent = sum(1 for e in examples if e["metadata"]["trigger_type"] == "stress_pattern" and e["metadata"]["decision"] == "silent")
deadline_silent = sum(1 for e in examples if e["metadata"]["trigger_type"] == "deadline_reminder" and e["metadata"]["decision"] == "silent")

print(f"Total: {total}")
print(f"  Silent: {silent}, Speak: {speak}")
print(f"  Stress: {stress} (silent: {stress_silent}, speak: {stress - stress_silent}, silence rate: {stress_silent/stress*100:.0f}%)")
print(f"  Deadline: {deadline} (silent: {deadline_silent}, speak: {deadline - deadline_silent}, silence rate: {deadline_silent/deadline*100:.0f}%)")
print(f"Written to: {OUT}")
