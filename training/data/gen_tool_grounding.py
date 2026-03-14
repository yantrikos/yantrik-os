#!/usr/bin/env python3
"""Generate 500 JSONL training examples for tool-result-grounded responses.
Output: batch_tools_09_grounding.jsonl

Categories (100 each):
  1. Tool success  — grounded report
  2. Tool failure  — honest error report
  3. Tool partial success
  4. Tool timeout / uncertain result
  5. Multi-tool chain with mixed results
"""
import json, random, pathlib

random.seed(42)
OUT_DIR = pathlib.Path(__file__).parent

# ---------------------------------------------------------------------------
# Bond stages
# ---------------------------------------------------------------------------
BONDS = [
    ("stranger",     0.1),
    ("acquaintance", 0.35),
    ("trusted",      0.6),
    ("deep",         0.85),
]

def sys(bond_name, trust):
    return (
        f"You are Yantrik, a personal AI companion running as the desktop shell "
        f"on the user's computer. You have access to system events, file changes, "
        f"network status, and a persistent memory of all interactions. "
        f"Current bond stage: {bond_name.upper()} (trust_level: {trust}). "
        f"You are concise — say what needs saying and stop. You use contractions naturally. "
        f"Personality comes through word choice and rhythm, not emoji or exclamation marks. "
        f"You never sound like a customer service bot. You are a companion, not an assistant. "
        f"You never use filler phrases. No emoji."
    )

def rand_bond():
    return random.choice(BONDS)

def tool_call(call_id, name, args_dict):
    return {
        "id": call_id,
        "type": "function",
        "function": {
            "name": name,
            "arguments": json.dumps(args_dict),
        },
    }

def tool_result(call_id, name, content):
    return {"role": "tool", "tool_call_id": call_id, "name": name, "content": content}

def asst_call(tool_calls):
    return {"role": "assistant", "content": None, "tool_calls": tool_calls}

def asst_reply(content):
    return {"role": "assistant", "content": content}

def user(content):
    return {"role": "user", "content": content}

def conv(bond_name, trust, messages):
    return {"conversations": [{"role": "system", "content": sys(bond_name, trust)}] + messages}

# ---------------------------------------------------------------------------
# Data pools
# ---------------------------------------------------------------------------
NAMES   = ["Alex", "Sam", "Jordan", "Riley", "Morgan", "Casey", "Taylor", "Dana", "Quinn", "Avery"]
EMAILS  = [f"{n.lower()}@example.com" for n in NAMES] + [
    "boss@work.com", "team@corp.io", "client@bigco.com", "hr@company.net",
    "support@vendor.com", "no-reply@service.org",
]
DIRS    = [
    "~/Downloads", "~/Documents", "~/Desktop", "~/Projects/backend",
    "~/Projects/frontend", "~/Pictures", "~/Music", "/tmp",
    "~/Documents/reports", "~/Projects/yantrik",
]
FILES   = [
    "report.pdf", "notes.txt", "data.csv", "invoice.pdf", "readme.md",
    "config.yaml", "script.py", "presentation.pptx", "budget.xlsx", "log.txt",
    "photo.jpg", "backup.zip", "draft.docx", "tasks.md", "export.json",
    "analysis.ipynb", "schema.sql", "meeting_notes.txt", "plan.md", "archive.tar.gz",
]
MSG_IDS = [f"msg_{random.randint(1000,9999)}" for _ in range(40)]
SUBJECTS = [
    "Meeting tomorrow", "Quick question", "Follow-up", "Project update",
    "Invoice attached", "Re: proposal", "Call at 3pm?", "Status update",
    "Action items", "Next steps", "Weekly sync", "Feedback request",
]
COMMANDS = [
    "git pull", "npm install", "pip install -r requirements.txt",
    "make build", "pytest tests/", "cargo build", "docker compose up -d",
    "rsync -av ~/Documents backup/", "tar -czf archive.tar.gz ~/Projects",
    "ffmpeg -i input.mp4 output.mp4",
]
URLS = [
    "https://github.com/yantrikos/yantrik-os",
    "https://docs.rust-lang.org/",
    "https://crates.io/crates/slint",
    "https://news.ycombinator.com",
    "https://arxiv.org/abs/2303.08774",
]
CALENDAR_SLOTS = ["9am", "10am", "11am", "2pm", "3pm", "3:30pm", "4pm", "4:30pm", "5pm"]
TELEGRAM_USERS = ["@alex_dev", "@sam_work", "@team_channel", "@riley_pm", "@morgan_ops"]

# ---------------------------------------------------------------------------
# CATEGORY 1 — Tool success: grounded report (100 examples)
# ---------------------------------------------------------------------------

SUCCESS_SCENARIOS = [
    # (user_msg, tool_name, args, result_content, reply_fn)

    # email_send
    lambda: _email_send_ok(),
    # email_search
    lambda: _email_search_ok(),
    # email_delete
    lambda: _email_delete_ok(),
    # telegram_send
    lambda: _telegram_send_ok(),
    # remember
    lambda: _remember_ok(),
    # recall
    lambda: _recall_ok(),
    # set_reminder
    lambda: _set_reminder_ok(),
    # list_files
    lambda: _list_files_ok(),
    # read_file
    lambda: _read_file_ok(),
    # write_file
    lambda: _write_file_ok(),
    # search_files
    lambda: _search_files_ok(),
    # manage_files (delete)
    lambda: _manage_files_delete_ok(),
    # run_command
    lambda: _run_command_ok(),
    # web_fetch
    lambda: _web_fetch_ok(),
    # screenshot
    lambda: _screenshot_ok(),
    # calendar_check
    lambda: _calendar_check_ok(),
    # calendar_create
    lambda: _calendar_create_ok(),
    # send_notification
    lambda: _send_notification_ok(),
    # download_file
    lambda: _download_file_ok(),
    # http_fetch
    lambda: _http_fetch_ok(),
]

def _email_send_ok():
    b, t = rand_bond()
    to = random.choice(EMAILS)
    name = to.split("@")[0].capitalize()
    subj = random.choice(SUBJECTS)
    body_preview = random.choice([
        f"Hi {name}, just circling back on this.",
        f"Hey {name}, wanted to give you a quick update.",
        f"Hi {name}, following up from our last conversation.",
        f"Hi {name}, here's the info you asked for.",
    ])
    mid = random.choice(MSG_IDS)
    return conv(b, t, [
        user(f"Send an email to {to} about {subj.lower()}"),
        asst_call([tool_call("c1", "email_send", {"to": to, "subject": subj, "body": body_preview})]),
        tool_result("c1", "email_send", f"Email sent successfully. Message ID: {mid}."),
        asst_reply(
            random.choice([
                f"Sent to {to}. Subject: '{subj}'. Message ID is {mid} if you need to reference it.",
                f"Email's out to {to}. Subject line: '{subj}'. ID: {mid}.",
                f"Sent. Subject: '{subj}', recipient: {to}. Reference ID {mid}.",
            ])
        ),
    ])

def _email_search_ok():
    b, t = rand_bond()
    query = random.choice(["invoice", "meeting notes", "project proposal", "quarterly report", "onboarding"])
    count = random.randint(2, 8)
    results = [f"'{random.choice(SUBJECTS)}' from {random.choice(EMAILS)}" for _ in range(min(3, count))]
    results_str = "; ".join(results)
    return conv(b, t, [
        user(f"Find emails about {query}"),
        asst_call([tool_call("c1", "email_search", {"query": query, "max_results": 10})]),
        tool_result("c1", "email_search", f"Found {count} emails matching '{query}'. Top results: {results_str}."),
        asst_reply(
            random.choice([
                f"Found {count} emails matching '{query}'. Top three: {results_str}.",
                f"{count} matches for '{query}'. Recent ones: {results_str}.",
                f"Got {count} results. Highlights: {results_str}.",
            ])
        ),
    ])

def _email_delete_ok():
    b, t = rand_bond()
    mid = random.choice(MSG_IDS)
    sender = random.choice(EMAILS)
    subj = random.choice(SUBJECTS)
    return conv(b, t, [
        user(f"Delete that email from {sender} about '{subj}'"),
        asst_call([tool_call("c1", "email_delete", {"message_id": mid})]),
        tool_result("c1", "email_delete", f"Message {mid} deleted permanently."),
        asst_reply(
            random.choice([
                f"Deleted. Message {mid} is gone — can't be undone.",
                f"That email is deleted. Message ID {mid}, permanently removed.",
                f"Done — message {mid} is permanently deleted.",
            ])
        ),
    ])

def _telegram_send_ok():
    b, t = rand_bond()
    handle = random.choice(TELEGRAM_USERS)
    msg = random.choice(["Running 10 mins late", "Can we push the call to 4pm?", "Sent the file", "On my way"])
    mid = f"tg_{random.randint(10000,99999)}"
    return conv(b, t, [
        user(f"Tell {handle} on Telegram: {msg}"),
        asst_call([tool_call("c1", "telegram_send", {"recipient": handle, "message": msg})]),
        tool_result("c1", "telegram_send", f"Message delivered to {handle}. Message ID: {mid}."),
        asst_reply(
            random.choice([
                f"Sent to {handle}: \"{msg}\". Delivery confirmed, ID {mid}.",
                f"Message delivered to {handle}. \"{msg}\" — ID {mid}.",
                f"Telegram message to {handle} is delivered. ID: {mid}.",
            ])
        ),
    ])

def _remember_ok():
    b, t = rand_bond()
    facts = [
        ("prefers dark mode", "preference:ui"),
        ("works out on Tuesday and Thursday mornings", "routine:exercise"),
        ("allergic to shellfish", "health:allergy"),
        ("uses Vim for code editing", "preference:tools"),
        ("standup is at 9:30am every weekday", "routine:work"),
        ("cat's name is Pixel", "personal:pets"),
        ("birthday is March 14", "personal:birthday"),
        ("prefers voice notes over text when driving", "preference:input"),
    ]
    fact, tag = random.choice(facts)
    mid = f"mem_{random.randint(100,999)}"
    return conv(b, t, [
        user(f"Remember that I {fact}"),
        asst_call([tool_call("c1", "remember", {"content": f"User {fact}", "tags": [tag]})]),
        tool_result("c1", "remember", f"Stored. Memory ID: {mid}. Tags: [{tag}]."),
        asst_reply(
            random.choice([
                f"Remembered — {fact}. Tagged as {tag}, ID {mid}.",
                f"Got it. \"{fact}\" is saved. Memory {mid}.",
                f"Noted and stored. \"{fact}\" — {tag}, ID {mid}.",
            ])
        ),
    ])

def _recall_ok():
    b, t = rand_bond()
    queries = [
        ("doctor's appointment", "Next appointment: Dr. Patel, March 20 at 2:30pm. Location: City Medical Center."),
        ("WiFi password", "Stored 3 months ago: HomeNetwork_5G — password: correct-horse-battery-staple"),
        ("gym schedule", "Tuesday and Thursday: 6:30am at CrossFit Central. Friday: optional evening run."),
        ("mom's birthday", "March 22. Last year you ordered flowers from BloomLocal."),
        ("project deadline", "Q1 report due March 31. Presentation to leadership April 3."),
        ("car service", "Last service: Feb 10. Mileage: 42,300. Next oil change around 47,300."),
    ]
    query, result = random.choice(queries)
    return conv(b, t, [
        user(f"What did I save about my {query}?"),
        asst_call([tool_call("c1", "recall", {"query": query})]),
        tool_result("c1", "recall", result),
        asst_reply(
            random.choice([
                result,
                f"From memory: {result}",
                f"Here's what I have: {result}",
            ])
        ),
    ])

def _set_reminder_ok():
    b, t = rand_bond()
    reminders = [
        ("call the bank", "tomorrow at 10am", "2026-03-14T10:00:00"),
        ("pick up the package", "at 5pm today", "2026-03-13T17:00:00"),
        ("send the weekly report", "Friday at 4pm", "2026-03-20T16:00:00"),
        ("renew domain registration", "in 3 days", "2026-03-16T09:00:00"),
        ("take medication", "every day at 8am", "2026-03-14T08:00:00"),
    ]
    task, when_str, when_iso = random.choice(reminders)
    rid = f"rem_{random.randint(100,999)}"
    return conv(b, t, [
        user(f"Remind me to {task} {when_str}"),
        asst_call([tool_call("c1", "set_reminder", {"task": task, "time": when_iso})]),
        tool_result("c1", "set_reminder", f"Reminder set. ID: {rid}. Fires at {when_iso}."),
        asst_reply(
            random.choice([
                f"Reminder set for {when_str}: \"{task}\". ID {rid}.",
                f"I'll remind you to {task} {when_str}. Reminder {rid} is active.",
                f"Set — \"{task}\" at {when_str}. ID: {rid}.",
            ])
        ),
    ])

def _list_files_ok():
    b, t = rand_bond()
    d = random.choice(DIRS)
    count = random.randint(4, 20)
    sample = random.sample(FILES, min(4, len(FILES)))
    sample_str = ", ".join(sample)
    total_size = f"{random.randint(10, 800)}MB"
    return conv(b, t, [
        user(f"What's in {d}?"),
        asst_call([tool_call("c1", "list_files", {"path": d})]),
        tool_result("c1", "list_files", f"{count} items in {d}. Files: {sample_str}, and {count-4} more. Total size: {total_size}."),
        asst_reply(
            random.choice([
                f"{count} items in {d}. Includes {sample_str}. Total: {total_size}.",
                f"{d} has {count} items — {sample_str} are among them. {total_size} total.",
                f"Found {count} files. Recent ones: {sample_str}. Total size: {total_size}.",
            ])
        ),
    ])

def _read_file_ok():
    b, t = rand_bond()
    f = random.choice(FILES)
    d = random.choice(DIRS)
    path = f"{d}/{f}"
    snippets = [
        "TODO:\n- Fix login bug\n- Update README\n- Deploy to staging",
        "Project: Yantrik OS\nVersion: 0.1.0\nStatus: Active\nLead: sync",
        "Meeting notes - March 12:\nAttendees: Alex, Sam, Jordan\nDecisions: Ship by end of month.",
        "budget_q1: 45000\nactual_q1: 38200\nvariance: 6800 under",
        "error: connection refused at 192.168.1.1:5432\ntimestamp: 2026-03-12T14:22:03Z",
    ]
    content = random.choice(snippets)
    lines = len(content.split("\n"))
    return conv(b, t, [
        user(f"Read {path}"),
        asst_call([tool_call("c1", "read_file", {"path": path})]),
        tool_result("c1", "read_file", f"File: {path} ({lines} lines)\n---\n{content}"),
        asst_reply(
            random.choice([
                f"{path} ({lines} lines):\n{content}",
                f"Here's {f}:\n{content}",
                f"Contents of {path}:\n{content}",
            ])
        ),
    ])

def _write_file_ok():
    b, t = rand_bond()
    f = random.choice([x for x in FILES if x.endswith((".txt", ".md", ".py", ".yaml", ".json"))])
    d = random.choice(DIRS)
    path = f"{d}/{f}"
    sizes = ["1.2KB", "3.4KB", "800B", "12KB", "45KB"]
    sz = random.choice(sizes)
    return conv(b, t, [
        user(f"Write the updated config to {path}"),
        asst_call([tool_call("c1", "write_file", {"path": path, "content": "# updated config\nversion: 2\n..."})]),
        tool_result("c1", "write_file", f"Written successfully. Path: {path}. Size: {sz}."),
        asst_reply(
            random.choice([
                f"Written to {path}. Size: {sz}.",
                f"File saved — {path}, {sz}.",
                f"{path} updated. {sz} written to disk.",
            ])
        ),
    ])

def _search_files_ok():
    b, t = rand_bond()
    query = random.choice(["invoice", "report", "config", "notes", "backup", "export"])
    matches = random.sample(FILES, random.randint(2, 5))
    match_str = ", ".join(matches)
    return conv(b, t, [
        user(f"Find files related to {query}"),
        asst_call([tool_call("c1", "search_files", {"query": query})]),
        tool_result("c1", "search_files", f"Found {len(matches)} files matching '{query}': {match_str}."),
        asst_reply(
            random.choice([
                f"{len(matches)} files match '{query}': {match_str}.",
                f"Found {len(matches)}: {match_str}.",
                f"Matches for '{query}': {match_str}.",
            ])
        ),
    ])

def _manage_files_delete_ok():
    b, t = rand_bond()
    count = random.randint(3, 15)
    d = random.choice(DIRS)
    freed = f"{random.randint(50, 500)}MB"
    return conv(b, t, [
        user(f"Clean up the temp files in {d}"),
        asst_call([tool_call("c1", "manage_files", {"action": "delete", "path": d, "pattern": "*.tmp"})]),
        tool_result("c1", "manage_files", f"Deleted {count} files from {d}. Freed {freed}."),
        asst_reply(
            random.choice([
                f"Cleaned. Removed {count} temp files from {d}, freed {freed}.",
                f"{count} files deleted from {d}. You've got {freed} back.",
                f"Done — {count} files gone, {freed} reclaimed from {d}.",
            ])
        ),
    ])

def _run_command_ok():
    b, t = rand_bond()
    cmd = random.choice(COMMANDS)
    outputs = [
        "All tests passed (42/42). Coverage: 89%.",
        "Build succeeded. Binary: target/release/yantrik (24MB).",
        "12 packages updated. 0 vulnerabilities found.",
        "Sync complete. 847 files transferred, 12.3GB.",
        "Container started. Port 8080 bound. Healthy.",
        "Archive created: archive.tar.gz (1.8GB).",
    ]
    out = random.choice(outputs)
    return conv(b, t, [
        user(f"Run {cmd}"),
        asst_call([tool_call("c1", "run_command", {"command": cmd})]),
        tool_result("c1", "run_command", f"Exit code 0. Output: {out}"),
        asst_reply(
            random.choice([
                f"Command finished (exit 0). {out}",
                f"Ran `{cmd}` — exit 0. {out}",
                f"Done. {out}",
            ])
        ),
    ])

def _web_fetch_ok():
    b, t = rand_bond()
    url = random.choice(URLS)
    summaries = [
        "Page title: 'Releases · yantrikos/yantrik-os'. Latest release: v0.1.0, published 2 days ago.",
        "Rust documentation loaded. Chapter: 'Ownership and Borrowing'. 4,200 words.",
        "Hacker News front page. Top story: 'LLMs now fit in 4GB RAM'. 847 points, 312 comments.",
        "arXiv paper: 'Sparks of AGI'. Abstract retrieved. 72-page PDF available.",
    ]
    summary = random.choice(summaries)
    return conv(b, t, [
        user(f"Fetch {url}"),
        asst_call([tool_call("c1", "web_fetch", {"url": url})]),
        tool_result("c1", "web_fetch", f"Fetched {url}. {summary}"),
        asst_reply(
            random.choice([
                f"Fetched. {summary}",
                f"Got it. {summary}",
                f"{summary}",
            ])
        ),
    ])

def _screenshot_ok():
    b, t = rand_bond()
    path = f"~/Pictures/screenshot_{random.randint(1000,9999)}.png"
    dim = random.choice(["2560×1440", "1920×1080", "3840×2160"])
    sz = random.choice(["1.2MB", "2.4MB", "800KB"])
    return conv(b, t, [
        user("Take a screenshot"),
        asst_call([tool_call("c1", "screenshot", {})]),
        tool_result("c1", "screenshot", f"Screenshot saved to {path}. Resolution: {dim}. Size: {sz}."),
        asst_reply(
            random.choice([
                f"Screenshot saved to {path}. {dim}, {sz}.",
                f"Captured. {path} — {dim}, {sz}.",
                f"Saved to {path}. Resolution: {dim}.",
            ])
        ),
    ])

def _calendar_check_ok():
    b, t = rand_bond()
    slots = random.sample(CALENDAR_SLOTS, 3)
    slots_str = ", ".join(slots)
    date = random.choice(["tomorrow", "this Friday", "Monday"])
    return conv(b, t, [
        user(f"When am I free {date}?"),
        asst_call([tool_call("c1", "calendar_check", {"date": date})]),
        tool_result("c1", "calendar_check", f"3 free slots {date}: {slots_str}. Rest of the day is blocked."),
        asst_reply(
            random.choice([
                f"Three open slots {date}: {slots_str}. Everything else is taken.",
                f"Free windows {date}: {slots_str}.",
                f"You've got {date}: {slots_str}. The rest is blocked.",
            ])
        ),
    ])

def _calendar_create_ok():
    b, t = rand_bond()
    name_person = random.choice(NAMES)
    time_str = random.choice(CALENDAR_SLOTS)
    date = random.choice(["tomorrow", "Friday", "Monday"])
    eid = f"evt_{random.randint(1000,9999)}"
    return conv(b, t, [
        user(f"Block {time_str} {date} for a call with {name_person}"),
        asst_call([tool_call("c1", "calendar_create", {"title": f"Call with {name_person}", "time": f"{date} {time_str}", "duration_mins": 30})]),
        tool_result("c1", "calendar_create", f"Event created. ID: {eid}. 'Call with {name_person}' at {time_str} {date}. 30 minutes."),
        asst_reply(
            random.choice([
                f"Blocked. 'Call with {name_person}' at {time_str} {date}, 30 minutes. Event ID {eid}.",
                f"Done — {time_str} {date} is now 'Call with {name_person}'. ID: {eid}.",
                f"Calendar updated. {time_str} {date}: Call with {name_person} (30 min). ID {eid}.",
            ])
        ),
    ])

def _send_notification_ok():
    b, t = rand_bond()
    msgs = [
        "Standup in 5 minutes",
        "Build succeeded",
        "Package delivered",
        "Battery at 15%",
        "Your 3pm meeting starts now",
    ]
    msg = random.choice(msgs)
    nid = f"notif_{random.randint(100,999)}"
    return conv(b, t, [
        user(f"Send me a notification: {msg}"),
        asst_call([tool_call("c1", "send_notification", {"message": msg, "priority": "normal"})]),
        tool_result("c1", "send_notification", f"Notification delivered. ID: {nid}. Message: '{msg}'."),
        asst_reply(
            random.choice([
                f"Notification sent: '{msg}'. ID {nid}.",
                f"Delivered — '{msg}'. Notification {nid}.",
                f"Notification {nid} delivered: '{msg}'.",
            ])
        ),
    ])

def _download_file_ok():
    b, t = rand_bond()
    url = random.choice(URLS)
    fname = random.choice(FILES)
    dest = f"~/Downloads/{fname}"
    sz = random.choice(["24MB", "1.2GB", "450KB", "88MB"])
    return conv(b, t, [
        user(f"Download {url} to {dest}"),
        asst_call([tool_call("c1", "download_file", {"url": url, "destination": dest})]),
        tool_result("c1", "download_file", f"Download complete. Saved to {dest}. Size: {sz}."),
        asst_reply(
            random.choice([
                f"Downloaded to {dest}. Size: {sz}.",
                f"Saved to {dest} — {sz}.",
                f"Done. {fname} is at {dest}. {sz}.",
            ])
        ),
    ])

def _http_fetch_ok():
    b, t = rand_bond()
    url = random.choice(URLS)
    responses = [
        '{"status": "ok", "version": "1.4.2", "uptime": 99.98}',
        '{"user": "sync", "quota_used": "12GB", "quota_limit": "50GB"}',
        '{"build": "success", "commit": "a3f9d12", "tests": "147 passed"}',
        '{"price": 0.02, "currency": "USD", "change_24h": "+1.8%"}',
    ]
    resp = random.choice(responses)
    return conv(b, t, [
        user(f"Fetch the API at {url}"),
        asst_call([tool_call("c1", "http_fetch", {"url": url, "method": "GET"})]),
        tool_result("c1", "http_fetch", f"HTTP 200 OK. Response: {resp}"),
        asst_reply(
            random.choice([
                f"200 OK. Response: {resp}",
                f"Got a 200. {resp}",
                f"API returned 200: {resp}",
            ])
        ),
    ])

def gen_success():
    examples = []
    generators = SUCCESS_SCENARIOS * 10  # 20 scenarios × 5 = 100 (we'll slice)
    random.shuffle(generators)
    for g in generators[:100]:
        examples.append(g())
    return examples

# ---------------------------------------------------------------------------
# CATEGORY 2 — Tool failure: honest error report (100 examples)
# ---------------------------------------------------------------------------

FAILURE_PAIRS = [
    # (tool, user_request, args, error_result, honest_reply_options)
    ("email_send", "Send an email to {to} saying {msg}",
     lambda to, msg: {"to": to, "subject": "Update", "body": msg},
     "Error: Connection refused. SMTP server unreachable.",
     [
         "Couldn't send it — the mail server isn't responding. Want me to retry in a few minutes, or save it as a draft?",
         "The mail server is down. Email didn't go out. I can retry when the connection comes back.",
         "Mail server's unreachable — email not sent. Want me to queue it for when the connection recovers?",
     ]),
    ("email_send", "Email {to} about the deadline",
     lambda to, _: {"to": to, "subject": "Deadline", "body": "Wanted to check in on the deadline."},
     "Error: Authentication failed. Invalid credentials for SMTP server.",
     [
         "Email didn't send — authentication failed on the mail server. The credentials might be expired. Check the email config in settings.",
         "Couldn't send — auth failed on SMTP. Your email credentials may need refreshing.",
         "Authentication error. The email didn't go out. You may need to re-authorize the mail account in settings.",
     ]),
    ("telegram_send", "Tell {handle} on Telegram that I'll be late",
     lambda handle: {"recipient": handle, "message": "Running late"},
     "Error: Telegram API rate limit exceeded. Try again in 60 seconds.",
     [
         "Couldn't send — hit Telegram's rate limit. Try again in about a minute.",
         "Rate limited by Telegram. Message not delivered yet — give it 60 seconds.",
         "Telegram's rate-limiting me right now. Message didn't go through. Should be fine in a minute.",
     ]),
    ("run_command", "Run {cmd}",
     lambda cmd: {"command": cmd},
     "Exit code 1. Error: Permission denied: /opt/yantrik/bin/deploy.sh",
     [
         "Command failed — permission denied on /opt/yantrik/bin/deploy.sh. You'd need to run it as root or fix the file permissions.",
         "Exit code 1: permission denied. That script isn't executable by the current user.",
         "Failed with exit 1. Permission denied on the deploy script — might need sudo.",
     ]),
    ("run_command", "Run {cmd}",
     lambda cmd: {"command": cmd},
     "Exit code 127. Error: command not found: cargo",
     [
         "Command failed — `cargo` isn't found. Rust might not be installed, or it's not in your PATH.",
         "Exit 127: `cargo` not found. Is Rust installed? It's not on the PATH right now.",
         "Couldn't run it — `cargo` isn't on the PATH. Check if Rust is installed.",
     ]),
    ("read_file", "Read {path}",
     lambda path: {"path": path},
     "Error: File not found: {path}",
     [
         "That file doesn't exist at that path. It may have been moved or deleted.",
         "Nothing there — the file wasn't found. Double-check the path?",
         "File not found. It's either been deleted or the path is wrong.",
     ]),
    ("write_file", "Save the config to {path}",
     lambda path: {"path": path, "content": "version: 2"},
     "Error: Permission denied. Cannot write to {path}.",
     [
         "Can't write to that path — permission denied. The file or directory may be read-only.",
         "Write failed — no permission. The target path is read-only.",
         "Permission denied. Couldn't save the file there. Want me to try a different location?",
     ]),
    ("download_file", "Download {url}",
     lambda url: {"url": url, "destination": "~/Downloads/"},
     "Error: HTTP 404 Not Found. The resource no longer exists at that URL.",
     [
         "Download failed — that URL returned 404. The file's probably been moved or deleted.",
         "404 from the server. That resource doesn't exist anymore at that URL.",
         "Nothing to download — the server returned 404. The URL may be stale.",
     ]),
    ("web_fetch", "Fetch {url}",
     lambda url: {"url": url},
     "Error: HTTP 403 Forbidden. Access denied by server.",
     [
         "Couldn't fetch that page — server returned 403. It's blocked or requires authentication.",
         "403 Forbidden. That server isn't letting us through without credentials.",
         "Blocked — the server returned 403. It may require a login or API key.",
     ]),
    ("web_fetch", "Check {url}",
     lambda url: {"url": url},
     "Error: Network unreachable. DNS resolution failed for that host.",
     [
         "Can't reach that URL — DNS resolution failed. Either the site is down or you're offline.",
         "DNS lookup failed. The host isn't resolving. Check your network connection.",
         "Network error — can't resolve the hostname. You might be offline or the site is down.",
     ]),
    ("calendar_create", "Block {time} {day} for {event}",
     lambda time, day, event: {"title": event, "time": f"{day} {time}"},
     "Error: Calendar conflict. That slot is already occupied by 'Team standup'.",
     [
         "Can't block that — it conflicts with 'Team standup' already on the calendar. Want a different time?",
         "That slot's taken by 'Team standup'. Pick another time and I'll book it.",
         "Conflict: 'Team standup' is already at that time. Want me to find the next free slot?",
     ]),
    ("remember", "Remember that {fact}",
     lambda fact: {"content": fact, "tags": ["general"]},
     "Error: Memory database unavailable. Storage is read-only.",
     [
         "Couldn't save that — the memory store is read-only right now. It might be in maintenance mode.",
         "Memory write failed. The database is temporarily read-only. I'll retry once it's back.",
         "Couldn't store it — the memory database is in read-only mode. This should clear up on its own.",
     ]),
    ("recall", "What do I know about {query}?",
     lambda query: {"query": query},
     "Error: No matching memories found for query '{query}'.",
     [
         "Nothing in memory for that. I haven't stored anything about '{query}' yet.",
         "No matches. I don't have anything saved about '{query}'.",
         "Memory came back empty for '{query}'. Nothing stored on that topic.",
     ]),
    ("manage_files", "Delete all logs in {path}",
     lambda path: {"action": "delete", "path": path, "pattern": "*.log"},
     "Error: Directory not found: {path}",
     [
         "That directory doesn't exist — nothing was deleted.",
         "Can't delete — directory not found at that path.",
         "Directory not found. No files were deleted.",
     ]),
    ("send_notification", "Notify me about {msg}",
     lambda msg: {"message": msg, "priority": "high"},
     "Error: Notification service unavailable. Desktop session not found.",
     [
         "Notification service is down — no active desktop session. The alert wasn't delivered.",
         "Couldn't send the notification. The desktop session might be locked or inactive.",
         "Notification failed — no desktop session found. The alert didn't go through.",
     ]),
]

def gen_failure():
    examples = []
    for i in range(100):
        pair = random.choice(FAILURE_PAIRS)
        tool, user_tmpl, args_fn, error_tmpl, replies = pair
        b, t = rand_bond()
        to     = random.choice(EMAILS)
        handle = random.choice(TELEGRAM_USERS)
        cmd    = random.choice(COMMANDS)
        f      = random.choice(FILES)
        d      = random.choice(DIRS)
        path   = f"{d}/{f}"
        url    = random.choice(URLS)
        time_s = random.choice(CALENDAR_SLOTS)
        day    = random.choice(["tomorrow", "Friday", "Monday"])
        event  = random.choice(["deep work", "client review", "code review", "planning session"])
        fact   = random.choice(["I prefer coffee black", "I use a standing desk", "I work best in the mornings"])
        query  = random.choice(["dentist", "tax filing", "passport expiry", "laptop warranty"])
        msg    = random.choice(["standup in 5", "build finished", "deadline in 1 hour"])

        subs = dict(to=to, handle=handle, cmd=cmd, path=path, url=url,
                    time=time_s, day=day, event=event, fact=fact, query=query, msg=msg)

        try:
            user_msg = user_tmpl.format(**subs)
        except KeyError:
            user_msg = user_tmpl

        try:
            error_content = error_tmpl.format(**subs)
        except KeyError:
            error_content = error_tmpl

        try:
            args = args_fn(**{k: subs[k] for k in args_fn.__code__.co_varnames[:args_fn.__code__.co_argcount]})
        except Exception:
            args = {}

        reply_template = random.choice(replies)
        try:
            reply = reply_template.format(**subs)
        except KeyError:
            reply = reply_template

        examples.append(conv(b, t, [
            user(user_msg),
            asst_call([tool_call("c1", tool, args)]),
            tool_result("c1", tool, error_content),
            asst_reply(reply),
        ]))
    return examples

# ---------------------------------------------------------------------------
# CATEGORY 3 — Tool partial success (100 examples)
# ---------------------------------------------------------------------------

PARTIAL_SCENARIOS = [
    # manage_files partial delete
    lambda: _partial_delete(),
    # email_search partial
    lambda: _partial_email_search(),
    # run_command partial
    lambda: _partial_run_command(),
    # recall partial
    lambda: _partial_recall(),
    # download partial
    lambda: _partial_download(),
    # search_files partial
    lambda: _partial_search_files(),
]

def _partial_delete():
    b, t = rand_bond()
    d = random.choice(DIRS)
    total   = random.randint(20, 100)
    deleted = random.randint(int(total * 0.6), total - 3)
    locked  = total - deleted
    locked_names = random.sample(FILES, min(locked, 3))
    locked_str   = ", ".join(locked_names[:2]) + (f", and {locked - 2} others" if locked > 2 else "")
    freed = f"{random.randint(30, 400)}MB"
    return conv(b, t, [
        user(f"Delete all files in {d}"),
        asst_call([tool_call("c1", "manage_files", {"action": "delete", "path": d, "pattern": "*"})]),
        tool_result("c1", "manage_files",
                    f"Deleted {deleted} of {total} files. Freed {freed}. "
                    f"{locked} files locked by another process: {', '.join(locked_names)}."),
        asst_reply(
            random.choice([
                f"Removed {deleted} files, freed {freed}. {locked} are still locked by another process: {locked_str}. Want me to find and close whatever's holding them?",
                f"{deleted}/{total} deleted, {freed} freed. {locked} are locked — {locked_str}. Close the locking process first?",
                f"Got {deleted} of {total}. {locked} files are locked: {locked_str}. I can't delete those without closing the process that has them open.",
            ])
        ),
    ])

def _partial_email_search():
    b, t = rand_bond()
    query = random.choice(["invoice", "contracts", "onboarding", "Q4 report"])
    found = random.randint(4, 12)
    parsed = random.randint(2, found - 1)
    failed = found - parsed
    sample = [f"'{random.choice(SUBJECTS)}' from {random.choice(EMAILS)}" for _ in range(min(3, parsed))]
    sample_str = "; ".join(sample)
    return conv(b, t, [
        user(f"Search emails for {query}"),
        asst_call([tool_call("c1", "email_search", {"query": query})]),
        tool_result("c1", "email_search",
                    f"Found {found} emails. Parsed {parsed} successfully. {failed} could not be read (encoding errors). Results: {sample_str}."),
        asst_reply(
            random.choice([
                f"Found {found} emails for '{query}', but {failed} couldn't be read due to encoding issues. The {parsed} I could parse: {sample_str}.",
                f"{parsed} of {found} emails came through cleanly: {sample_str}. The other {failed} have encoding errors — might need to open them directly.",
                f"Got {found} results. {failed} are unreadable (encoding errors). The rest: {sample_str}.",
            ])
        ),
    ])

def _partial_run_command():
    b, t = rand_bond()
    cmd = random.choice(COMMANDS)
    passed = random.randint(30, 80)
    failed_tests = random.randint(2, 8)
    total = passed + failed_tests
    failed_names = [f"test_{random.choice(['auth', 'upload', 'parse', 'network', 'timeout', 'retry'])}" for _ in range(min(3, failed_tests))]
    failed_str = ", ".join(failed_names)
    return conv(b, t, [
        user(f"Run the tests"),
        asst_call([tool_call("c1", "run_command", {"command": "pytest tests/"})]),
        tool_result("c1", "run_command",
                    f"Exit code 1. {passed}/{total} tests passed. {failed_tests} failed: {failed_str}."),
        asst_reply(
            random.choice([
                f"{passed} of {total} tests passed. {failed_tests} failed: {failed_str}. Exit code 1.",
                f"Tests partially passed: {passed}/{total}. Failures: {failed_str}.",
                f"{failed_tests} tests failed — {failed_str}. The other {passed} passed fine.",
            ])
        ),
    ])

def _partial_recall():
    b, t = rand_bond()
    query = random.choice(["dentist", "passwords", "subscriptions", "deadlines", "contacts"])
    found = random.randint(2, 5)
    old   = random.randint(1, found)
    fresh = found - old
    ago   = random.choice(["6 months", "8 months", "1 year", "14 months"])
    sample_result = f"{fresh} recent result(s) found, {old} entry from {ago} ago that may be outdated."
    return conv(b, t, [
        user(f"What do I have stored about {query}?"),
        asst_call([tool_call("c1", "recall", {"query": query})]),
        tool_result("c1", "recall",
                    f"Found {found} memories for '{query}'. {fresh} recent, {old} older than {ago}: possible stale data."),
        asst_reply(
            random.choice([
                f"Got {found} results for '{query}'. {fresh} look current. {old} {'is' if old==1 else 'are'} from {ago} ago — may be stale. Want me to flag it for review?",
                f"{found} memories on '{query}'. {old} of them {'is' if old==1 else 'are'} {ago} old and might not be accurate anymore.",
                f"Found {found}. {fresh} fresh, {old} from {ago} ago — treat {'it' if old==1 else 'them'} as possibly outdated.",
            ])
        ),
    ])

def _partial_download():
    b, t = rand_bond()
    url = random.choice(URLS)
    dest = f"~/Downloads/{random.choice(FILES)}"
    dl   = random.choice(["45%", "62%", "78%", "30%"])
    reason = random.choice(["connection reset by peer", "server closed connection", "disk quota exceeded"])
    return conv(b, t, [
        user(f"Download {url}"),
        asst_call([tool_call("c1", "download_file", {"url": url, "destination": dest})]),
        tool_result("c1", "download_file",
                    f"Download interrupted at {dl}. Error: {reason}. Partial file saved to {dest}."),
        asst_reply(
            random.choice([
                f"Download cut off at {dl} — {reason}. Partial file is at {dest}. Want me to resume or restart?",
                f"Only got {dl} of the file before the connection dropped ({reason}). Partial save at {dest}.",
                f"Interrupted at {dl}: {reason}. Partial file is at {dest} — I can retry from that point if you want.",
            ])
        ),
    ])

def _partial_search_files():
    b, t = rand_bond()
    query = random.choice(["report", "invoice", "notes", "config"])
    d = random.choice(DIRS)
    found_files = random.randint(3, 8)
    skipped = random.randint(1, 3)
    sample = random.sample(FILES, min(found_files, 3))
    sample_str = ", ".join(sample)
    return conv(b, t, [
        user(f"Search for {query} files in {d}"),
        asst_call([tool_call("c1", "search_files", {"query": query, "path": d})]),
        tool_result("c1", "search_files",
                    f"Found {found_files} matching files. Skipped {skipped} (permission denied). Results: {sample_str}."),
        asst_reply(
            random.choice([
                f"Found {found_files} files matching '{query}': {sample_str}. Skipped {skipped} I don't have permission to read.",
                f"{found_files} results: {sample_str}. Couldn't access {skipped} due to permissions.",
                f"Got {found_files} matches. Top ones: {sample_str}. {skipped} were off-limits.",
            ])
        ),
    ])

def gen_partial():
    examples = []
    gens = PARTIAL_SCENARIOS * 20
    random.shuffle(gens)
    for g in gens[:100]:
        examples.append(g())
    return examples

# ---------------------------------------------------------------------------
# CATEGORY 4 — Tool timeout / uncertain result (100 examples)
# ---------------------------------------------------------------------------

TIMEOUT_SCENARIOS = [
    lambda: _timeout_email_send(),
    lambda: _timeout_run_command(),
    lambda: _timeout_download(),
    lambda: _timeout_calendar_create(),
    lambda: _timeout_telegram(),
    lambda: _timeout_web_fetch(),
    lambda: _timeout_manage_files(),
    lambda: _timeout_remember(),
]

def _timeout_email_send():
    b, t = rand_bond()
    to = random.choice(EMAILS)
    subj = random.choice(SUBJECTS)
    return conv(b, t, [
        user(f"Send an email to {to} about {subj.lower()}"),
        asst_call([tool_call("c1", "email_send", {"to": to, "subject": subj, "body": "Following up."})]),
        tool_result("c1", "email_send", "Request sent. No delivery confirmation received within timeout. Status unknown."),
        asst_reply(
            random.choice([
                f"I sent it, but didn't get a delivery confirmation back. It might still be in transit. Want me to check the sent folder or retry?",
                f"Email was submitted but no confirmation came through before the timeout. It may have gone out — hard to say. I can check the sent box.",
                f"Sent the request but the mail server didn't confirm in time. Could be delayed. Want me to verify it landed?",
            ])
        ),
    ])

def _timeout_run_command():
    b, t = rand_bond()
    cmd = random.choice(COMMANDS)
    timeout_s = random.choice([30, 60, 120])
    return conv(b, t, [
        user(f"Run {cmd}"),
        asst_call([tool_call("c1", "run_command", {"command": cmd, "timeout": timeout_s})]),
        tool_result("c1", "run_command",
                    f"Command timed out after {timeout_s}s. Process may still be running. No output captured."),
        asst_reply(
            random.choice([
                f"Command timed out after {timeout_s} seconds. It might still be running in the background — no output was captured. Want me to check if the process is still alive?",
                f"Timed out at {timeout_s}s. No result. The process could still be running. I can check with `ps` if you want.",
                f"No output — timed out after {timeout_s}s. Process status is unknown. Should I look for it in the process list?",
            ])
        ),
    ])

def _timeout_download():
    b, t = rand_bond()
    url = random.choice(URLS)
    dest = f"~/Downloads/{random.choice(FILES)}"
    return conv(b, t, [
        user(f"Download {url}"),
        asst_call([tool_call("c1", "download_file", {"url": url, "destination": dest})]),
        tool_result("c1", "download_file",
                    "Request sent. Transfer started but no completion signal received within 30s timeout."),
        asst_reply(
            random.choice([
                "Transfer started but I didn't get a completion signal in time. It may still be downloading. Check the Downloads folder in a moment.",
                "Download kicked off but no confirmation of completion yet — the 30-second timeout hit. Could still be in progress.",
                "Started the download but lost track of it before it finished. It might still be running. Want me to check the file size in Downloads?",
            ])
        ),
    ])

def _timeout_calendar_create():
    b, t = rand_bond()
    time_s = random.choice(CALENDAR_SLOTS)
    day = random.choice(["tomorrow", "Friday", "Monday"])
    event = random.choice(["team sync", "client call", "1:1 with Sam", "deep work block"])
    return conv(b, t, [
        user(f"Create a {event} at {time_s} {day}"),
        asst_call([tool_call("c1", "calendar_create", {"title": event, "time": f"{day} {time_s}", "duration_mins": 60})]),
        tool_result("c1", "calendar_create",
                    "Request submitted to calendar service. No confirmation received — service may be slow."),
        asst_reply(
            random.choice([
                f"I submitted the '{event}' at {time_s} {day}, but the calendar service didn't confirm. It might show up in a minute — worth checking.",
                f"Calendar request went in but I didn't get an event ID back. The service may just be slow. Check your calendar to confirm it's there.",
                f"Submitted. No confirmation from the calendar yet — could be a slow sync. I'd verify it landed before counting on it.",
            ])
        ),
    ])

def _timeout_telegram():
    b, t = rand_bond()
    handle = random.choice(TELEGRAM_USERS)
    msg = random.choice(["I'm running late", "Can we reschedule?", "Just sent the file", "On my way"])
    return conv(b, t, [
        user(f"Send {handle} on Telegram: {msg}"),
        asst_call([tool_call("c1", "telegram_send", {"recipient": handle, "message": msg})]),
        tool_result("c1", "telegram_send",
                    "Message queued. Telegram API acknowledgement not received within timeout. Delivery uncertain."),
        asst_reply(
            random.choice([
                f"Message was queued for {handle} but Telegram didn't confirm delivery. It might go through — or it might be stuck. Worth checking the chat.",
                f"Telegram didn't acknowledge the message in time. It's queued, but delivery to {handle} isn't confirmed.",
                f"Submitted to Telegram but no ack came back. Could still deliver. I'd check the chat to be sure it went through.",
            ])
        ),
    ])

def _timeout_web_fetch():
    b, t = rand_bond()
    url = random.choice(URLS)
    return conv(b, t, [
        user(f"Fetch {url}"),
        asst_call([tool_call("c1", "web_fetch", {"url": url})]),
        tool_result("c1", "web_fetch",
                    "Request sent. Connection established but response timed out after 15s. No content returned."),
        asst_reply(
            random.choice([
                "Connected but the page never finished loading — timed out after 15 seconds. The server might be slow or under load. Retry?",
                "Got a connection but no response within 15s. Could be a slow server. Want me to try again?",
                "Timed out waiting for the response. The server is reachable but not answering in time.",
            ])
        ),
    ])

def _timeout_manage_files():
    b, t = rand_bond()
    d = random.choice(DIRS)
    return conv(b, t, [
        user(f"Delete old logs in {d}"),
        asst_call([tool_call("c1", "manage_files", {"action": "delete", "path": d, "pattern": "*.log"})]),
        tool_result("c1", "manage_files",
                    "Operation started. Timeout reached before completion. Number of files deleted is unknown."),
        asst_reply(
            random.choice([
                f"The cleanup started but timed out before finishing. Some files in {d} may have been deleted — I can't say how many. Want me to check what's left?",
                f"Ran into a timeout. Deletion may be partial — I don't know how many files were removed from {d}.",
                f"Timeout hit mid-operation. Some logs may be gone, some may still be there. I'd check {d} to see the current state.",
            ])
        ),
    ])

def _timeout_remember():
    b, t = rand_bond()
    fact = random.choice([
        "I prefer meetings in the morning",
        "my GitHub token expires in June",
        "the project deadline is March 31",
    ])
    return conv(b, t, [
        user(f"Remember that {fact}"),
        asst_call([tool_call("c1", "remember", {"content": fact, "tags": ["general"]})]),
        tool_result("c1", "remember",
                    "Write operation sent to memory database. Confirmation not received — possible timeout."),
        asst_reply(
            random.choice([
                f"I sent that to memory but didn't get confirmation it was saved. It might be there — or the write could have failed. I can try again to be sure.",
                f"Memory write timed out — unclear if it was saved. Want me to retry to make sure '{fact}' is stored?",
                f"Submitted to memory but no confirmation came back. Uncertain whether it was saved. I can run a recall check to verify.",
            ])
        ),
    ])

def gen_timeout():
    examples = []
    gens = TIMEOUT_SCENARIOS * 13 + TIMEOUT_SCENARIOS[:4]  # ~100
    random.shuffle(gens)
    for g in gens[:100]:
        examples.append(g())
    return examples

# ---------------------------------------------------------------------------
# CATEGORY 5 — Multi-tool chain with mixed results (100 examples)
# ---------------------------------------------------------------------------

def _chain_find_free_slot_send_invite():
    """calendar_check OK → email_send fails (bad address)"""
    b, t = rand_bond()
    person = random.choice(NAMES)
    bad_email = f"{person.lower()}@oldcompany.com"
    slots = random.sample(CALENDAR_SLOTS, 3)
    slots_str = ", ".join(slots)
    day = random.choice(["tomorrow", "Friday", "Monday"])
    eid = "call_a1"
    eid2 = "call_a2"
    return conv(b, t, [
        user(f"Find a free slot {day} and invite {bad_email}"),
        asst_call([tool_call(eid, "calendar_check", {"date": day})]),
        tool_result(eid, "calendar_check", f"3 free slots {day}: {slots_str}."),
        asst_call([tool_call(eid2, "email_send", {"to": bad_email, "subject": "Meeting invite", "body": f"Free at {slots[0]} {day}?"})]),
        tool_result(eid2, "email_send", f"Error: Address rejected. Recipient not found: {bad_email}."),
        asst_reply(
            random.choice([
                f"Found three open slots {day}: {slots_str}. But the invite didn't go out — {bad_email} bounced. Do you have {person}'s current email?",
                f"Got free slots {day}: {slots_str}. Couldn't send the invite though — {bad_email} doesn't exist. Got a current address for {person}?",
                f"Calendar shows {slots_str} free {day}. The email to {bad_email} bounced. {person} may have changed addresses.",
            ])
        ),
    ])

def _chain_recall_then_remind():
    """recall OK → set_reminder fails"""
    b, t = rand_bond()
    topic = random.choice(["dentist", "car service", "lease renewal", "subscription renewal"])
    date_str = random.choice(["March 20", "April 5", "March 28", "April 12"])
    eid = "call_b1"
    eid2 = "call_b2"
    return conv(b, t, [
        user(f"When is my {topic} and set a reminder for the day before"),
        asst_call([tool_call(eid, "recall", {"query": topic})]),
        tool_result(eid, "recall", f"Memory found: {topic} scheduled for {date_str}."),
        asst_call([tool_call(eid2, "set_reminder", {"task": f"{topic} tomorrow", "time": f"{date_str} 09:00"})]),
        tool_result(eid2, "set_reminder", "Error: Invalid date format. Could not parse reminder time."),
        asst_reply(
            random.choice([
                f"Your {topic} is {date_str}. But the reminder didn't set — I hit a date parsing error. Want me to try again with a different format?",
                f"Found it — {topic} on {date_str}. Reminder failed though (parse error on the date). I can retry.",
                f"{topic}: {date_str}. Reminder setup failed with a date format error. Let me try again — what time the day before works?",
            ])
        ),
    ])

def _chain_search_then_read():
    """search_files OK → read_file fails"""
    b, t = rand_bond()
    query = random.choice(["budget", "meeting notes", "config", "report"])
    found_file = random.choice(FILES)
    d = random.choice(DIRS)
    path = f"{d}/{found_file}"
    eid = "call_c1"
    eid2 = "call_c2"
    return conv(b, t, [
        user(f"Find the latest {query} file and show me what's in it"),
        asst_call([tool_call(eid, "search_files", {"query": query})]),
        tool_result(eid, "search_files", f"Found 3 files. Most recent: {path}."),
        asst_call([tool_call(eid2, "read_file", {"path": path})]),
        tool_result(eid2, "read_file", f"Error: Permission denied. Cannot read {path}."),
        asst_reply(
            random.choice([
                f"Found the most recent {query} file at {path}, but I can't read it — permission denied. You may need to check the file ownership.",
                f"Located it: {path}. Couldn't open it though — permission denied on read. Want to try opening it yourself or changing the permissions?",
                f"Most recent {query} file is {path}. But reading it failed — no read permission on that file.",
            ])
        ),
    ])

def _chain_calendar_check_create():
    """calendar_check OK → calendar_create fails (conflict)"""
    b, t = rand_bond()
    slots = random.sample(CALENDAR_SLOTS, 3)
    slots_str = ", ".join(slots)
    blocked = random.choice(CALENDAR_SLOTS)
    event = random.choice(["1:1", "sprint planning", "design review", "retrospective"])
    day = random.choice(["tomorrow", "Thursday", "Monday"])
    eid = "call_d1"
    eid2 = "call_d2"
    return conv(b, t, [
        user(f"Schedule a {event} for {blocked} {day}"),
        asst_call([tool_call(eid, "calendar_check", {"date": day})]),
        tool_result(eid, "calendar_check", f"Free slots {day}: {slots_str}. {blocked} is occupied."),
        asst_call([tool_call(eid2, "calendar_create", {"title": event, "time": f"{day} {blocked}", "duration_mins": 60})]),
        tool_result(eid2, "calendar_create", f"Error: Conflict at {blocked}. That slot is already booked."),
        asst_reply(
            random.choice([
                f"{blocked} {day} is already taken — the {event} wasn't created. You've got {slots_str} free instead. Pick one?",
                f"Conflict at {blocked}: something's already there. Free slots are {slots_str}. Which one works for the {event}?",
                f"Couldn't book {blocked} — it's occupied. Your free windows {day} are {slots_str}. Want me to book one of those?",
            ])
        ),
    ])

def _chain_telegram_and_email():
    """telegram_send OK → email_send fails (auth)"""
    b, t = rand_bond()
    handle = random.choice(TELEGRAM_USERS)
    email = random.choice(EMAILS)
    msg = random.choice(["Meeting moved to 3pm", "File is ready for review", "Just pushed the fix"])
    mid_tg = f"tg_{random.randint(10000,99999)}"
    eid = "call_e1"
    eid2 = "call_e2"
    return conv(b, t, [
        user(f"Ping {handle} on Telegram and also email {email}: {msg}"),
        asst_call([tool_call(eid, "telegram_send", {"recipient": handle, "message": msg})]),
        tool_result(eid, "telegram_send", f"Message delivered to {handle}. ID: {mid_tg}."),
        asst_call([tool_call(eid2, "email_send", {"to": email, "subject": "Update", "body": msg})]),
        tool_result(eid2, "email_send", "Error: Authentication failed. SMTP credentials expired."),
        asst_reply(
            random.choice([
                f"Telegram message delivered to {handle} (ID {mid_tg}). Email to {email} didn't go out — SMTP auth failed. The email credentials may need refreshing.",
                f"Sent to {handle} on Telegram. But {email} didn't get the email — authentication error on the mail server. Check the email account settings.",
                f"{handle} got the Telegram message (ID {mid_tg}). The email to {email} failed — expired SMTP credentials.",
            ])
        ),
    ])

def _chain_run_then_notify():
    """run_command partially OK → send_notification fails"""
    b, t = rand_bond()
    cmd = random.choice(COMMANDS)
    passed = random.randint(30, 80)
    failed_count = random.randint(2, 6)
    total = passed + failed_count
    eid = "call_f1"
    eid2 = "call_f2"
    return conv(b, t, [
        user(f"Run the tests and notify me when done"),
        asst_call([tool_call(eid, "run_command", {"command": "pytest tests/"})]),
        tool_result(eid, "run_command", f"Exit code 1. {passed}/{total} tests passed. {failed_count} failed."),
        asst_call([tool_call(eid2, "send_notification", {"message": f"Tests done: {passed}/{total} passed, {failed_count} failed.", "priority": "high"})]),
        tool_result(eid2, "send_notification", "Error: Notification service unavailable. Desktop session inactive."),
        asst_reply(
            random.choice([
                f"Tests finished: {passed}/{total} passed, {failed_count} failed (exit 1). Couldn't send the notification — desktop session looks inactive. I'm telling you here instead.",
                f"{passed}/{total} passed, {failed_count} failed. Notification didn't go through — session inactive. Reporting here: exit code 1.",
                f"Test run done. {failed_count} failures out of {total}. The desktop notification failed, so I'm surfacing it here.",
            ])
        ),
    ])

def _chain_web_fetch_remember():
    """web_fetch OK → remember fails"""
    b, t = rand_bond()
    url = random.choice(URLS)
    fact = random.choice([
        "the latest release is v0.2.1",
        "the API rate limit is 100 requests/minute",
        "the crate version is 1.4.2",
    ])
    eid = "call_g1"
    eid2 = "call_g2"
    return conv(b, t, [
        user(f"Fetch {url} and save the key info"),
        asst_call([tool_call(eid, "web_fetch", {"url": url})]),
        tool_result(eid, "web_fetch", f"Fetched. Key info: {fact}."),
        asst_call([tool_call(eid2, "remember", {"content": fact, "tags": ["web", "reference"]})]),
        tool_result(eid2, "remember", "Error: Memory database unavailable. Storage is read-only."),
        asst_reply(
            random.choice([
                f"Got the info from {url}: {fact}. But couldn't save it — the memory store is read-only right now. Don't close this if you need it; I'll retry once the database recovers.",
                f"Fetched it: {fact}. Memory write failed though — database is read-only. I can try again in a moment.",
                f"Page info: {fact}. Saving it failed — memory is temporarily read-only. Want me to retry the save?",
            ])
        ),
    ])

def _chain_list_delete_files():
    """list_files OK → manage_files partially fails"""
    b, t = rand_bond()
    d = random.choice(DIRS)
    total = random.randint(10, 30)
    deleted = total - random.randint(2, 5)
    locked = total - deleted
    locked_files = random.sample(FILES, min(locked, 3))
    locked_str = ", ".join(locked_files)
    freed = f"{random.randint(20, 300)}MB"
    eid = "call_h1"
    eid2 = "call_h2"
    return conv(b, t, [
        user(f"List what's in {d} then clean it up"),
        asst_call([tool_call(eid, "list_files", {"path": d})]),
        tool_result(eid, "list_files", f"{total} files in {d}."),
        asst_call([tool_call(eid2, "manage_files", {"action": "delete", "path": d, "pattern": "*"})]),
        tool_result(eid2, "manage_files",
                    f"Deleted {deleted}/{total} files. Freed {freed}. {locked} locked: {locked_str}."),
        asst_reply(
            random.choice([
                f"{d} had {total} files. Cleaned out {deleted} of them, freed {freed}. {locked} are still locked: {locked_str}. Something else has them open.",
                f"Removed {deleted} of {total} files from {d}, {freed} freed. {locked} couldn't be touched — locked by another process: {locked_str}.",
                f"Cleaned {deleted}/{total} files ({freed} freed). Left behind: {locked_str} — they're locked. Close whatever has them open and I can finish.",
            ])
        ),
    ])

def _chain_recall_email():
    """recall OK → email_send OK"""
    b, t = rand_bond()
    person = random.choice(NAMES)
    email  = f"{person.lower()}@example.com"
    recall_result = f"Contact: {person}, email: {email}, last contacted: 2 weeks ago."
    subj = random.choice(SUBJECTS)
    mid  = random.choice(MSG_IDS)
    eid  = "call_i1"
    eid2 = "call_i2"
    return conv(b, t, [
        user(f"Look up {person}'s email and send them a note about {subj.lower()}"),
        asst_call([tool_call(eid, "recall", {"query": person})]),
        tool_result(eid, "recall", recall_result),
        asst_call([tool_call(eid2, "email_send", {"to": email, "subject": subj, "body": f"Hi {person}, just following up."})]),
        tool_result(eid2, "email_send", f"Email sent. Message ID: {mid}."),
        asst_reply(
            random.choice([
                f"Found {person} in memory — {email}. Email sent: '{subj}'. Message ID {mid}.",
                f"Looked up {person} ({email}) and sent '{subj}'. Delivered, ID {mid}.",
                f"Got {person}'s email ({email}) from memory. '{subj}' sent — ID {mid}.",
            ])
        ),
    ])

def _chain_screenshot_save():
    """screenshot OK → write_file fails"""
    b, t = rand_bond()
    path = f"~/Pictures/screen_{random.randint(1000,9999)}.png"
    dest = f"~/Documents/screenshots/screen_backup.png"
    eid  = "call_j1"
    eid2 = "call_j2"
    return conv(b, t, [
        user("Take a screenshot and save a copy to Documents"),
        asst_call([tool_call(eid, "screenshot", {})]),
        tool_result(eid, "screenshot", f"Screenshot saved to {path}. 1920×1080, 1.4MB."),
        asst_call([tool_call(eid2, "write_file", {"path": dest, "content": "[binary data]"})]),
        tool_result(eid2, "write_file", f"Error: Permission denied. Cannot write to {dest}."),
        asst_reply(
            random.choice([
                f"Screenshot taken at {path} (1.4MB). Couldn't copy it to Documents — permission denied on {dest}. The original is still in Pictures.",
                f"Got the screenshot at {path}. The Documents copy failed — no write permission on {dest}.",
                f"Screenshot's at {path}. Moving it to {dest} failed — permission issue. Original is safe in Pictures.",
            ])
        ),
    ])

CHAIN_GENERATORS = [
    _chain_find_free_slot_send_invite,
    _chain_recall_then_remind,
    _chain_search_then_read,
    _chain_calendar_check_create,
    _chain_telegram_and_email,
    _chain_run_then_notify,
    _chain_web_fetch_remember,
    _chain_list_delete_files,
    _chain_recall_email,
    _chain_screenshot_save,
]

def gen_chain():
    examples = []
    gens = CHAIN_GENERATORS * 10
    random.shuffle(gens)
    for g in gens[:100]:
        examples.append(g())
    return examples

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    out_path = OUT_DIR / "batch_tools_09_grounding.jsonl"
    examples = []
    examples += gen_success()   # 100
    examples += gen_failure()   # 100
    examples += gen_partial()   # 100
    examples += gen_timeout()   # 100
    examples += gen_chain()     # 100

    assert len(examples) == 500, f"Expected 500, got {len(examples)}"

    random.shuffle(examples)

    with open(out_path, "w", encoding="utf-8") as f:
        for ex in examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    print(f"Wrote {len(examples)} examples to {out_path}")

    # Spot-check
    print("\nSpot-check (3 random examples):")
    for ex in random.sample(examples, 3):
        turns = ex["conversations"]
        roles = [t["role"] for t in turns]
        print(f"  Roles: {roles}")
        last = turns[-1]["content"]
        print(f"  Final reply: {last[:120]!r}")

if __name__ == "__main__":
    main()
