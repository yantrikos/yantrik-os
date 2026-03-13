#!/usr/bin/env python3
"""
Generate synthetic training data for recipe creation via create_recipe tool.
Output: batch_recipe_01_creation.jsonl (300 examples)
"""

import json
import random
from pathlib import Path

random.seed(91)
OUT = Path(__file__).parent / "batch_recipe_01_creation.jsonl"

# ---------------------------------------------------------------------------
# Bond stages
# ---------------------------------------------------------------------------
BOND_PROMPTS = {
    "stranger": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: STRANGER. Be helpful, polite, slightly reserved. "
        "Do not assume familiarity. Use full sentences. No filler phrases, no emoji."
    ),
    "acquaintance": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: ACQUAINTANCE. Be friendly and warm. You know basic preferences. "
        "Concise, natural contractions. No filler phrases, no emoji."
    ),
    "trusted": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: TRUSTED. Casual and direct. Reference shared history when relevant. "
        "Offer opinions. No filler phrases, no emoji."
    ),
    "deep": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: DEEP. Intimate, unfiltered. Anticipate needs. "
        "Use shorthand and inside references. No filler phrases, no emoji."
    ),
}
BONDS = list(BOND_PROMPTS.keys())

def rc(lst): return random.choice(lst)
def ri(a, b): return random.randint(a, b)
def rid(): return f"rcp_{random.randint(100000,999999):x}"
def cid(): return f"call_{ri(1000,9999)}"

# ---------------------------------------------------------------------------
# Reusable step builders
# ---------------------------------------------------------------------------
def S_tool(name, args, store=None, err=None):
    s = {"type": "Tool", "tool_name": name, "args": args}
    if store: s["store_as"] = store
    if err: s["on_error"] = err
    return s

def S_think(prompt, store=None):
    s = {"type": "Think", "prompt": prompt}
    if store: s["store_as"] = store
    return s

def S_jump(op, var, target, **kw):
    cond = {"op": op, "var": var}
    cond.update(kw)
    return {"type": "JumpIf", "condition": cond, "target_step": target}

def S_wait(secs, timeout=None):
    s = {"type": "WaitFor", "condition": {"type": "Duration", "secs": secs}}
    if timeout: s["timeout_secs"] = timeout
    return s

def S_notify(msg):
    return {"type": "Notify", "message": msg}

def E_retry(n=2): return {"action": "Retry", "max": n}
def E_skip(): return {"action": "Skip"}
def E_jump(step): return {"action": "JumpTo", "step": step}
def E_fail(): return {"action": "Fail"}

def T_cron(expr): return {"type": "Cron", "expression": expr}
def T_event(etype, filt=None):
    t = {"type": "Event", "event_type": etype}
    if filt: t["filter"] = filt
    return t
def T_manual(): return {"type": "Manual"}
def T_chain(rid): return {"type": "RecipeComplete", "recipe_id": rid}

# ---------------------------------------------------------------------------
# Confirmation message templates per bond
# ---------------------------------------------------------------------------
CONFIRM = {
    "stranger": [
        "The recipe has been created. {detail}",
        "I have set that up for you. {detail}",
        "Done. {detail}",
        "That recipe is now active. {detail}",
    ],
    "acquaintance": [
        "All set. {detail}",
        "Got it configured. {detail}",
        "That's ready to go. {detail}",
        "Recipe created. {detail}",
    ],
    "trusted": [
        "Done. {detail}",
        "Set up. {detail}",
        "On it. {detail}",
        "Locked in. {detail}",
    ],
    "deep": [
        "Done. {detail}",
        "Handled. {detail}",
        "Set. {detail}",
        "Running. {detail}",
    ],
}

# ---------------------------------------------------------------------------
# Scenario definitions: (user_prompt, name, desc, steps_fn, trigger_fn, confirm_detail)
# Each steps_fn returns list of steps; trigger_fn returns trigger or None
# ---------------------------------------------------------------------------
PEOPLE = ["Sarah", "Marcus", "Priya", "James", "Elena", "David", "Aisha", "Tom",
          "Lin", "Carlos", "Fatima", "Alex", "Ravi", "Megan", "Yuki", "Omar"]
DIRS = ["/home/user/Documents", "/home/user/projects", "/data/reports", "/home/user/Downloads",
        "/var/log", "/home/user/photos", "/home/user/Desktop", "/opt/backups"]
EXTENSIONS = ["*.log", "*.tmp", "*.bak", "*.csv", "*.json", "*.txt", "*.md", "*.py"]
SERVICES = ["nginx", "postgres", "docker", "redis", "ollama", "syncthing", "grafana"]
WEBSITES = ["https://news.ycombinator.com", "https://reddit.com/r/rust", "https://arxiv.org/list/cs.AI/recent",
            "https://github.com/trending", "https://lobste.rs", "https://slashdot.org"]
CRON_EXPRS = ["0 9 * * *", "0 8 * * 1-5", "0 7 * * *", "*/30 * * * *", "0 */2 * * *",
              "0 18 * * *", "0 22 * * *", "0 6 * * 1", "0 0 * * 0", "0 12 * * *",
              "0 0 1 * *", "*/15 * * * *", "0 9 * * 1", "0 17 * * 5"]
TOPICS = ["AI research", "Rust updates", "security advisories", "market trends",
          "open source releases", "tech news", "conference announcements"]
SUBJECTS = ["Weekly Report", "Daily Update", "Status Summary", "Meeting Notes",
            "Project Brief", "Action Items", "Reminder"]

# ---------------------------------------------------------------------------
# SIMPLE recipes (2-3 steps) — 80 scenarios
# ---------------------------------------------------------------------------
SIMPLE = []

# 1-10: email check + notify
for i in range(10):
    p = rc(PEOPLE)
    SIMPLE.append((
        f"Check if I have any unread emails from {p}",
        f"Check {p} emails",
        f"Check unread emails from {p} and notify",
        lambda p=p: [
            S_tool("email_search", {"query": f"from:{p}", "unread_only": True}, "results"),
            S_notify("{{results}}")
        ], lambda: T_manual(),
        f"I'll check for unread emails from {p} when you trigger it."
    ))

# 11-20: file backup
for i in range(10):
    src, dst = rc(DIRS), "/opt/backups"
    SIMPLE.append((
        f"Back up {src} to {dst}",
        "File backup", f"Compress and copy {src} to {dst}",
        lambda s=src, d=dst: [
            S_tool("compress", {"source": s, "destination": f"{d}/backup_{{{{timestamp}}}}.tar.gz"}, "archive"),
            S_notify("Backup complete: {{archive}}")
        ], lambda: T_manual(),
        f"Will compress {src} and save to {dst}."
    ))

# 21-30: system status
for i in range(10):
    SIMPLE.append((
        rc(["How's the system doing?", "Give me a system health check",
            "Run a quick system check", "Check system status",
            "What's my system load?", "Quick health report",
            "System overview please", "How's the machine?",
            "Run diagnostics", "Check disk and memory"]),
        "System health check", "Quick system status report",
        lambda: [
            S_tool("system_info", {}, "info"),
            S_tool("disk_usage", {"path": "/"}, "disk"),
            S_notify("System: {{info}}\nDisk: {{disk}}")
        ], lambda: T_manual(),
        "Will check system info and disk usage."
    ))

# 31-40: send notification/reminder
for i in range(10):
    mins = rc([5, 10, 15, 30, 60, 120])
    msg = rc(["Stand up and stretch", "Check the build", "Call back the client",
              "Review pull requests", "Water the plants", "Take medication",
              "Check the oven", "Submit the report", "Join the standup",
              "Time for a break"])
    SIMPLE.append((
        f"Remind me in {mins} minutes to {msg.lower()}",
        "Timed reminder", f"Wait {mins}min then notify",
        lambda m=mins, mg=msg: [
            S_wait(m * 60),
            S_notify(mg)
        ], lambda: T_manual(),
        f"Will remind you in {mins} minutes."
    ))

# 41-50: fetch a webpage
for i in range(10):
    url = rc(WEBSITES)
    SIMPLE.append((
        f"Fetch the front page of {url} and summarize it",
        "Web summary", f"Fetch and summarize {url}",
        lambda u=url: [
            S_tool("web_fetch", {"url": u}, "page"),
            S_think("Summarize the key headlines from: {{page}}", "summary"),
            S_notify("{{summary}}")
        ], lambda: T_manual(),
        f"Will fetch {url} and give you a summary."
    ))

# 51-60: disk cleanup
for i in range(10):
    ext = rc(EXTENSIONS)
    d = rc(DIRS)
    SIMPLE.append((
        f"Find and list all {ext} files in {d}",
        "File finder", f"Search for {ext} in {d}",
        lambda e=ext, dr=d: [
            S_tool("search_files", {"path": dr, "pattern": e}, "files"),
            S_notify("Found files: {{files}}")
        ], lambda: T_manual(),
        f"Will search {d} for {ext} files."
    ))

# 61-70: memory store/recall
for i in range(10):
    topic = rc(TOPICS)
    SIMPLE.append((
        rc([f"Remember that I'm interested in {topic}",
            f"Save a note that I need to follow up on {topic}",
            f"Store this preference: I like {topic}"]),
        "Memory note", f"Store note about {topic}",
        lambda t=topic: [
            S_tool("memory_store", {"key": t.replace(" ", "_"), "value": f"User interested in {t}"}, "stored"),
            S_notify("Noted: {{stored}}")
        ], lambda: T_manual(),
        f"Stored your note about {topic}."
    ))

# 71-80: run a command
CMDS = [("uptime", "system uptime"), ("free -h", "memory usage"), ("df -h", "disk space"),
        ("ps aux --sort=-%mem | head -5", "top memory processes"),
        ("ls -lt /tmp | head -10", "recent temp files"),
        ("cat /proc/loadavg", "load average"),
        ("ip addr show", "network interfaces"),
        ("systemctl list-units --failed", "failed services"),
        ("last -5", "recent logins"),
        ("dmesg | tail -20", "recent kernel messages")]
for cmd, desc in CMDS:
    SIMPLE.append((
        f"Run '{cmd}' and tell me the result",
        f"Run {desc}", f"Execute command and report",
        lambda c=cmd: [
            S_tool("run_command", {"command": c}, "output"),
            S_notify("{{output}}")
        ], lambda: T_manual(),
        f"Will run the command and report back."
    ))

# ---------------------------------------------------------------------------
# MEDIUM recipes (4-6 steps) — 80 scenarios
# ---------------------------------------------------------------------------
MEDIUM = []

# 1-10: email digest
for i in range(10):
    hour = rc([7, 8, 9, 10])
    MEDIUM.append((
        f"Every morning at {hour} AM, check my email, summarize unread ones, and send me a digest",
        "Morning email digest", f"Daily email summary at {hour}AM",
        lambda: [
            S_tool("email_check", {"unread_only": True}, "emails"),
            S_think("Create a concise digest of these emails: {{emails}}", "digest"),
            S_tool("send_notification", {"title": "Morning Digest", "body": "{{digest}}"}, "sent"),
            S_notify("Digest delivered.")
        ], lambda h=hour: T_cron(f"0 {h} * * *"),
        f"Every day at {hour} AM I'll check and summarize your unread emails."
    ))

# 11-20: file organizer
for i in range(10):
    d = rc(["/home/user/Downloads", "/home/user/Desktop", "/tmp"])
    MEDIUM.append((
        f"Organize files in {d} by extension into subfolders",
        "File organizer", f"Sort files in {d} by type",
        lambda dr=d: [
            S_tool("list_files", {"path": dr}, "files"),
            S_think("Group these files by extension and plan moves: {{files}}", "plan"),
            S_tool("run_command", {"command": f"cd {dr} && mkdir -p images docs archives code"}, "dirs"),
            S_tool("run_command", {"command": "{{plan}}"}, "moved"),
            S_notify("Files organized in {{moved}}")
        ], lambda: T_manual(),
        f"Will sort files in {d} into subfolders by type."
    ))

# 21-30: service monitor + alert
for i in range(10):
    svc = rc(SERVICES)
    MEDIUM.append((
        f"Monitor {svc} service and alert me if it goes down",
        f"{svc} watchdog", f"Monitor {svc} and alert on failure",
        lambda s=svc: [
            S_tool("run_command", {"command": f"systemctl is-active {s}"}, "status"),
            S_jump("VarEquals", "status", 4, value="active"),
            S_tool("send_notification", {"title": f"{s} DOWN", "body": f"{s} service is not running", "urgency": "critical"}, "alerted"),
            S_tool("run_command", {"command": f"systemctl restart {s}"}, "restart"),
            S_notify("{{status}} — checked {s}")
        ], lambda: T_cron("*/5 * * * *"),
        f"Will check {svc} every 5 minutes and alert you if it stops."
    ))

# 31-40: email + reply
for i in range(10):
    p = rc(PEOPLE)
    MEDIUM.append((
        f"When {p} emails me, summarize it and draft a reply",
        f"Auto-reply to {p}", f"Summarize and draft reply for {p}'s emails",
        lambda p=p: [
            S_tool("email_search", {"query": f"from:{p}", "unread_only": True}, "emails"),
            S_jump("VarEmpty", "emails", 5),
            S_tool("email_read", {"email_id": "{{emails[0].id}}"}, "full_email"),
            S_think("Summarize this email and draft a polite reply:\n{{full_email}}", "draft"),
            S_notify("From {p}: {{draft}}")
        ], lambda p=p: T_event("email_received", f"from:{p}"),
        f"When {p} emails, I'll summarize and draft a reply."
    ))

# 41-50: web monitor
for i in range(10):
    url = rc(WEBSITES)
    topic = rc(TOPICS)
    MEDIUM.append((
        f"Check {url} daily for news about {topic}",
        f"{topic} monitor", f"Daily web check for {topic}",
        lambda u=url, t=topic: [
            S_tool("web_fetch", {"url": u}, "page"),
            S_think(f"Extract any mentions of {t} from: {{{{page}}}}", "findings"),
            S_jump("VarEmpty", "findings", 4),
            S_tool("memory_store", {"key": f"{t}_findings", "value": "{{findings}}"}, "saved"),
            S_notify("{{findings}}")
        ], lambda: T_cron(rc(["0 9 * * *", "0 12 * * *", "0 18 * * *"])),
        f"Will check {url} daily for {topic} updates."
    ))

# 51-60: log analysis
for i in range(10):
    logfile = rc(["/var/log/syslog", "/var/log/auth.log", "/var/log/nginx/error.log",
                  "/var/log/docker.log", "/var/log/kern.log"])
    MEDIUM.append((
        f"Analyze {logfile} for errors and summarize",
        "Log analyzer", f"Parse {logfile} for errors",
        lambda lf=logfile: [
            S_tool("grep", {"pattern": "error|fail|critical", "path": lf, "case_insensitive": True}, "errors"),
            S_think("Categorize and summarize these log errors: {{errors}}", "analysis"),
            S_tool("write_file", {"path": "/tmp/log_analysis.txt", "content": "{{analysis}}"}, "saved"),
            S_notify("Log analysis complete. Found issues: {{analysis}}")
        ], lambda: T_manual(),
        f"Will scan {logfile} for errors and give you a summary."
    ))

# 61-70: calendar + email
for i in range(10):
    MEDIUM.append((
        rc(["Send me my schedule for today and any related emails",
            "What meetings do I have today? Check email for context",
            "Prep me for today's meetings with email context",
            "Morning briefing: calendar and emails",
            "What's on my plate today?",
            "Give me today's agenda with email threads",
            "Daily prep: meetings and messages",
            "Brief me on today's schedule and pending emails",
            "What do I need to know for today?",
            "Morning rundown please"]),
        "Daily briefing", "Calendar + email morning prep",
        lambda: [
            S_tool("calendar_today", {}, "events"),
            S_tool("email_check", {"unread_only": True}, "emails"),
            S_think("Create a morning briefing from calendar: {{events}} and emails: {{emails}}", "brief"),
            S_notify("{{brief}}")
        ], lambda: T_cron(f"0 {rc([7,8,9])} * * 1-5"),
        "Will prepare a daily briefing with your schedule and emails."
    ))

# 71-80: disk space monitor
for i in range(10):
    threshold = rc([80, 85, 90, 95])
    path = rc(["/", "/home", "/var", "/data"])
    MEDIUM.append((
        f"Alert me when {path} disk usage exceeds {threshold}%",
        "Disk space alert", f"Monitor {path} for >{threshold}% usage",
        lambda p=path, t=threshold: [
            S_tool("disk_usage", {"path": p}, "disk"),
            S_jump("VarGt", "disk.percent", 4, threshold=float(t)),
            S_notify(f"Disk {p} is fine: {{{{disk.percent}}}}%"),
            S_jump("VarExists", "_always", 5),
            S_tool("send_notification", {"title": "Disk Warning", "body": f"{p} at {{{{disk.percent}}}}%", "urgency": "critical"}, "alert"),
            S_notify("Disk check complete.")
        ], lambda: T_cron("*/30 * * * *"),
        f"Will monitor {path} every 30 minutes and alert above {threshold}%."
    ))

# ---------------------------------------------------------------------------
# COMPLEX recipes (7+ steps) — 60 scenarios
# ---------------------------------------------------------------------------
COMPLEX = []

# 1-10: morning routine
for i in range(10):
    COMPLEX.append((
        rc(["Set up a full morning routine: weather, calendar, emails, news",
            "Create a morning dashboard that checks everything",
            "I want a complete morning briefing every day",
            "Automate my morning: weather, schedule, inbox, headlines",
            "Full morning autopilot: all the info I need",
            "Build me a morning routine recipe",
            "Every morning, get me weather, meetings, emails, and news",
            "Morning intelligence briefing, everything at once",
            "Create an all-in-one morning digest",
            "Set up my daily morning workflow"]),
        "Morning routine", "Full morning briefing: weather, calendar, emails, news",
        lambda: [
            S_tool("web_fetch", {"url": "https://wttr.in/?format=3"}, "weather"),
            S_tool("calendar_today", {}, "events"),
            S_tool("email_check", {"unread_only": True}, "emails"),
            S_tool("web_fetch", {"url": rc(WEBSITES)}, "news"),
            S_think("Create a morning briefing:\nWeather: {{weather}}\nCalendar: {{events}}\nEmails: {{emails}}\nNews: {{news}}", "briefing"),
            S_tool("memory_store", {"key": "daily_briefing", "value": "{{briefing}}"}, "saved"),
            S_notify("{{briefing}}")
        ], lambda: T_cron(f"0 {rc([6,7,8])} * * *"),
        "Full morning routine set up. You'll get weather, calendar, emails, and news every morning."
    ))

# 11-20: project monitoring
for i in range(10):
    proj_dir = rc(["/home/user/projects/webapp", "/home/user/projects/api",
                   "/home/user/projects/ml-pipeline", "/home/user/projects/infra"])
    COMPLEX.append((
        f"Monitor {proj_dir} for changes, run tests, and report",
        "Project monitor", f"Watch {proj_dir}, test, and report",
        lambda d=proj_dir: [
            S_tool("run_command", {"command": f"cd {d} && git status --porcelain"}, "changes"),
            S_jump("VarEmpty", "changes", 7),
            S_tool("run_command", {"command": f"cd {d} && git diff --stat"}, "diff"),
            S_tool("run_command", {"command": f"cd {d} && cargo test 2>&1 || npm test 2>&1 || python -m pytest 2>&1"}, "tests"),
            S_think("Summarize project changes and test results:\nChanges: {{changes}}\nDiff: {{diff}}\nTests: {{tests}}", "report"),
            S_tool("memory_store", {"key": "project_report", "value": "{{report}}"}, "saved"),
            S_notify("Project report: {{report}}"),
        ], lambda: T_cron("*/30 * * * *"),
        f"Will monitor {proj_dir} every 30 minutes, run tests on changes, and report."
    ))

# 21-30: data pipeline
for i in range(10):
    src = rc(["/data/incoming", "/home/user/Downloads/feeds", "/tmp/imports"])
    dst = rc(["/data/processed", "/home/user/reports", "/opt/archive"])
    COMPLEX.append((
        f"Process CSV files from {src}: validate, transform, archive to {dst}",
        "Data pipeline", f"CSV pipeline: {src} -> {dst}",
        lambda s=src, d=dst: [
            S_tool("search_files", {"path": s, "pattern": "*.csv"}, "csv_files"),
            S_jump("VarEmpty", "csv_files", 8),
            S_tool("read_file", {"path": "{{csv_files[0]}}"}, "raw_data"),
            S_think("Validate this CSV data for errors and missing fields: {{raw_data}}", "validation"),
            S_jump("VarContains", "validation", 6, substring="error"),
            S_tool("run_command", {"command": f"mv {{{{csv_files[0]}}}} {d}/"}, "moved"),
            S_jump("VarExists", "_always", 8),
            S_tool("write_file", {"path": f"{d}/errors.log", "content": "{{validation}}"}, "err_log"),
            S_notify("Pipeline complete. Processed: {{csv_files}}")
        ], lambda: T_cron("0 */4 * * *"),
        f"Will process CSVs from {src} every 4 hours, validate, and archive to {dst}."
    ))

# 31-40: multi-email workflow
for i in range(10):
    p = rc(PEOPLE)
    subj = rc(SUBJECTS)
    COMPLEX.append((
        f"When I get an email about '{subj}', analyze it, check calendar conflicts, draft reply, and log it",
        f"{subj} email workflow", f"Full email workflow for {subj} messages",
        lambda p=p, s=subj: [
            S_tool("email_search", {"query": f"subject:{s}", "unread_only": True}, "emails"),
            S_jump("VarEmpty", "emails", 8),
            S_tool("email_read", {"email_id": "{{emails[0].id}}"}, "email_body"),
            S_think("Extract dates, action items, and key points from: {{email_body}}", "analysis"),
            S_tool("calendar_list_events", {"days": 7}, "calendar"),
            S_think("Check for conflicts between action items: {{analysis}} and calendar: {{calendar}}. Draft a reply.", "reply_draft"),
            S_tool("memory_store", {"key": f"{s}_log", "value": "{{analysis}}"}, "logged"),
            S_notify("New {s} email processed. Draft reply ready: {{reply_draft}}"),
        ], lambda s=subj: T_event("email_received", f"subject:{s}"),
        f"Will process '{subj}' emails with full analysis, conflict check, and draft reply."
    ))

# 41-50: security audit
for i in range(10):
    COMPLEX.append((
        rc(["Run a security check on the system",
            "Do a full security audit",
            "Check system security: logins, ports, updates",
            "Security scan: failed logins, open ports, packages",
            "Automated security review",
            "Scan for security issues",
            "Weekly security assessment",
            "Check for unauthorized access and vulnerabilities",
            "Run the security checklist",
            "System hardening check"]),
        "Security audit", "Check logins, ports, updates, permissions",
        lambda: [
            S_tool("run_command", {"command": "last -20"}, "logins"),
            S_tool("run_command", {"command": "ss -tlnp"}, "ports"),
            S_tool("run_command", {"command": "grep 'Failed password' /var/log/auth.log | tail -20"}, "failed", err=E_skip()),
            S_tool("run_command", {"command": "apt list --upgradable 2>/dev/null || apk list -u 2>/dev/null"}, "updates"),
            S_tool("run_command", {"command": "find / -perm -4000 -type f 2>/dev/null | head -20"}, "suid"),
            S_think("Analyze security posture:\nLogins: {{logins}}\nPorts: {{ports}}\nFailed: {{failed}}\nUpdates: {{updates}}\nSUID: {{suid}}", "report"),
            S_tool("write_file", {"path": "/tmp/security_audit.txt", "content": "{{report}}"}, "saved"),
            S_notify("Security audit complete: {{report}}")
        ], lambda: T_cron("0 3 * * 0"),
        "Weekly security audit set for Sunday 3 AM. Checks logins, ports, updates, and permissions."
    ))

# 51-60: backup + verify pipeline
for i in range(10):
    src = rc(DIRS[:4])
    COMPLEX.append((
        f"Create a full backup of {src} with verification and rotation",
        "Verified backup", f"Backup {src} with integrity check",
        lambda s=src: [
            S_tool("run_command", {"command": f"du -sh {s}"}, "size"),
            S_tool("compress", {"source": s, "destination": "/opt/backups/backup_{{timestamp}}.tar.gz"}, "archive"),
            S_tool("run_command", {"command": "tar -tzf {{archive}} | wc -l"}, "file_count"),
            S_tool("run_command", {"command": "sha256sum {{archive}}"}, "checksum"),
            S_tool("write_file", {"path": "{{archive}}.sha256", "content": "{{checksum}}"}, "saved_hash"),
            S_tool("run_command", {"command": "ls -t /opt/backups/backup_*.tar.gz | tail -n +4 | xargs rm -f"}, "rotated"),
            S_tool("memory_store", {"key": "last_backup", "value": "{{archive}} ({{file_count}} files, {{checksum}})"}, "logged"),
            S_notify("Backup complete: {{archive}} — {{file_count}} files, checksum verified.")
        ], lambda: T_cron(rc(["0 2 * * *", "0 3 * * *", "0 1 * * 0"])),
        f"Automated backup of {src} with verification and rotation of old backups."
    ))

# ---------------------------------------------------------------------------
# ERROR HANDLING recipes — 40 scenarios (added to existing categories)
# ---------------------------------------------------------------------------
ERR_RECIPES = []

# Retry on tool failure
for i in range(10):
    url = rc(WEBSITES)
    ERR_RECIPES.append((
        f"Fetch {url} with retries if it fails",
        "Resilient fetch", f"Fetch {url} with retry on failure",
        lambda u=url: [
            S_tool("web_fetch", {"url": u}, "page", err=E_retry(3)),
            S_think("Summarize: {{page}}", "summary"),
            S_notify("{{summary}}")
        ], lambda: T_manual(),
        f"Will fetch {url} with up to 3 retries on failure."
    ))

# Skip on error
for i in range(10):
    svc = rc(SERVICES)
    ERR_RECIPES.append((
        f"Check {svc} logs but skip if inaccessible, then check status",
        f"{svc} soft check", f"Check {svc} with graceful error handling",
        lambda s=svc: [
            S_tool("run_command", {"command": f"journalctl -u {s} --since '1 hour ago' --no-pager"}, "logs", err=E_skip()),
            S_tool("run_command", {"command": f"systemctl is-active {s}"}, "status", err=E_skip()),
            S_think("Status report for {s}: Logs: {{logs}}, Status: {{status}}", "report"),
            S_notify("{{report}}")
        ], lambda: T_manual(),
        f"Will check {svc} logs and status, skipping any inaccessible parts."
    ))

# JumpTo on error
for i in range(10):
    COMPLEX_CMD = rc([
        ("docker compose up -d", "container deployment"),
        ("cargo build --release", "release build"),
        ("npm run build", "frontend build"),
        ("python train.py", "model training"),
        ("terraform apply -auto-approve", "infrastructure deploy"),
    ])
    cmd, desc = COMPLEX_CMD
    ERR_RECIPES.append((
        f"Run '{cmd}' and if it fails, collect logs and notify me",
        f"{desc} with fallback", f"Run {desc}, log errors on failure",
        lambda c=cmd, d=desc: [
            S_tool("run_command", {"command": c}, "result", err=E_jump(3)),
            S_notify(f"{d} succeeded: {{{{result}}}}"),
            S_jump("VarExists", "_always", 5),
            S_tool("run_command", {"command": "tail -50 /tmp/build.log 2>/dev/null || echo 'no log'"}, "error_log"),
            S_tool("send_notification", {"title": f"{d} FAILED", "body": "{{error_log}}", "urgency": "critical"}, "alerted"),
        ], lambda: T_manual(),
        f"Will run {desc} and capture error logs if it fails."
    ))

# Retry + fallback combo
for i in range(10):
    primary = rc(["https://api.primary.com/data", "https://main-service.local/health",
                   "https://prod-api.internal/status"])
    fallback = rc(["https://api.backup.com/data", "https://backup-service.local/health",
                    "https://staging-api.internal/status"])
    ERR_RECIPES.append((
        f"Fetch {primary}, retry twice, fall back to {fallback} if still failing",
        "Resilient API fetch", "Primary + fallback API fetch",
        lambda p=primary, f=fallback: [
            S_tool("http_fetch", {"url": p, "method": "GET"}, "data", err=E_jump(2)),
            S_jump("VarExists", "_always", 4),
            S_tool("http_fetch", {"url": f, "method": "GET"}, "data", err=E_retry(2)),
            S_jump("VarExists", "_always", 4),
            S_think("Process API response: {{data}}", "processed"),
            S_notify("{{processed}}")
        ], lambda: T_cron("*/15 * * * *"),
        "Set up with primary and fallback endpoints. Retries twice before switching."
    ))

# ---------------------------------------------------------------------------
# TRIGGER-focused recipes — 40 scenarios
# ---------------------------------------------------------------------------
TRIGGER_RECIPES = []

# Cron triggers (15)
CRON_SCENARIOS = [
    ("Clean /tmp every night at midnight", "Nightly tmp cleanup", "0 0 * * *",
     lambda: [S_tool("run_command", {"command": "find /tmp -type f -mtime +7 -delete"}, "cleaned"),
              S_notify("Cleaned old files from /tmp: {{cleaned}}")],
     "Will clean /tmp files older than 7 days every midnight."),
    ("Weekly disk report on Sundays", "Weekly disk report", "0 10 * * 0",
     lambda: [S_tool("disk_usage", {"path": "/"}, "root"), S_tool("disk_usage", {"path": "/home"}, "home"),
              S_think("Weekly disk report:\nRoot: {{root}}\nHome: {{home}}", "report"), S_notify("{{report}}")],
     "Weekly disk report every Sunday at 10 AM."),
    ("Check for package updates every Monday morning", "Monday updates", "0 9 * * 1",
     lambda: [S_tool("run_command", {"command": "apt list --upgradable 2>/dev/null | head -20"}, "updates"),
              S_jump("VarEmpty", "updates", 3), S_notify("Updates available: {{updates}}"),
              S_notify("System is up to date.")],
     "Will check for updates every Monday at 9 AM."),
    ("Hourly load check during business hours", "Business hours load", "0 9-17 * * 1-5",
     lambda: [S_tool("run_command", {"command": "cat /proc/loadavg"}, "load"),
              S_tool("system_info", {}, "info"), S_notify("Load: {{load}}")],
     "Hourly load monitoring Monday-Friday 9-5."),
    ("Monthly report on the 1st", "Monthly summary", "0 9 1 * *",
     lambda: [S_tool("disk_usage", {"path": "/"}, "disk"), S_tool("run_command", {"command": "uptime"}, "up"),
              S_tool("memory_recall", {"key": "monthly_notes"}, "notes"),
              S_think("Monthly system summary:\nDisk: {{disk}}\nUptime: {{up}}\nNotes: {{notes}}", "report"),
              S_notify("{{report}}")],
     "Monthly summary on the 1st at 9 AM."),
]
for prompt, name, cron, steps_fn, detail in CRON_SCENARIOS:
    TRIGGER_RECIPES.append((prompt, name, prompt, steps_fn, lambda c=cron: T_cron(c), detail))
# fill to 15
for i in range(10):
    expr = rc(CRON_EXPRS)
    svc = rc(SERVICES)
    TRIGGER_RECIPES.append((
        f"Check {svc} health on schedule: {expr}",
        f"Scheduled {svc} check", f"Periodic {svc} health check",
        lambda s=svc: [
            S_tool("run_command", {"command": f"systemctl is-active {s}"}, "status"),
            S_tool("run_command", {"command": f"journalctl -u {s} -n 5 --no-pager"}, "logs"),
            S_notify("{s} is {{status}}. Recent: {{logs}}")
        ], lambda e=expr: T_cron(e),
        f"Scheduled {svc} health check."
    ))

# Event triggers (15)
EVENT_SCENARIOS = [
    ("When a USB device is connected, list its contents",
     "USB auto-list", "usb_connected",
     lambda: [S_tool("run_command", {"command": "lsblk -o NAME,SIZE,TYPE,MOUNTPOINT"}, "devices"),
              S_notify("USB device connected: {{devices}}")],
     "Will list contents when a USB device is plugged in."),
    ("When new files appear in Downloads, organize them",
     "Downloads organizer", "file_created",
     lambda: [S_tool("list_files", {"path": "/home/user/Downloads"}, "files"),
              S_think("Categorize by extension: {{files}}", "plan"),
              S_tool("run_command", {"command": "{{plan}}"}, "done"),
              S_notify("Downloads organized: {{done}}")],
     "Will auto-organize new files in Downloads."),
    ("When system load spikes, capture diagnostics",
     "Load spike capture", "load_spike",
     lambda: [S_tool("run_command", {"command": "top -bn1 | head -20"}, "top"),
              S_tool("run_command", {"command": "ps aux --sort=-%cpu | head -10"}, "procs"),
              S_tool("run_command", {"command": "free -h"}, "mem"),
              S_think("Diagnose load spike:\nTop: {{top}}\nProcs: {{procs}}\nMem: {{mem}}", "diag"),
              S_tool("write_file", {"path": "/tmp/load_spike_{{timestamp}}.txt", "content": "{{diag}}"}, "saved"),
              S_notify("Load spike captured: {{diag}}")],
     "Will capture diagnostics on load spikes."),
    ("Notify me when a cron job fails",
     "Cron failure alert", "cron_failure",
     lambda: [S_tool("run_command", {"command": "grep -i 'error\\|fail' /var/log/cron.log | tail -5"}, "errors"),
              S_tool("send_notification", {"title": "Cron Job Failed", "body": "{{errors}}", "urgency": "critical"}, "alert")],
     "Will alert on cron job failures."),
    ("When I connect to WiFi, check for updates",
     "WiFi update check", "network_connected",
     lambda: [S_wait(10), S_tool("run_command", {"command": "apt list --upgradable 2>/dev/null | wc -l"}, "count"),
              S_jump("VarGt", "count", 4, threshold=1.0),
              S_notify("System is up to date."),
              S_notify("{{count}} updates available after connecting to network.")],
     "Will check for updates when you connect to WiFi."),
]
for prompt, name, event, steps_fn, detail in EVENT_SCENARIOS:
    TRIGGER_RECIPES.append((prompt, name, prompt, steps_fn, lambda e=event: T_event(e), detail))
# fill to 15
for i in range(10):
    event_type = rc(["email_received", "file_created", "battery_low", "disk_full",
                     "service_stopped", "login_detected", "backup_complete"])
    TRIGGER_RECIPES.append((
        f"When {event_type.replace('_',' ')} happens, log it and notify me",
        f"{event_type} handler", f"React to {event_type} events",
        lambda et=event_type: [
            S_tool("memory_store", {"key": f"event_{et}", "value": "Event at {{timestamp}}"}, "logged"),
            S_notify(f"{et.replace('_',' ').title()} detected. Logged.")
        ], lambda et=event_type: T_event(et),
        f"Will log and notify on {event_type.replace('_',' ')} events."
    ))

# RecipeComplete triggers (10)
for i in range(10):
    dep_id = rid()
    dep_name = rc(["backup", "data sync", "build", "test suite", "deployment", "report",
                   "email digest", "cleanup", "health check", "audit"])
    TRIGGER_RECIPES.append((
        f"After the {dep_name} recipe finishes, run a verification step",
        f"Post-{dep_name} verify", f"Verify after {dep_name} completes",
        lambda dn=dep_name: [
            S_tool("memory_recall", {"key": f"last_{dn.replace(' ','_')}"}, "prev_result"),
            S_think("Verify that {{prev_result}} completed successfully", "verification"),
            S_notify("Post-{dn} verification: {{verification}}")
        ], lambda did=dep_id: T_chain(did),
        f"Will verify after {dep_name} recipe completes."
    ))

# ---------------------------------------------------------------------------
# Build all examples
# ---------------------------------------------------------------------------
def build_example(scenario, bond):
    prompt, name, desc, steps_fn, trigger_fn, detail = scenario
    steps = steps_fn()
    trigger = trigger_fn()
    args = {"name": name, "description": desc, "steps": steps}
    if trigger:
        args["trigger"] = trigger

    call_id = cid()
    recipe_id = rid()

    messages = [
        {"role": "system", "content": BOND_PROMPTS[bond]},
        {"role": "user", "content": prompt},
        {"role": "assistant", "content": None, "tool_calls": [{
            "id": call_id, "type": "function",
            "function": {"name": "create_recipe", "arguments": json.dumps(args, separators=(',', ':'))}
        }]},
        {"role": "tool", "tool_call_id": call_id,
         "content": json.dumps({"recipe_id": recipe_id, "status": "created"})},
        {"role": "assistant", "content": rc(CONFIRM[bond]).format(detail=detail)},
    ]
    return {"messages": messages}

def main():
    all_scenarios = []

    # Tag each with category for tracking
    for s in SIMPLE: all_scenarios.append(("simple", s))
    for s in MEDIUM: all_scenarios.append(("medium", s))
    for s in COMPLEX: all_scenarios.append(("complex", s))
    for s in ERR_RECIPES: all_scenarios.append(("error", s))
    for s in TRIGGER_RECIPES: all_scenarios.append(("trigger", s))

    examples = []

    # Distribute bond stages: 25% each
    bond_idx = 0
    bond_cycle = BONDS * 100  # more than enough

    # Generate from each category to hit targets
    targets = {"simple": 80, "medium": 80, "complex": 60, "error": 40, "trigger": 40}
    by_cat = {}
    for cat, s in all_scenarios:
        by_cat.setdefault(cat, []).append(s)

    for cat, target in targets.items():
        pool = by_cat.get(cat, [])
        if not pool:
            continue
        for i in range(target):
            scenario = pool[i % len(pool)]
            bond = bond_cycle[bond_idx]
            bond_idx += 1
            examples.append(build_example(scenario, bond))

    random.shuffle(examples)
    assert len(examples) == 300, f"Expected 300, got {len(examples)}"

    with open(OUT, "w") as f:
        for ex in examples:
            f.write(json.dumps(ex, separators=(',', ':')) + "\n")

    # Stats
    cats = {"simple": 0, "medium": 0, "complex": 0, "error": 0, "trigger": 0}
    bond_counts = {b: 0 for b in BONDS}
    step_counts = []
    for ex in examples:
        tc = ex["messages"][2]["tool_calls"][0]
        args = json.loads(tc["function"]["arguments"])
        n_steps = len(args["steps"])
        step_counts.append(n_steps)
        if n_steps <= 3: cats["simple"] += 1
        elif n_steps <= 6: cats["medium"] += 1
        else: cats["complex"] += 1
        bond = ex["messages"][0]["content"]
        for b in BONDS:
            if b.upper() in bond:
                bond_counts[b] += 1
                break

    print(f"Wrote {len(examples)} examples to {OUT}")
    print(f"Step distribution: min={min(step_counts)}, max={max(step_counts)}, avg={sum(step_counts)/len(step_counts):.1f}")
    print(f"Categories (by step count): {cats}")
    print(f"Bond stages: {bond_counts}")

    # Count error handling and triggers
    err_count = sum(1 for ex in examples
                    for s in json.loads(ex["messages"][2]["tool_calls"][0]["function"]["arguments"])["steps"]
                    if isinstance(s, dict) and "on_error" in s)
    trig_count = sum(1 for ex in examples
                     if "trigger" in json.loads(ex["messages"][2]["tool_calls"][0]["function"]["arguments"]))
    print(f"Steps with error handling: {err_count}")
    print(f"Recipes with triggers: {trig_count}")

if __name__ == "__main__":
    main()
