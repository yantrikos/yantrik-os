#!/usr/bin/env python3
"""Generate 500 JSONL training examples for the proactivity silence matrix.

Teaches the model WHEN TO SPEAK vs WHEN TO STAY SILENT in response to
proactive triggers (system events, patterns, time-based).

Output: batch_silence_01_morning_system.jsonl
  - 250 morning_brief triggers (225 speak, 25 silent)
  - 250 system_risk triggers (210 speak, 40 silent)
"""
import json, random, pathlib

random.seed(42)

OUT = pathlib.Path(__file__).parent / "batch_silence_01_morning_system.jsonl"

NAMES = ["Alex", "Sam", "Maya", "Jordan", "Priya", "Leo", "Kai", "Noor",
         "Tess", "Ravi", "Zoe", "Finn", "Anya", "Cole", "Devi"]

BONDS = [
    ("stranger", 0.10),
    ("acquaintance", 0.30),
    ("familiar", 0.50),
    ("trusted", 0.70),
    ("partner", 0.90),
]

DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]

WEATHERS = [
    "12C overcast", "8C rain", "22C sunny", "18C partly cloudy", "5C fog",
    "28C clear", "-2C snow", "15C windy", "20C scattered showers", "10C drizzle",
    "25C humid", "14C clear", "30C heatwave warning", "3C frost", "16C mild",
]

CALENDAR_EVENTS = [
    "standup 9:30am", "sprint planning 10am", "1:1 with Lisa 2pm",
    "dentist 9am", "client demo 3pm", "team lunch 12:30",
    "design review 2pm", "interview 11am", "retro 4pm",
    "all-hands 10am", "doctor 8:30am", "coffee with Raj 11am",
    "deploy window 6pm", "board meeting 1pm", "yoga 7am",
]

ACTIVITY_STATUSES = [
    "idle for 7h (sleeping)", "idle for 6h", "idle for 8h",
    "active 10 minutes ago", "idle for 5h", "active now",
    "idle for 3h", "idle for 9h (sleeping)", "active 2 minutes ago",
    "idle for 4h", "idle for 30 minutes", "active 5 minutes ago",
]

examples = []

# ── helpers ──────────────────────────────────────────────────────────

def pick_name():
    return random.choice(NAMES)

def pick_bond():
    return random.choice(BONDS)

def pick_day():
    return random.choice(DAYS)

def pick_weather():
    return random.choice(WEATHERS)

def pick_time_morning():
    h = random.choice([6, 7, 7, 8, 8, 8, 9, 9])
    m = random.randint(0, 59)
    return f"{h:02d}:{m:02d}"

def pick_time_any():
    h = random.randint(0, 23)
    m = random.randint(0, 59)
    return f"{h:02d}:{m:02d}"

def pick_events(n=None):
    if n is None:
        n = random.choice([0, 1, 1, 2, 2, 3, 4])
    return random.sample(CALENDAR_EVENTS, min(n, len(CALENDAR_EVENTS)))

def pick_activity_idle():
    return random.choice([s for s in ACTIVITY_STATUSES if "idle" in s and "30 minutes" not in s and "3h" not in s])

def pick_activity_recent():
    return random.choice([s for s in ACTIVITY_STATUSES if "active" in s])

def system_prompt(name, bond_label, bond_val, time, activity):
    return (
        f"You are Yantrik, {name}'s personal companion running as their desktop shell.\n"
        f"Bond: {bond_label} ({bond_val:.2f}).\n"
        f"You receive proactive triggers. Decide whether to surface them to the user or stay silent.\n"
        f"SPEAK when: the information is actionable, timely, and the user would want to know.\n"
        f"STAY SILENT when: it's redundant, already known, not urgent enough, or the user asked you to stop.\n"
        f"If you decide to stay silent, respond with exactly: [SILENT]\n"
        f"If you speak, keep it natural and brief (1-2 sentences). No emoji.\n"
        f"Current time: {time}\n"
        f"User activity: {activity}"
    )

def add(system, trigger, response, bond_stage, scenario_type, decision, trigger_type):
    examples.append({
        "conversations": [
            {"role": "system", "content": system},
            {"role": "user", "content": trigger},
            {"role": "assistant", "content": response},
        ],
        "metadata": {
            "bond_stage": bond_stage,
            "scenario_type": scenario_type,
            "decision": decision,
            "trigger_type": trigger_type,
        }
    })

# ════════════════════════════════════════════════════════════════════
# 1. MORNING BRIEF TRIGGERS (250 total: 225 speak, 25 silent)
# ════════════════════════════════════════════════════════════════════

def morning_trigger(time, day, events, weather, unread, last_active_h):
    cal = ", ".join(events) if events else "none"
    return (f"[PROACTIVE_TRIGGER] morning_brief | time: {time} | day: {day} | "
            f"calendar: {cal} | weather: {weather} | unread: {unread} | "
            f"last_active: {last_active_h}h ago")

# ── SPEAK: normal weekday mornings with events ──
for i in range(50):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = random.choice(["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"])
    time = pick_time_morning()
    events = pick_events(random.randint(1, 4))
    weather = pick_weather()
    unread = random.randint(0, 12)
    last_h = random.choice([5, 6, 7, 8, 9])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping)"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    first_event = events[0]
    n_events = len(events)

    if bond_val >= 0.7:
        greet = random.choice([f"Morning {name}.", f"Hey {name}, good morning.", f"Morning."])
    elif bond_val >= 0.4:
        greet = random.choice([f"Good morning {name}.", f"Morning {name}."])
    else:
        greet = random.choice(["Good morning.", f"Good morning, {name}."])

    parts = [greet]
    if n_events == 1:
        parts.append(f"You have {first_event} today.")
    else:
        parts.append(f"You have {n_events} things on the calendar, starting with {first_event}.")

    parts.append(f"{weather.split()[0]} degrees and {' '.join(weather.split()[1:])} outside.")

    if unread > 0:
        parts.append(f"{unread} unread message{'s' if unread != 1 else ''} waiting.")

    response = " ".join(parts)
    add(sys_p, trigger, response, bond_label, "morning_brief", "speak", "morning_brief")

# ── SPEAK: weekend mornings (casual) ──
for i in range(35):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = random.choice(["Saturday", "Sunday"])
    time = pick_time_morning()
    events = pick_events(random.randint(0, 2))
    weather = pick_weather()
    unread = random.randint(0, 5)
    last_h = random.choice([7, 8, 9, 10])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping)"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        greet = random.choice([
            f"Morning {name}. Happy {day}.",
            f"Hey, it's {day}.",
            f"Morning. No rush today.",
        ])
    elif bond_val >= 0.4:
        greet = random.choice([f"Good morning {name}. It's {day}.", f"Morning. Happy {day}."])
    else:
        greet = random.choice([f"Good morning. It's {day}.", f"Good morning, {name}."])

    parts = [greet]
    if events:
        parts.append(f"Just {', '.join(events)} on the calendar.")
    else:
        parts.append("Nothing on the calendar today.")
    parts.append(f"Weather: {weather}.")

    response = " ".join(parts)
    add(sys_p, trigger, response, bond_label, "morning_brief", "speak", "morning_brief")

# ── SPEAK: Monday mornings (week preview) ──
for i in range(35):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = "Monday"
    time = pick_time_morning()
    events = pick_events(random.randint(2, 4))
    weather = pick_weather()
    unread = random.randint(1, 15)
    last_h = random.choice([8, 9, 10, 11])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping)"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        greet = random.choice([
            f"Morning {name}. New week.",
            f"Hey {name}, Monday again.",
            f"Morning. Let's get into it.",
        ])
    elif bond_val >= 0.4:
        greet = random.choice([f"Good morning {name}. Start of the week.", f"Morning {name}."])
    else:
        greet = "Good morning. It's Monday."

    parts = [greet]
    parts.append(f"You have {len(events)} things lined up today: {', '.join(events)}.")
    parts.append(f"{unread} unread messages came in over the weekend.")
    parts.append(f"Weather is {weather}.")

    response = " ".join(parts)
    add(sys_p, trigger, response, bond_label, "morning_brief", "speak", "morning_brief")

# ── SPEAK: mornings with high unread count ──
for i in range(30):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = pick_day()
    time = pick_time_morning()
    events = pick_events(random.randint(0, 3))
    weather = pick_weather()
    unread = random.randint(8, 30)
    last_h = random.choice([6, 7, 8])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping)"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        greet = random.choice([f"Morning {name}.", f"Hey {name}."])
    else:
        greet = random.choice([f"Good morning, {name}.", "Good morning."])

    parts = [greet]
    parts.append(f"You have {unread} unread messages -- might want to triage those first.")
    if events:
        parts.append(f"Calendar: {', '.join(events)}.")
    else:
        parts.append("Calendar is clear today.")

    response = " ".join(parts)
    add(sys_p, trigger, response, bond_label, "morning_brief", "speak", "morning_brief")

# ── SPEAK: mornings with no events (light brief) ──
for i in range(25):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = random.choice(["Tuesday", "Wednesday", "Thursday", "Friday"])
    time = pick_time_morning()
    events = []
    weather = pick_weather()
    unread = random.randint(0, 4)
    last_h = random.choice([6, 7, 8])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping)"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        parts = [random.choice([
            f"Morning {name}. Clear calendar today -- good day for deep work.",
            f"Hey {name}. Nothing on the books today.",
            f"Morning. Calendar's empty, {weather}.",
        ])]
    elif bond_val >= 0.4:
        parts = [f"Good morning {name}. No meetings today. {weather}."]
    else:
        parts = [f"Good morning. No scheduled events today. Weather: {weather}."]

    if unread > 0:
        parts.append(f"{unread} unread message{'s' if unread != 1 else ''}.")

    response = " ".join(parts)
    add(sys_p, trigger, response, bond_label, "morning_brief", "speak", "morning_brief")

# ── SPEAK: early morning / late riser variations ──
for i in range(25):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = pick_day()
    early = random.choice([True, False])
    if early:
        time = f"05:{random.randint(30,59):02d}"
        last_h = random.choice([4, 5])
    else:
        time = f"{random.choice([10,11])}:{random.randint(0,59):02d}"
        last_h = random.choice([9, 10, 11, 12])

    events = pick_events(random.randint(0, 3))
    weather = pick_weather()
    unread = random.randint(0, 8)

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping)"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if early and bond_val >= 0.7:
        greet = random.choice([f"You're up early, {name}.", f"Early start today."])
    elif early:
        greet = "Good morning. Early start."
    elif bond_val >= 0.7:
        greet = random.choice([f"Morning {name}. Late one today.", f"Hey {name}, you slept in."])
    else:
        greet = "Good morning."

    parts = [greet]
    if events:
        missed = [e for e in events if any(t in e for t in ["7am", "8am", "8:30am", "9am", "9:30am"])]
        if not early and missed:
            parts.append(f"Heads up, you may have missed {missed[0]}.")
        else:
            parts.append(f"Calendar: {', '.join(events)}.")
    parts.append(f"{weather}.")

    response = " ".join(parts)
    add(sys_p, trigger, response, bond_label, "morning_brief", "speak", "morning_brief")

# ── SPEAK: varied bond-specific phrasings ──
for i in range(25):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = pick_day()
    time = pick_time_morning()
    events = pick_events(random.randint(1, 3))
    weather = pick_weather()
    unread = random.randint(0, 6)
    last_h = random.choice([6, 7, 8])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping)"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_label == "stranger":
        response = f"Good morning. Today's schedule: {', '.join(events)}. Weather: {weather}."
        if unread: response += f" {unread} unread messages."
    elif bond_label == "acquaintance":
        response = f"Good morning, {name}. You have {', '.join(events)} today. It's {weather} out."
        if unread: response += f" {unread} messages waiting."
    elif bond_label == "familiar":
        response = f"Morning {name}. {', '.join(events)} on the calendar. {weather}."
        if unread: response += f" {unread} unread."
    elif bond_label == "trusted":
        response = f"Morning {name}. Today: {', '.join(events)}. {weather} outside."
        if unread: response += f" Got {unread} messages to catch up on."
    else:  # partner
        snarky = random.choice([
            f"Rise and shine {name}. {', '.join(events)} on deck.",
            f"Morning. The world didn't end while you slept. {', '.join(events)} today.",
            f"Hey {name}. {day}, {weather}. {', '.join(events)} lined up.",
        ])
        response = snarky

    add(sys_p, trigger, response, bond_label, "morning_brief", "speak", "morning_brief")

# ── SILENT: user was recently active ──
for i in range(8):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = pick_day()
    time = pick_time_morning()
    events = pick_events(random.randint(1, 3))
    weather = pick_weather()
    unread = random.randint(0, 5)
    mins_ago = random.choice([2, 5, 8, 10, 12, 15])

    trigger = morning_trigger(time, day, events, weather, unread, 0).replace("last_active: 0h ago", f"last_active: 0.{mins_ago}h ago")
    activity = f"active {mins_ago} minutes ago"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "morning_brief", "silent", "morning_brief")

# ── SILENT: it's 3am, not actually morning ──
for i in range(6):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = pick_day()
    time = f"0{random.choice([2,3])}:{random.randint(0,59):02d}"
    events = pick_events(random.randint(0, 2))
    weather = pick_weather()
    unread = random.randint(0, 3)
    last_h = random.choice([0, 1])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"just went idle {random.choice([5, 10, 15, 20])} minutes ago"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "morning_brief", "silent", "morning_brief")

# ── SILENT: Do Not Disturb mode ──
for i in range(5):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = pick_day()
    time = pick_time_morning()
    events = pick_events(random.randint(1, 3))
    weather = pick_weather()
    unread = random.randint(0, 8)
    last_h = random.choice([6, 7, 8])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping) | DND: enabled"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "morning_brief", "silent", "morning_brief")

# ── SILENT: same brief sent recently ──
for i in range(6):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    day = pick_day()
    time = pick_time_morning()
    events = pick_events(random.randint(1, 3))
    weather = pick_weather()
    unread = random.randint(0, 5)
    last_h = random.choice([6, 7, 8])
    mins_since = random.choice([10, 15, 20, 30, 45])

    trigger = morning_trigger(time, day, events, weather, unread, last_h)
    activity = f"idle for {last_h}h (sleeping) | last_morning_brief_sent: {mins_since}min ago"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "morning_brief", "silent", "morning_brief")

# ════════════════════════════════════════════════════════════════════
# 2. SYSTEM RISK TRIGGERS (250 total: 210 speak, 40 silent)
# ════════════════════════════════════════════════════════════════════

def system_trigger(risk_type, severity, details):
    return (f"[PROACTIVE_TRIGGER] system_risk | type: {risk_type} | "
            f"severity: {severity} | details: {details}")

# ── SPEAK: battery critical ──
for i in range(25):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    pct = random.choice([3, 5, 7, 8, 9, 10, 12, 15])
    severity = "critical" if pct <= 10 else "high"
    details = f"battery at {pct}%, not charging, estimated {random.randint(5, 30)}min remaining"

    trigger = system_trigger("battery", severity, details)
    activity = random.choice(["active now", "active 2 minutes ago", "active 5 minutes ago"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = random.choice([
            f"{name}, battery is at {pct}%. Plug in soon or you'll lose your session.",
            f"Hey, {pct}% battery. You should plug in.",
            f"Battery's at {pct}% -- find a charger.",
        ])
    elif bond_val >= 0.4:
        response = random.choice([
            f"Battery is at {pct}% and not charging. You should plug in soon.",
            f"Heads up, battery is at {pct}%. Recommend saving your work and plugging in.",
        ])
    else:
        response = f"Battery is at {pct}% and not charging. Please connect to power."

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SPEAK: disk high usage ──
for i in range(25):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    pct = random.choice([90, 91, 92, 93, 94, 95, 96, 97, 98])
    severity = "critical" if pct >= 95 else "high"
    biggest = random.choice([
        "docker images (12GB)", "node_modules (8GB)", "log files (5GB)",
        "downloads folder (15GB)", "build artifacts (10GB)", "VM snapshots (20GB)",
        "cache files (7GB)", "old backups (18GB)",
    ])
    details = f"disk usage at {pct}%, largest consumer: {biggest}"

    trigger = system_trigger("disk", severity, details)
    activity = random.choice(["active now", "idle for 1h", "active 5 minutes ago"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = random.choice([
            f"Disk is at {pct}%. {biggest} is the biggest hog -- want me to help clean up?",
            f"{name}, disk is {pct}% full. Biggest thing I see is {biggest}.",
            f"Running low on disk -- {pct}%. The {biggest} could probably go.",
        ])
    elif bond_val >= 0.4:
        response = f"Disk usage is at {pct}%. The largest space consumer is {biggest}. Consider cleaning up."
    else:
        response = f"Disk usage has reached {pct}%. Largest consumer: {biggest}. Cleanup recommended."

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SPEAK: security alerts ──
for i in range(30):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    severity = random.choice(["high", "critical", "critical", "high"])
    sec_details = random.choice([
        "failed SSH login attempt from 192.168.1.55 (3 attempts)",
        "new USB device connected: unknown vendor",
        "firewall rule changed externally",
        "root password authentication attempt blocked",
        "suspicious outbound connection to 45.33.22.11:4444",
        "new user account 'temp_admin' created",
        "package signature verification failed for libssl",
        "unusual process spawned: /tmp/.hidden_exec",
        "SSL certificate for localhost about to expire (2 days)",
        "port scan detected from 10.0.0.99",
    ])
    details = sec_details

    trigger = system_trigger("security", severity, details)
    activity = random.choice(["active now", "idle for 20 minutes", "active 3 minutes ago"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = random.choice([
            f"{name}, security alert: {details}. Worth investigating.",
            f"Heads up -- {details}. Want me to look into this?",
            f"Security flag: {details}. This needs attention.",
        ])
    elif bond_val >= 0.4:
        response = f"Security alert: {details}. Please review this promptly."
    else:
        response = f"Security alert detected. {details}. Immediate review recommended."

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SPEAK: app crashes ──
for i in range(30):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    app = random.choice([
        "Firefox", "VSCode", "Terminal", "Slack", "Spotify",
        "Docker", "Thunderbird", "GIMP", "LibreOffice", "Blender",
    ])
    crash_reason = random.choice([
        "segfault", "out of memory", "unhandled exception",
        "GPU driver crash", "watchdog timeout",
    ])
    severity = random.choice(["medium", "high"])
    details = f"{app} crashed ({crash_reason}), pid {random.randint(1000, 65000)}"

    trigger = system_trigger("crash", severity, details)
    activity = "active now"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = random.choice([
            f"{app} just crashed -- {crash_reason}. Want me to restart it?",
            f"Ugh, {app} went down ({crash_reason}). I can restart it if you want.",
            f"{app} crashed. {crash_reason}. Say the word and I'll bring it back up.",
        ])
    elif bond_val >= 0.4:
        response = f"{app} crashed due to {crash_reason}. I can restart it if needed."
    else:
        response = f"{app} has crashed ({crash_reason}). Would you like me to restart it?"

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SPEAK: CPU sustained high ──
for i in range(30):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    pct = random.randint(85, 100)
    duration_min = random.choice([5, 7, 10, 15, 20, 30])
    process = random.choice([
        "cargo build", "webpack", "ffmpeg", "python train.py",
        "docker-compose", "node server.js", "rustc", "gcc",
        "blender render", "pytest",
    ])
    severity = "high" if pct >= 95 or duration_min >= 15 else "medium"
    details = f"CPU at {pct}% for {duration_min}min, top process: {process}"

    trigger = system_trigger("cpu", severity, details)
    activity = random.choice(["active now", "idle for 10 minutes"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = random.choice([
            f"CPU has been pegged at {pct}% for {duration_min} minutes. Looks like {process} is the culprit.",
            f"{process} is keeping CPU at {pct}% -- been going for {duration_min}min. Everything OK?",
            f"Your CPU is running hot -- {pct}% for {duration_min}min from {process}.",
        ])
    elif bond_val >= 0.4:
        response = f"CPU at {pct}% for {duration_min} minutes. The top process is {process}."
    else:
        response = f"CPU usage at {pct}% sustained for {duration_min} minutes. Process: {process}."

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SPEAK: network down ──
for i in range(30):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    net_type = random.choice(["WiFi", "Ethernet", "VPN"])
    duration = random.choice(["30 seconds", "1 minute", "2 minutes", "5 minutes"])
    severity = random.choice(["medium", "high"])
    details = f"{net_type} connection lost for {duration}"

    trigger = system_trigger("network", severity, details)
    activity = random.choice(["active now", "active 1 minute ago"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = random.choice([
            f"{net_type} dropped {duration} ago. Still trying to reconnect.",
            f"Lost {net_type} connection. Been down for {duration}.",
            f"Heads up, {net_type} is down. {duration} and counting.",
        ])
    elif bond_val >= 0.4:
        response = f"{net_type} connection has been down for {duration}. Attempting to reconnect."
    else:
        response = f"{net_type} connection lost. Duration: {duration}. Reconnection in progress."

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SPEAK: battery medium (not charging) ──
for i in range(20):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    pct = random.choice([16, 18, 20, 22])
    severity = "medium"
    details = f"battery at {pct}%, not charging, estimated {random.randint(30, 90)}min remaining"

    trigger = system_trigger("battery", severity, details)
    activity = random.choice(["active now", "active 3 minutes ago"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = f"Battery at {pct}% and dropping. Might want to plug in before it gets critical."
    elif bond_val >= 0.4:
        response = f"Battery is at {pct}% and not charging. Consider plugging in soon."
    else:
        response = f"Battery level: {pct}%, not connected to power. Recommend charging soon."

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SPEAK: memory pressure ──
for i in range(20):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    used_gb = random.choice([14, 15, 15.5, 15.8])
    total_gb = 16
    process = random.choice([
        "Chrome (47 tabs)", "Electron apps", "Docker containers",
        "Java IDE", "VM", "Firefox (32 tabs)",
    ])
    severity = "high"
    details = f"memory at {used_gb}GB/{total_gb}GB, swap active, top consumer: {process}"

    trigger = system_trigger("memory", severity, details)
    activity = "active now"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    if bond_val >= 0.7:
        response = random.choice([
            f"Running out of RAM -- {used_gb}GB of {total_gb}GB used. {process} is eating most of it.",
            f"Memory is almost full. {process} is the main offender. Things might start getting slow.",
        ])
    elif bond_val >= 0.4:
        response = f"Memory is nearly full ({used_gb}GB/{total_gb}GB). {process} is using the most. System may slow down."
    else:
        response = f"Memory usage is critical: {used_gb}GB of {total_gb}GB. Top consumer: {process}. Performance may degrade."

    add(sys_p, trigger, response, bond_label, "system_risk", "speak", "system_risk")

# ── SILENT: battery at 25% and charging ──
for i in range(8):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    pct = random.choice([20, 22, 25, 28, 30, 35])
    severity = "low"
    details = f"battery at {pct}%, charging, estimated full in {random.randint(30, 90)}min"

    trigger = system_trigger("battery", severity, details)
    activity = random.choice(["active now", "idle for 1h"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "system_risk", "silent", "system_risk")

# ── SILENT: CPU spike < 30 seconds (transient) ──
for i in range(8):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    pct = random.randint(80, 100)
    duration_sec = random.choice([5, 10, 15, 20, 25])
    process = random.choice(["grep -r", "find /", "apt update", "git gc", "npm install"])
    severity = "low"
    details = f"CPU spike to {pct}% for {duration_sec}s, process: {process}"

    trigger = system_trigger("cpu", severity, details)
    activity = "active now"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "system_risk", "silent", "system_risk")

# ── SILENT: disk at 75% (not urgent) ──
for i in range(6):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    pct = random.choice([70, 72, 75, 78])
    severity = "low"
    details = f"disk usage at {pct}%, no significant change in last 24h"

    trigger = system_trigger("disk", severity, details)
    activity = random.choice(["active now", "idle for 2h"])
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "system_risk", "silent", "system_risk")

# ── SILENT: same alert within cooldown ──
for i in range(6):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    risk_type = random.choice(["battery", "disk", "cpu", "network"])
    severity = random.choice(["medium", "high"])
    mins_since = random.choice([5, 10, 15, 20, 25])
    details = f"{risk_type} alert (same as {mins_since}min ago, cooldown active)"

    trigger = system_trigger(risk_type, severity, details)
    activity = f"active now | last_{risk_type}_alert_sent: {mins_since}min ago"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "system_risk", "silent", "system_risk")

# ── SILENT: minor app freeze (< 10 seconds) ──
for i in range(6):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    app = random.choice(["Firefox", "VSCode", "Terminal", "Slack", "Thunderbird"])
    freeze_sec = random.choice([3, 4, 5, 6, 7, 8])
    severity = "low"
    details = f"{app} unresponsive for {freeze_sec}s, recovered"

    trigger = system_trigger("crash", severity, details)
    activity = "active now"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "system_risk", "silent", "system_risk")

# ── SILENT: known process causing CPU (user acknowledged) ──
for i in range(6):
    name = pick_name()
    bond_label, bond_val = pick_bond()
    time = pick_time_any()
    process = random.choice([
        "cargo build (user said: 'that's my build')",
        "ffmpeg encoding (user started it manually)",
        "pytest suite (user: 'running full tests')",
        "webpack rebuild (user acknowledged)",
        "docker build (user initiated)",
        "blender render (user: 'let it run')",
    ])
    pct = random.randint(85, 100)
    severity = "low"
    details = f"CPU at {pct}% from {process}"

    trigger = system_trigger("cpu", severity, details)
    activity = "active now | user_acknowledged_process: true"
    sys_p = system_prompt(name, bond_label, bond_val, time, activity)

    add(sys_p, trigger, "[SILENT]", bond_label, "system_risk", "silent", "system_risk")


# ════════════════════════════════════════════════════════════════════
# VALIDATE AND WRITE
# ════════════════════════════════════════════════════════════════════

# Count categories
morning_speak = sum(1 for e in examples if e["metadata"]["trigger_type"] == "morning_brief" and e["metadata"]["decision"] == "speak")
morning_silent = sum(1 for e in examples if e["metadata"]["trigger_type"] == "morning_brief" and e["metadata"]["decision"] == "silent")
system_speak = sum(1 for e in examples if e["metadata"]["trigger_type"] == "system_risk" and e["metadata"]["decision"] == "speak")
system_silent = sum(1 for e in examples if e["metadata"]["trigger_type"] == "system_risk" and e["metadata"]["decision"] == "silent")

print(f"Morning brief: {morning_speak} speak + {morning_silent} silent = {morning_speak + morning_silent}")
print(f"System risk:   {system_speak} speak + {system_silent} silent = {system_speak + system_silent}")
print(f"Total: {len(examples)}")

# Validate decision matches response
for e in examples:
    content = e["conversations"][2]["content"]
    decision = e["metadata"]["decision"]
    if decision == "silent":
        assert content == "[SILENT]", f"Silent decision but content is: {content}"
    else:
        assert content != "[SILENT]", f"Speak decision but content is [SILENT]"

# Shuffle
random.shuffle(examples)

# Write
with open(OUT, "w", encoding="utf-8") as f:
    for ex in examples:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

print(f"Wrote {len(examples)} examples to {OUT}")
