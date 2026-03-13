#!/usr/bin/env python3
"""Generate training data for recipe auto-healing and error recovery (200 examples).
Output: batch_recipe_02_autohealing.jsonl
"""
import json, random
from pathlib import Path

random.seed(73)
OUT = Path(__file__).parent / "batch_recipe_02_autohealing.jsonl"
_call_n = 0

def cid():
    global _call_n; _call_n += 1; return f"call_{_call_n}"

def rid(p="rcp_"):
    return p + "".join(random.choices("abcdefghijklmnopqrstuvwxyz0123456789", k=6))

def jd(o): return json.dumps(o, ensure_ascii=False)

BONDS = ["stranger", "acquaintance", "trusted", "deep"]
BOND_SYS = {
    "stranger": "You are Yantrik, a personal AI companion. Bond: STRANGER. Helpful, polite, slightly reserved. No emoji.",
    "acquaintance": "You are Yantrik, a personal AI companion. Bond: ACQUAINTANCE. Friendly, warm. No emoji.",
    "trusted": "You are Yantrik, a personal AI companion. Bond: TRUSTED. Casual, direct. No emoji.",
    "deep": "You are Yantrik, a personal AI companion. Bond: DEEP. Intimate, anticipate needs. No emoji.",
}

REPLAN_SYS = "You are Yantrik. A recipe step failed and needs replanning. Analyze the failure, then output ONLY a JSON array of replacement steps."
LEARN_SYS = "You are Yantrik. Record what you learned from this recipe failure."

# ── Replan scenario tables ──
# Each: (category, recipe_name, goal, completed_steps, failed_step_idx, failed_tool, error_msg, remaining_steps, replacement_steps, summary)
# We define templates per category and expand with variation pools

PEOPLE = ["Sarah", "Marcus", "Priya", "James", "Elena", "David", "Aisha", "Tom", "Kenji", "Olivia"]
PATHS = ["/home/user/documents", "/home/user/photos", "/data/exports", "/home/user/projects", "/tmp/workspace"]
URLS = ["https://api.weather.com/v3/forecast", "https://news-api.io/headlines", "https://stock-api.com/quotes",
        "https://calendar.example.com/events", "https://ci.internal/api/builds"]
ALT_URLS = ["https://backup-api.weather.com/v2/forecast", "https://alt-news.io/feed", "https://finance-backup.com/data",
            "https://calendar-mirror.example.com/events", "https://ci-backup.internal/api/builds"]
SERVICES = ["SMTP server", "calendar sync", "cloud storage", "CI pipeline", "RSS feed aggregator"]
FILES = ["report.pdf", "backup.tar.gz", "export.csv", "summary.txt", "data.json"]

def rand_person(): return random.choice(PEOPLE)
def rand_path(): return random.choice(PATHS)
def rand_file(): return random.choice(FILES)

# ── Category-specific scenario generators ──

def gen_smtp_scenarios(n):
    """SMTP/email auth failure -> fallback to notification"""
    errors = [
        "SMTP authentication failed: invalid credentials",
        "SMTP connection refused: port 587 blocked",
        "SMTP error 535: Authentication credentials invalid",
        "SMTP timeout: server did not respond within 30s",
        "SMTP TLS handshake failed: certificate expired",
    ]
    recipes = [
        ("Morning digest", "Check emails and send summary"),
        ("Daily report", "Compile metrics and email to team"),
        ("Meeting notes", "Summarize meeting and email attendees"),
        ("Weekly roundup", "Aggregate weekly updates and email"),
        ("Alert forwarding", "Forward critical alerts via email"),
    ]
    out = []
    for i in range(n):
        rname, goal = recipes[i % len(recipes)]
        err = errors[i % len(errors)]
        person = rand_person()
        completed = [{"step": 0, "tool": random.choice(["email_check", "web_fetch", "file_read", "summarize"]),
                       "result": random.choice(["12 unread emails", "3 new alerts", "report generated", "5 updates found"])}]
        failed_step = 1
        remaining = [{"step": 2, "type": "Notify", "message": "Done"}]
        replacement = jd([
            {"type": "Tool", "tool_name": "send_notification", "args": {"title": rname, "body": "{{summary}}"}, "store_as": "notif_result", "on_error": {"action": "Fail"}},
            {"type": "Notify", "message": f"{rname} delivered via notification (email was unavailable)"}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step {failed_step}: email_send error: '{err}'\n"
                f"Remaining: {jd(remaining)}")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_http_scenarios(n):
    """HTTP 404/500 -> try alternative URL or cached data"""
    errors = [
        ("404", "HTTP 404 Not Found: endpoint removed"),
        ("500", "HTTP 500 Internal Server Error"),
        ("502", "HTTP 502 Bad Gateway: upstream unavailable"),
        ("503", "HTTP 503 Service Unavailable: maintenance"),
        ("ConnectionError", "Connection refused: host unreachable"),
    ]
    recipes = [
        ("Weather check", "Fetch weather forecast and notify"),
        ("News digest", "Fetch headlines and summarize"),
        ("Stock update", "Get stock prices and alert on changes"),
        ("Calendar sync", "Fetch remote calendar events"),
        ("Build status", "Check CI build status"),
    ]
    out = []
    for i in range(n):
        rname, goal = recipes[i % len(recipes)]
        code, err = errors[i % len(errors)]
        url = URLS[i % len(URLS)]
        alt_url = ALT_URLS[i % len(ALT_URLS)]
        completed = [{"step": 0, "tool": "prepare_request", "result": f"URL: {url}"}]
        replacement = jd([
            {"type": "Tool", "tool_name": "web_fetch", "args": {"url": alt_url, "timeout_secs": 15}, "store_as": "fetch_result",
             "on_error": {"action": "Replan"}},
            {"type": "Tool", "tool_name": "send_notification", "args": {"title": rname, "body": "{{fetch_result}}"}, "store_as": "notif", "on_error": {"action": "Fail"}}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: web_fetch error: '{err}'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Data fetched\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_permission_scenarios(n):
    """Permission denied -> try alternative path or ask user"""
    errors = [
        "Permission denied: /etc/shadow",
        "Permission denied: /root/.ssh/config",
        "EACCES: cannot write to /opt/data/output.csv",
        "Permission denied: /var/log/syslog (requires root)",
        "EPERM: operation not permitted on /sys/class/backlight",
    ]
    paths_alt = ["/tmp/shadow_copy", "/home/user/.ssh/config", "/home/user/output.csv",
                 "/home/user/logs/syslog_copy", "/home/user/.config/backlight"]
    recipes = [
        ("Config backup", "Back up system configuration"),
        ("SSH setup", "Configure SSH keys"),
        ("Data export", "Export data to CSV"),
        ("Log analysis", "Analyze system logs"),
        ("Display config", "Adjust display settings"),
    ]
    out = []
    for i in range(n):
        rname, goal = recipes[i % len(recipes)]
        err = errors[i % len(errors)]
        alt = paths_alt[i % len(paths_alt)]
        completed = [{"step": 0, "tool": "list_files", "result": "found 3 target files"}]
        replacement = jd([
            {"type": "Notify", "message": f"Cannot access protected file directly. Trying alternative path."},
            {"type": "Tool", "tool_name": "file_copy", "args": {"src": alt, "dst": f"/home/user/backup/{rand_file()}"}, "store_as": "copy_result",
             "on_error": {"action": "Fail"}},
            {"type": "Notify", "message": "Completed with alternative path. Original location requires elevated permissions."}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: file_read error: '{err}'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Backup complete\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_tool_unavailable_scenarios(n):
    """Tool not available -> use alternative tool"""
    swaps = [
        ("screenshot_capture", "browser_screenshot", "Capture screen", {"path": "/tmp/screen.png"}),
        ("pdf_convert", "browser_pdf", "Convert page to PDF", {"path": "/tmp/page.pdf"}),
        ("speech_to_text", "file_read", "Transcribe audio", {"path": "/tmp/audio_notes.txt"}),
        ("calendar_create", "file_write", "Create calendar event", {"path": "/home/user/events.ics", "content": "{{event_ics}}"}),
        ("cloud_upload", "file_copy", "Upload to cloud", {"src": "/tmp/data.tar.gz", "dst": "/home/user/backup/data.tar.gz"}),
    ]
    recipes = [
        ("Screen capture workflow", "Capture and annotate screen"),
        ("Document pipeline", "Convert documents to PDF"),
        ("Voice notes", "Transcribe and save voice notes"),
        ("Event creation", "Create calendar events"),
        ("Cloud backup", "Upload files to cloud storage"),
    ]
    out = []
    for i in range(n):
        orig_tool, alt_tool, desc, alt_args = swaps[i % len(swaps)]
        rname, goal = recipes[i % len(recipes)]
        completed = [{"step": 0, "tool": "prepare", "result": "ready"}]
        replacement = jd([
            {"type": "Tool", "tool_name": alt_tool, "args": alt_args, "store_as": "alt_result", "on_error": {"action": "Fail"}},
            {"type": "Notify", "message": f"Used {alt_tool} as fallback since {orig_tool} is unavailable."}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: {orig_tool} error: 'Tool not found: {orig_tool} is not registered'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Done\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_timeout_scenarios(n):
    """Network timeout -> retry with shorter timeout or use cached"""
    recipes = [
        ("API sync", "Sync data from remote API"),
        ("Feed refresh", "Refresh RSS feeds"),
        ("Package update check", "Check for package updates"),
        ("Remote backup verify", "Verify remote backup integrity"),
        ("DNS check", "Check domain DNS records"),
    ]
    tools = ["web_fetch", "api_call", "http_get", "remote_check", "dns_lookup"]
    out = []
    for i in range(n):
        rname, goal = recipes[i % len(recipes)]
        tool = tools[i % len(tools)]
        timeout = random.choice([30, 60, 120])
        completed = [{"step": 0, "tool": "prepare", "result": "request prepared"}]
        replacement = jd([
            {"type": "Tool", "tool_name": tool, "args": {"url": URLS[i % len(URLS)], "timeout_secs": 10},
             "store_as": "retry_result", "on_error": {"action": "Skip"}},
            {"type": "JumpIf", "condition": {"op": "VarExists", "var": "retry_result"}, "target_step": 4},
            {"type": "Tool", "tool_name": "cache_read", "args": {"key": f"{rname.lower().replace(' ', '_')}_last"},
             "store_as": "cached_data", "on_error": {"action": "Fail"}},
            {"type": "Notify", "message": f"Used cached data (live fetch timed out after {timeout}s)."}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: {tool} error: 'Request timed out after {timeout}s'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Sync complete\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_invalid_json_scenarios(n):
    """Invalid JSON response -> add validation step"""
    bad_responses = [
        "Unexpected token '<' at position 0 (received HTML instead of JSON)",
        "JSON parse error: unterminated string at line 3",
        "Expected object, got array at root",
        "Invalid UTF-8 sequence in JSON response",
        "JSON parse error: trailing comma at position 245",
    ]
    recipes = [
        ("Data pipeline", "Fetch and process structured data"),
        ("API integration", "Pull data from external API"),
        ("Config loader", "Load remote configuration"),
    ]
    out = []
    for i in range(n):
        rname, goal = recipes[i % len(recipes)]
        err = bad_responses[i % len(bad_responses)]
        completed = [{"step": 0, "tool": "web_fetch", "result": "response received (raw)"}]
        replacement = jd([
            {"type": "Tool", "tool_name": "web_fetch", "args": {"url": URLS[i % len(URLS)], "headers": {"Accept": "application/json"}, "timeout_secs": 15},
             "store_as": "raw_response", "on_error": {"action": "Fail"}},
            {"type": "Think", "prompt": "Validate and extract JSON from {{raw_response}}. If invalid, return a minimal valid structure.", "store_as": "validated_data"},
            {"type": "Notify", "message": "Data re-fetched with explicit JSON Accept header and validated."}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: json_parse error: '{err}'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Tool\",\"tool_name\":\"process_data\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_rate_limit_scenarios(n):
    """Rate limited (429) -> add WaitFor + retry"""
    apis = ["GitHub API", "OpenAI API", "Twitter API", "Slack API", "Google API"]
    waits = [60, 30, 120, 45, 90]
    recipes = [
        ("Code review", "Fetch PR comments and summarize"),
        ("Content generation", "Generate and post content"),
        ("Social monitor", "Monitor social mentions"),
        ("Team updates", "Post status to Slack channels"),
        ("Drive sync", "Sync files with Google Drive"),
    ]
    out = []
    for i in range(n):
        rname, goal = recipes[i % len(recipes)]
        api = apis[i % len(apis)]
        wait = waits[i % len(waits)]
        completed = [{"step": 0, "tool": "prepare", "result": "request ready"}]
        replacement = jd([
            {"type": "WaitFor", "condition": {"type": "Duration", "seconds": wait}, "timeout_secs": wait + 30},
            {"type": "Tool", "tool_name": "web_fetch", "args": {"url": URLS[i % len(URLS)]},
             "store_as": "retry_result", "on_error": {"action": "Retry", "max": 2}},
            {"type": "Notify", "message": f"Retried after {wait}s rate limit cooldown from {api}."}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: web_fetch error: 'HTTP 429 Too Many Requests. {api} rate limit exceeded. Retry-After: {wait}'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Complete\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_disk_full_scenarios(n):
    """Disk full -> cleanup + retry"""
    sizes = ["2.1GB", "500MB", "1.3GB", "800MB", "3.5GB"]
    recipes = [
        ("Daily backup", "Compress and store daily backup"),
        ("Log rotation", "Rotate and compress logs"),
        ("Media download", "Download and organize media"),
        ("Build artifacts", "Store build outputs"),
        ("Database dump", "Export database snapshot"),
    ]
    out = []
    for i in range(n):
        rname, goal = recipes[i % len(recipes)]
        size = sizes[i % len(sizes)]
        completed = [{"step": 0, "tool": "prepare", "result": "ready to write"}]
        replacement = jd([
            {"type": "Tool", "tool_name": "disk_usage", "args": {"path": "/"}, "store_as": "disk_info", "on_error": {"action": "Fail"}},
            {"type": "Tool", "tool_name": "cleanup_temp", "args": {"paths": ["/tmp", "/var/cache"], "older_than_days": 7},
             "store_as": "cleanup_result", "on_error": {"action": "Skip"}},
            {"type": "Tool", "tool_name": "compress", "args": {"path": rand_path(), "output": f"/backup/{rand_file()}"}, "store_as": "write_result",
             "on_error": {"action": "Fail"}},
            {"type": "Notify", "message": f"Freed space via cleanup, then completed write. Original failure: disk full ({size} needed)."}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: compress error: 'No space left on device (need {size}, 0 bytes free)'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Backup stored\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_service_unavailable_scenarios(n):
    """Service unavailable -> check status + notify user"""
    services = [
        ("PostgreSQL", "database", "psql: could not connect to server: Connection refused (port 5432)"),
        ("Redis", "cache", "redis-cli: Could not connect to Redis at 127.0.0.1:6379: Connection refused"),
        ("Ollama", "LLM inference", "connect ECONNREFUSED 127.0.0.1:11434"),
        ("Docker", "container runtime", "Cannot connect to the Docker daemon at unix:///var/run/docker.sock"),
        ("Nginx", "web server", "connect() failed (111: Connection refused) to upstream"),
    ]
    recipes = [
        ("Data pipeline", "Query database and generate report"),
        ("Cache warmup", "Preload frequently accessed data"),
        ("AI summary", "Generate AI summary of documents"),
        ("Container deploy", "Deploy new container version"),
        ("Health check", "Verify all services are running"),
    ]
    out = []
    for i in range(n):
        svc_name, svc_type, err = services[i % len(services)]
        rname, goal = recipes[i % len(recipes)]
        completed = [{"step": 0, "tool": "prepare", "result": "configuration loaded"}]
        replacement = jd([
            {"type": "Tool", "tool_name": "service_status", "args": {"service": svc_name.lower()}, "store_as": "svc_status", "on_error": {"action": "Skip"}},
            {"type": "Notify", "message": f"{svc_name} ({svc_type}) is unavailable. Error: {err}. Recipe paused until service recovers."},
            {"type": "WaitFor", "condition": {"type": "Duration", "seconds": 300}, "timeout_secs": 600}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: {svc_name.lower()}_query error: '{err}'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Pipeline complete\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

def gen_auth_expired_scenarios(n):
    """Authentication expired -> refresh + retry"""
    auths = [
        ("OAuth token", "oauth_refresh", {"grant_type": "refresh_token", "token": "{{refresh_token}}"}),
        ("API key", "config_read", {"key": "api_key_backup"}),
        ("Session cookie", "web_login", {"url": "https://app.example.com/login", "credentials_key": "app_creds"}),
        ("SSH key", "ssh_agent_add", {"key_path": "/home/user/.ssh/id_ed25519"}),
        ("JWT token", "jwt_refresh", {"endpoint": "https://auth.example.com/token/refresh"}),
    ]
    recipes = [
        ("Cloud sync", "Sync files with cloud storage"),
        ("API monitor", "Monitor API health metrics"),
        ("Web scrape", "Scrape and store web data"),
        ("Deploy", "Deploy to remote server"),
        ("Auth check", "Verify auth tokens are valid"),
    ]
    out = []
    for i in range(n):
        auth_type, refresh_tool, refresh_args = auths[i % len(auths)]
        rname, goal = recipes[i % len(recipes)]
        orig_tool = random.choice(["web_fetch", "api_call", "cloud_upload", "ssh_exec"])
        completed = [{"step": 0, "tool": "prepare", "result": "target identified"}]
        replacement = jd([
            {"type": "Tool", "tool_name": refresh_tool, "args": refresh_args, "store_as": "new_auth", "on_error": {"action": "Fail"}},
            {"type": "Tool", "tool_name": orig_tool, "args": {"url": URLS[i % len(URLS)], "auth": "{{new_auth}}"},
             "store_as": "retry_result", "on_error": {"action": "Fail"}},
            {"type": "Notify", "message": f"Refreshed {auth_type} and retried successfully."}
        ])
        user = (f"Recipe: '{rname}'\nGoal: {goal}\n"
                f"Completed: {jd(completed)}\n"
                f"Failed step 1: {orig_tool} error: '{auth_type} expired: 401 Unauthorized'\n"
                f"Remaining: [{{\"step\":2,\"type\":\"Notify\",\"message\":\"Sync done\"}}]")
        out.append({"messages": [
            {"role": "system", "content": REPLAN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": replacement},
        ]})
    return out

# ── Retry/Skip/JumpTo recipe creation examples (60) ──

def gen_retry_skip_jumpto(n):
    """Show recipe creation with error handling baked in."""
    templates = [
        # (user_request, recipe_name, steps, response_text)
        ("Set up a backup that retries 3 times if it fails",
         "Resilient backup",
         [{"type": "Tool", "tool_name": "compress", "args": {"path": "/home/user/documents", "output": "/backup/docs.tar.gz"},
           "store_as": "backup", "on_error": {"action": "Retry", "max": 3}},
          {"type": "Notify", "message": "Backup complete: {{backup}}"}],
         "Created. The backup will retry up to 3 times on failure."),

        ("Make a recipe that checks disk space before downloading, skip the download if space is low",
         "Safe download",
         [{"type": "Tool", "tool_name": "disk_usage", "args": {"path": "/"}, "store_as": "disk", "on_error": {"action": "Fail"}},
          {"type": "JumpIf", "condition": {"op": "VarGt", "var": "disk.free_gb", "threshold": 5.0}, "target_step": 3},
          {"type": "Notify", "message": "Skipped download: less than 5GB free."},
          {"type": "Tool", "tool_name": "web_fetch", "args": {"url": "https://releases.example.com/latest.tar.gz"},
           "store_as": "download", "on_error": {"action": "Retry", "max": 2}},
          {"type": "Notify", "message": "Download complete."}],
         "Created. It checks disk space first and skips the download if under 5GB free."),

        ("Create a recipe to send a report, but if email fails, send it via notification instead",
         "Report delivery",
         [{"type": "Tool", "tool_name": "generate_report", "args": {"type": "daily"}, "store_as": "report", "on_error": {"action": "Fail"}},
          {"type": "Tool", "tool_name": "email_send", "args": {"to": "team@company.com", "subject": "Daily Report", "body": "{{report}}"},
           "store_as": "email_result", "on_error": {"action": "JumpTo", "step": 3}},
          {"type": "Notify", "message": "Report emailed to team."},
          {"type": "Tool", "tool_name": "send_notification", "args": {"title": "Daily Report", "body": "{{report}}"},
           "store_as": "notif", "on_error": {"action": "Fail"}},
          {"type": "Notify", "message": "Report sent via notification (email failed)."}],
         "Created. If email fails, it falls back to a desktop notification."),

        ("Build a recipe that monitors a service every 5 minutes, skip failures silently",
         "Service monitor",
         [{"type": "Tool", "tool_name": "web_fetch", "args": {"url": "https://api.myapp.com/health", "timeout_secs": 10},
           "store_as": "health", "on_error": {"action": "Skip"}},
          {"type": "JumpIf", "condition": {"op": "VarExists", "var": "health"}, "target_step": 3},
          {"type": "Tool", "tool_name": "send_notification", "args": {"title": "Service Down", "body": "myapp.com health check failed"},
           "store_as": "alert", "on_error": {"action": "Skip"}},
          {"type": "WaitFor", "condition": {"type": "Duration", "seconds": 300}}],
         "Created. Health checks run every 5 minutes, alerting on failures and skipping errors silently."),

        ("Recipe to clean old logs, retry cleanup twice, then replan if still failing",
         "Log cleanup",
         [{"type": "Tool", "tool_name": "cleanup_temp", "args": {"paths": ["/var/log/old"], "older_than_days": 30},
           "store_as": "cleaned", "on_error": {"action": "Retry", "max": 2}},
          {"type": "Tool", "tool_name": "compress", "args": {"path": "/var/log/archive", "output": "/backup/logs.tar.gz"},
           "store_as": "archive", "on_error": {"action": "Replan"}},
          {"type": "Notify", "message": "Logs cleaned and archived."}],
         "Created. Cleanup retries twice, and compression will replan on failure."),

        ("Create a deploy recipe: build first, if build fails just stop, then deploy with 2 retries",
         "Deploy pipeline",
         [{"type": "Tool", "tool_name": "run_command", "args": {"cmd": "cargo build --release"}, "store_as": "build", "on_error": {"action": "Fail"}},
          {"type": "Tool", "tool_name": "file_copy", "args": {"src": "/target/release/app", "dst": "/opt/app/bin/app"},
           "store_as": "deploy", "on_error": {"action": "Retry", "max": 2}},
          {"type": "Tool", "tool_name": "run_command", "args": {"cmd": "systemctl restart app"}, "store_as": "restart", "on_error": {"action": "Replan"}},
          {"type": "Notify", "message": "Deployed and restarted successfully."}],
         "Created. Build failure stops immediately. Deploy retries twice. Restart replans on failure."),

        ("Make a recipe that syncs 3 folders, skipping any that fail",
         "Multi-folder sync",
         [{"type": "Tool", "tool_name": "rsync", "args": {"src": "/home/user/docs", "dst": "/backup/docs"}, "store_as": "s1", "on_error": {"action": "Skip"}},
          {"type": "Tool", "tool_name": "rsync", "args": {"src": "/home/user/photos", "dst": "/backup/photos"}, "store_as": "s2", "on_error": {"action": "Skip"}},
          {"type": "Tool", "tool_name": "rsync", "args": {"src": "/home/user/music", "dst": "/backup/music"}, "store_as": "s3", "on_error": {"action": "Skip"}},
          {"type": "Notify", "message": "Sync complete. Check logs for any skipped folders."}],
         "Created. Each folder syncs independently. Failures are skipped so the others still run."),

        ("Set up a data pipeline: fetch data, transform it, load it. If transform fails, replan",
         "ETL pipeline",
         [{"type": "Tool", "tool_name": "web_fetch", "args": {"url": "https://data.source.com/export"}, "store_as": "raw_data", "on_error": {"action": "Retry", "max": 2}},
          {"type": "Tool", "tool_name": "run_command", "args": {"cmd": "python3 /scripts/transform.py"}, "store_as": "transformed", "on_error": {"action": "Replan"}},
          {"type": "Tool", "tool_name": "db_insert", "args": {"table": "analytics", "data": "{{transformed}}"}, "store_as": "loaded", "on_error": {"action": "Retry", "max": 3}},
          {"type": "Notify", "message": "ETL pipeline complete: {{loaded}}"}],
         "Created. Fetch retries twice, transform replans on failure, load retries three times."),

        ("Create a recipe: try to send via Telegram, if unavailable jump to email fallback",
         "Message relay",
         [{"type": "Tool", "tool_name": "telegram_send", "args": {"chat": "team_chat", "message": "{{update}}"},
           "store_as": "tg_result", "on_error": {"action": "JumpTo", "step": 2}},
          {"type": "Notify", "message": "Sent via Telegram."},
          {"type": "Tool", "tool_name": "email_send", "args": {"to": "team@company.com", "subject": "Update", "body": "{{update}}"},
           "store_as": "email_result", "on_error": {"action": "Fail"}},
          {"type": "Notify", "message": "Sent via email (Telegram was unavailable)."}],
         "Created. Tries Telegram first, falls back to email on failure."),

        ("Recipe to check for updates at midnight, retry network errors, skip if already up to date",
         "Auto updater",
         [{"type": "Tool", "tool_name": "web_fetch", "args": {"url": "https://releases.yantrikos.com/manifest.json"},
           "store_as": "manifest", "on_error": {"action": "Retry", "max": 3}},
          {"type": "Think", "prompt": "Compare {{manifest}} with current version. Is an update available?", "store_as": "update_check"},
          {"type": "JumpIf", "condition": {"op": "VarContains", "var": "update_check", "substring": "up to date"}, "target_step": 5},
          {"type": "Tool", "tool_name": "run_command", "args": {"cmd": "yantrik-upgrade apply"}, "store_as": "upgrade_result", "on_error": {"action": "Replan"}},
          {"type": "Notify", "message": "Updated to latest version: {{upgrade_result}}"},
          {"type": "Notify", "message": "Already up to date. No action needed."}],
         "Created. Checks for updates with 3 retries, skips if current, replans on upgrade failure."),
    ]
    out = []
    for i in range(n):
        t = templates[i % len(templates)]
        user_req, rname, steps, resp = t
        bond = BONDS[i % len(BONDS)]
        tc = {"id": cid(), "type": "function", "function": {"name": "create_recipe", "arguments": jd({"name": rname, "steps": steps})}}
        recipe_id = rid()
        out.append({"messages": [
            {"role": "system", "content": BOND_SYS[bond]},
            {"role": "user", "content": user_req},
            {"role": "assistant", "content": None, "tool_calls": [tc]},
            {"role": "tool", "tool_call_id": tc["id"], "content": jd({"recipe_id": recipe_id, "status": "ok"})},
            {"role": "assistant", "content": resp},
        ]})
    return out

# ── Failure learning examples (20) ──

def gen_failure_learning(n):
    learnings = [
        ("Daily backup", 2, "compress", "No space left on device",
         "added a cleanup step first",
         "compress tool can fail with disk space errors. Resolution: add disk_usage check and cleanup step before compression. Applies to any recipe writing large files."),
        ("Email digest", 1, "email_send", "SMTP authentication failed",
         "switched to desktop notification",
         "email_send can fail if SMTP credentials expire. Resolution: fall back to send_notification tool. Consider adding periodic credential validation."),
        ("API sync", 3, "web_fetch", "HTTP 429 Too Many Requests",
         "added a WaitFor delay before retrying",
         "web_fetch can hit rate limits on frequent API calls. Resolution: add WaitFor step with Retry-After duration before retry. Batch requests where possible."),
        ("File transfer", 1, "file_copy", "Permission denied: /etc/config",
         "copied to user home directory instead",
         "file_copy fails on protected system paths without root. Resolution: use user-writable alternative paths (/home/user, /tmp). Notify user if original path was critical."),
        ("Build pipeline", 2, "run_command", "exit code 137 (OOM killed)",
         "added memory check and reduced parallel jobs",
         "Build commands can be OOM-killed on constrained systems. Resolution: check available memory with system_info before builds, reduce parallelism with -j1 flag."),
        ("Web scrape", 1, "browser_navigate", "ERR_NAME_NOT_RESOLVED",
         "checked DNS and used IP fallback",
         "DNS resolution failures break web tools. Resolution: try alternative DNS or use IP address directly. Add connectivity check step before web operations."),
        ("Cron report", 3, "db_query", "connection pool exhausted",
         "added connection cleanup step before query",
         "Database connections can leak if recipes fail mid-execution. Resolution: add explicit connection cleanup before DB operations. Set connection timeouts."),
        ("Media encode", 1, "transcode", "Unsupported codec: hevc",
         "added codec detection step first",
         "transcode fails on unsupported codecs. Resolution: add a probe/detect step to check codec support before transcoding. Fall back to compatible codec."),
        ("Certificate renewal", 2, "acme_renew", "Rate limit: too many requests this week",
         "scheduled retry for next week",
         "ACME certificate renewal has weekly rate limits. Resolution: track renewal attempts and space them out. Add WaitFor with longer delay (days not seconds)."),
        ("Notification relay", 1, "telegram_send", "Bot token revoked",
         "fell back to email notification",
         "Telegram bot tokens can be revoked without warning. Resolution: fall back to alternative notification channels. Add periodic token validation step to maintenance recipes."),
        ("Log analysis", 2, "file_read", "File too large: 4.2GB exceeds 2GB limit",
         "added a split step to process in chunks",
         "file_read has a 2GB size limit. Resolution: use file_split or stream processing for large files. Add file_size check before read operations."),
        ("Container deploy", 3, "docker_run", "image not found: registry.local/app:v2.1",
         "rebuilt and pushed the image first",
         "docker_run fails if image is not in registry. Resolution: add docker_build and docker_push steps before docker_run. Verify image exists with docker_inspect."),
        ("SSH tunnel", 1, "ssh_connect", "Host key verification failed",
         "added known_hosts update step",
         "ssh_connect fails on unknown or changed host keys. Resolution: add ssh_keyscan step to update known_hosts before connecting. Warn user if key changed unexpectedly."),
        ("Package install", 2, "apt_install", "dpkg was interrupted, run dpkg --configure -a",
         "added dpkg repair step before install",
         "apt_install can fail if previous package operations were interrupted. Resolution: run dpkg --configure -a as a repair step before retrying install."),
        ("Database migration", 1, "db_migrate", "relation 'users' already exists",
         "added migration state check before running",
         "db_migrate can fail if migrations were partially applied. Resolution: check migration state table before running. Add idempotent checks (IF NOT EXISTS) to migration scripts."),
        ("Disk benchmark", 2, "fio_run", "Permission denied: cannot open /dev/sda for direct I/O",
         "switched to file-based benchmark in user directory",
         "fio_run needs device-level access for direct I/O. Resolution: fall back to file-based benchmarks in user-writable directories. Results are approximate but still useful."),
        ("Image resize", 1, "image_process", "Corrupt JPEG data: premature end of data segment",
         "added image validation step before processing",
         "image_process fails on corrupt files. Resolution: add image validation/integrity check before processing. Skip corrupt files and notify user of which files were skipped."),
        ("Git sync", 2, "git_pull", "error: Your local changes would be overwritten by merge",
         "added git stash before pull",
         "git_pull fails with uncommitted local changes. Resolution: add git_stash step before pull, then git_stash_pop after. Notify user if stash conflicts occur."),
        ("System update", 3, "reboot", "Reboot blocked: active user sessions detected",
         "scheduled reboot for 3 AM when idle",
         "reboot can be blocked by active sessions. Resolution: check active sessions first, schedule reboot for low-activity window using WaitFor with TimeAfter condition."),
        ("PDF generation", 1, "html_to_pdf", "wkhtmltopdf exited with code 1: QXcbConnection: could not connect to display",
         "added virtual framebuffer setup step",
         "html_to_pdf requires a display server. Resolution: add Xvfb (virtual framebuffer) setup step before PDF generation on headless systems. Or use a library that does not need X11."),
    ]
    out = []
    for i in range(n):
        l = learnings[i % len(learnings)]
        recipe_name, step_idx, tool, error, action, recorded = l
        user = (f"Recipe '{recipe_name}' step {step_idx} ({tool}) failed: '{error}'. "
                f"You replanned by {action}. Record this learning.")
        out.append({"messages": [
            {"role": "system", "content": LEARN_SYS},
            {"role": "user", "content": user},
            {"role": "assistant", "content": f"Recorded: {recorded}"},
        ]})
    return out

# ── Main ──

def main():
    all_examples = []

    # Replan: 120 total (15+15+15+15+15+10+10+10+10+5)
    all_examples.extend(gen_smtp_scenarios(15))
    all_examples.extend(gen_http_scenarios(15))
    all_examples.extend(gen_permission_scenarios(15))
    all_examples.extend(gen_tool_unavailable_scenarios(15))
    all_examples.extend(gen_timeout_scenarios(15))
    all_examples.extend(gen_invalid_json_scenarios(10))
    all_examples.extend(gen_rate_limit_scenarios(10))
    all_examples.extend(gen_disk_full_scenarios(10))
    all_examples.extend(gen_service_unavailable_scenarios(10))
    all_examples.extend(gen_auth_expired_scenarios(5))

    # Retry/Skip/JumpTo: 60
    all_examples.extend(gen_retry_skip_jumpto(60))

    # Failure learning: 20
    all_examples.extend(gen_failure_learning(20))

    random.shuffle(all_examples)

    with open(OUT, "w", encoding="utf-8") as f:
        for ex in all_examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    print(f"Wrote {len(all_examples)} examples to {OUT}")

if __name__ == "__main__":
    main()
