#!/usr/bin/env python3
"""
Generate synthetic training data for BROWSE and DELEGATE tool families.
Outputs:
  - batch_tools_05_browse.jsonl   (200 examples)
  - batch_tools_06_delegate.jsonl (150 examples)
"""

import json
import random
from pathlib import Path

random.seed(73)

OUT_DIR = Path(__file__).parent

# ---------------------------------------------------------------------------
# System prompts per bond stage
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

BOND_STAGES = list(BOND_PROMPTS.keys())

# ---------------------------------------------------------------------------
# Helper data
# ---------------------------------------------------------------------------
PEOPLE = [
    "Sarah", "Marcus", "Priya", "James", "Elena", "David", "Aisha", "Tom",
    "Kenji", "Olivia", "Carlos", "Nina", "Raj", "Megan", "Leo", "Fatima",
    "Chen Wei", "Ananya", "Derek", "Sofia",
]
WEBSITES = [
    "github.com", "stackoverflow.com", "news.ycombinator.com", "reddit.com",
    "amazon.com", "google.com", "wikipedia.org", "docs.python.org",
    "developer.mozilla.org", "crates.io", "npmjs.com", "gitlab.com",
    "medium.com", "dev.to", "arxiv.org", "youtube.com", "twitter.com",
    "linkedin.com", "notion.so", "figma.com",
]
SEARCH_QUERIES = [
    "rust async tutorial", "best mechanical keyboard 2026", "flights to Tokyo",
    "python dataclass vs attrs", "how to set up nginx reverse proxy",
    "latest AI research papers", "weather in San Francisco", "recipe for pad thai",
    "rust trait objects vs generics", "kubernetes pod scheduling",
    "best budget monitor for programming", "how to fix git merge conflicts",
    "react vs svelte comparison", "docker compose networking",
    "stock price AAPL", "nearest coffee shop", "train schedule NYC to Boston",
    "how to write a cover letter", "vim cheatsheet", "linux kernel compilation",
]
GITHUB_REPOS = [
    "torvalds/linux", "rust-lang/rust", "facebook/react", "microsoft/vscode",
    "denoland/deno", "vercel/next.js", "golang/go", "kubernetes/kubernetes",
    "apache/spark", "tensorflow/tensorflow",
]
FORM_FIELDS = {
    "login": [("username", "myuser"), ("password", "********")],
    "signup": [("email", "user@example.com"), ("password", "********"), ("confirm_password", "********")],
    "search": [("q", "search query")],
    "contact": [("name", "John Doe"), ("email", "john@example.com"), ("message", "Hello, I have a question about your API.")],
    "checkout": [("card_number", "****-****-****-1234"), ("expiry", "12/27"), ("cvv", "***")],
}
PAGE_TITLES = [
    "GitHub - Dashboard", "Stack Overflow - Questions", "Hacker News",
    "Reddit - Front Page", "Amazon.com Shopping", "Google Search",
    "Wikipedia", "Python Documentation", "MDN Web Docs", "crates.io",
]
ELEMENT_IDS = [
    "btn-submit", "btn-login", "btn-search", "nav-home", "nav-settings",
    "link-profile", "link-notifications", "tab-issues", "tab-prs",
    "dropdown-sort", "checkbox-agree", "input-search", "btn-next",
    "btn-previous", "link-read-more", "btn-download", "btn-add-to-cart",
    "link-sign-out", "btn-approve", "btn-reject",
]
JS_SNIPPETS = [
    "document.title",
    "document.querySelectorAll('a').length",
    "window.scrollY",
    "document.querySelector('.price').textContent",
    "localStorage.getItem('theme')",
    "document.cookie",
    "window.location.href",
    "document.querySelectorAll('tr').length",
]
TASK_DESCRIPTIONS = [
    "Research competitors pricing pages", "Compile a list of open issues in the repo",
    "Monitor server status dashboard every hour", "Scrape product reviews from Amazon",
    "Check CI/CD pipeline status", "Download all PDFs from the documentation site",
    "Update the project wiki with release notes", "Generate a report from analytics dashboard",
    "Cross-reference bug reports with changelog", "Test the signup flow on staging",
]
RECIPE_NAMES = [
    "daily-standup-prep", "weekly-report", "deploy-and-verify", "backup-databases",
    "morning-briefing", "pr-review-workflow", "invoice-collection", "server-health-check",
    "content-publishing", "data-pipeline-run", "security-scan", "dependency-update",
]
SCRIPT_NAMES = [
    "cleanup_logs.sh", "backup_db.py", "deploy.sh", "sync_data.py",
    "generate_report.py", "test_api.sh", "migrate_db.py", "check_health.sh",
    "rotate_keys.py", "compress_images.sh", "parse_csv.py", "fetch_metrics.py",
]
CRON_EXPRESSIONS = [
    "0 9 * * 1-5", "0 0 * * *", "*/30 * * * *", "0 */2 * * *",
    "0 8 * * 1", "0 18 * * 5", "0 6 1 * *", "0 22 * * *",
]
AGENT_NAMES = [
    "research-agent", "code-review-agent", "data-agent", "monitor-agent",
    "deploy-agent", "test-agent", "analysis-agent", "scraper-agent",
]
CODE_LANGUAGES = ["python", "bash", "rust", "javascript", "sql"]

_call_counter = 0
def next_call_id():
    global _call_counter
    _call_counter += 1
    return f"call_{_call_counter}"

def rand_bond():
    return random.choice(BOND_STAGES)

def rand_id(prefix="", length=6):
    return prefix + "".join(random.choices("abcdefghijklmnopqrstuvwxyz0123456789", k=length))

def make_tool_call(name, arguments):
    return {
        "id": next_call_id(),
        "type": "function",
        "function": {
            "name": name,
            "arguments": json.dumps(arguments, ensure_ascii=False),
        },
    }

def make_assistant_tool(content, tool_calls):
    return {"role": "assistant", "content": content, "tool_calls": tool_calls}

def make_tool_result(call_id, result):
    return {"role": "tool", "tool_call_id": call_id, "content": json.dumps(result, ensure_ascii=False)}

def make_example(bond, messages):
    return json.dumps({"messages": [
        {"role": "system", "content": BOND_PROMPTS[bond]},
        *messages,
    ]}, ensure_ascii=False)


# ===================================================================
# BROWSE FAMILY (200 examples)
# ===================================================================

def gen_browse_examples():
    examples = []

    # --- 1. launch_browser + browser_see (25 examples) ---
    for i in range(25):
        bond = BOND_STAGES[i % 4]
        site = random.choice(WEBSITES)
        url = f"https://{site}"
        title = random.choice(PAGE_TITLES)
        elements = random.sample([
            "Navigation menu", "Search bar", "Login button", "Main content area",
            "Sidebar with links", "Footer", "Notification bell", "User avatar",
            "Settings gear icon", "Create new button",
        ], k=random.randint(3, 6))

        user_msgs = [
            f"Open {site} for me",
            f"Go to {url}",
            f"Launch the browser and navigate to {site}",
            f"Pull up {site}",
            f"Can you open {site}?",
        ]
        tc1 = make_tool_call("launch_browser", {"url": url})
        tc2 = make_tool_call("browser_see", {})

        replies = [
            f"I've opened {site}. The page shows: {', '.join(elements)}.",
            f"{site} is loaded. I can see: {', '.join(elements)}.",
            f"Browser is open to {site}. Visible elements: {', '.join(elements)}.",
        ]
        msgs = [
            {"role": "user", "content": user_msgs[i % len(user_msgs)]},
            make_assistant_tool(None, [tc1]),
            make_tool_result(tc1["id"], {"status": "ok", "page_title": title, "url": url}),
            make_assistant_tool(None, [tc2]),
            make_tool_result(tc2["id"], {"elements": elements}),
            {"role": "assistant", "content": random.choice(replies)},
        ]
        examples.append(make_example(bond, msgs))

    # --- 2. browse (direct URL) (15 examples) ---
    for i in range(15):
        bond = BOND_STAGES[i % 4]
        site = random.choice(WEBSITES)
        path = random.choice(["", "/docs", "/about", "/pricing", "/blog", "/api", "/status"])
        url = f"https://{site}{path}"
        title = f"{site.split('.')[0].title()} - {path.lstrip('/').title() or 'Home'}"
        snippet = random.choice([
            "Welcome to our platform. Get started by creating an account.",
            "API Documentation v3.2. Authentication, endpoints, rate limits.",
            "Pricing: Free tier, Pro $12/mo, Enterprise contact us.",
            "Latest blog posts about technology and innovation.",
            "System status: All services operational.",
        ])
        tc = make_tool_call("browse", {"url": url})
        user_msgs = [
            f"What's on {url}?",
            f"Check {url} for me",
            f"Browse to {url} and tell me what you see",
            f"Navigate to {url}",
        ]
        msgs = [
            {"role": "user", "content": user_msgs[i % len(user_msgs)]},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "title": title, "url": url, "text_content": snippet}),
            {"role": "assistant", "content": f"The page at {url} shows: {snippet}"},
        ]
        examples.append(make_example(bond, msgs))

    # --- 3. browser_search (15 examples) ---
    for i in range(15):
        bond = BOND_STAGES[i % 4]
        query = random.choice(SEARCH_QUERIES)
        results = [
            {"title": f"Result {j+1}: {query}", "url": f"https://example.com/result{j+1}", "snippet": f"Comprehensive guide to {query}..."}
            for j in range(random.randint(3, 5))
        ]
        tc = make_tool_call("browser_search", {"query": query})
        user_msgs = [
            f"Search for {query}",
            f"Look up {query} online",
            f"Find information about {query}",
            f"Google {query} for me",
        ]
        result_summary = "\n".join(f"- {r['title']}: {r['snippet']}" for r in results[:3])
        msgs = [
            {"role": "user", "content": user_msgs[i % len(user_msgs)]},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "results": results}),
            {"role": "assistant", "content": f"Top results for \"{query}\":\n{result_summary}"},
        ]
        examples.append(make_example(bond, msgs))

    # --- 4. browser_click_element (10 examples) ---
    for i in range(10):
        bond = BOND_STAGES[i % 4]
        elem = random.choice(ELEMENT_IDS)
        label = elem.replace("btn-", "").replace("link-", "").replace("tab-", "").replace("-", " ").title()
        tc = make_tool_call("browser_click_element", {"selector": f"#{elem}"})
        user_msgs = [
            f"Click the {label} button",
            f"Click on {label}",
            f"Press the {label} element",
            f"Hit {label}",
        ]
        msgs = [
            {"role": "user", "content": user_msgs[i % len(user_msgs)]},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "action": "clicked", "element": elem}),
            {"role": "assistant", "content": f"Clicked {label}. The page has updated."},
        ]
        examples.append(make_example(bond, msgs))

    # --- 5. browser_type_element (10 examples) ---
    for i in range(10):
        bond = BOND_STAGES[i % 4]
        field_name = random.choice(["search", "email", "username", "query", "message"])
        text = random.choice(["hello world", "rust async await", "user@example.com", "myusername", "test query"])
        tc = make_tool_call("browser_type_element", {"selector": f"#input-{field_name}", "text": text})
        user_msgs = [
            f"Type \"{text}\" in the {field_name} field",
            f"Enter \"{text}\" into {field_name}",
            f"Fill in the {field_name} with \"{text}\"",
        ]
        msgs = [
            {"role": "user", "content": user_msgs[i % len(user_msgs)]},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "typed": text, "element": f"input-{field_name}"}),
            {"role": "assistant", "content": f"Typed \"{text}\" into the {field_name} field."},
        ]
        examples.append(make_example(bond, msgs))

    # --- 6. browser_snapshot (8 examples) ---
    for i in range(8):
        bond = BOND_STAGES[i % 4]
        site = random.choice(WEBSITES)
        tc = make_tool_call("browser_snapshot", {})
        snapshot_data = {
            "url": f"https://{site}",
            "title": f"{site.split('.')[0].title()} Page",
            "dom_summary": {"total_elements": random.randint(50, 300), "interactive": random.randint(5, 30), "forms": random.randint(0, 3)},
        }
        msgs = [
            {"role": "user", "content": random.choice(["Take a snapshot of the current page", "Snapshot the browser state", "Capture the current page DOM", "What does the page DOM look like?"])},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], snapshot_data),
            {"role": "assistant", "content": f"The page at {site} has {snapshot_data['dom_summary']['total_elements']} elements, {snapshot_data['dom_summary']['interactive']} interactive elements, and {snapshot_data['dom_summary']['forms']} forms."},
        ]
        examples.append(make_example(bond, msgs))

    # --- 7. browser_scroll (8 examples) ---
    for i in range(8):
        bond = BOND_STAGES[i % 4]
        direction = random.choice(["down", "up"])
        amount = random.choice([300, 500, 1000, "bottom", "top"])
        tc = make_tool_call("browser_scroll", {"direction": direction, "amount": amount})
        msgs = [
            {"role": "user", "content": random.choice([
                f"Scroll {direction}", f"Scroll {direction} on the page",
                f"Go {'to the bottom' if amount == 'bottom' else 'further ' + direction}",
                f"Page {direction}",
            ])},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "scroll_position": random.randint(0, 5000)}),
            {"role": "assistant", "content": f"Scrolled {direction}. {'Reached the ' + str(amount) + '.' if amount in ('bottom', 'top') else 'Page moved ' + direction + '.'}"},
        ]
        examples.append(make_example(bond, msgs))

    # --- 8. browser_back / browser_forward (8 examples) ---
    for i in range(8):
        bond = BOND_STAGES[i % 4]
        action = "browser_back" if i < 4 else "browser_forward"
        direction_word = "back" if action == "browser_back" else "forward"
        new_url = f"https://{random.choice(WEBSITES)}"
        tc = make_tool_call(action, {})
        msgs = [
            {"role": "user", "content": random.choice([
                f"Go {direction_word}", f"Navigate {direction_word}",
                f"Hit the {direction_word} button", f"Take me {direction_word}",
            ])},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "url": new_url}),
            {"role": "assistant", "content": f"Navigated {direction_word}. Now on {new_url}."},
        ]
        examples.append(make_example(bond, msgs))

    # --- 9. browser_tabs / browser_new_tab / browser_close_tab / browser_switch_tab (12 examples) ---
    for i in range(12):
        bond = BOND_STAGES[i % 4]
        if i < 3:
            # list tabs
            tc = make_tool_call("browser_tabs", {})
            tabs = [{"id": j, "title": f"Tab {j}: {random.choice(WEBSITES)}", "active": j == 0} for j in range(random.randint(2, 5))]
            tab_list = "\n".join(f"- {'[active] ' if t['active'] else ''}{t['title']}" for t in tabs)
            msgs = [
                {"role": "user", "content": random.choice(["What tabs are open?", "List my browser tabs", "Show open tabs"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"tabs": tabs}),
                {"role": "assistant", "content": f"You have {len(tabs)} tabs open:\n{tab_list}"},
            ]
        elif i < 6:
            # new tab
            url = f"https://{random.choice(WEBSITES)}"
            tc = make_tool_call("browser_new_tab", {"url": url})
            msgs = [
                {"role": "user", "content": random.choice([f"Open a new tab to {url}", f"New tab with {url}", f"Open {url} in a new tab"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "tab_id": random.randint(1, 10), "url": url}),
                {"role": "assistant", "content": f"Opened a new tab to {url}."},
            ]
        elif i < 9:
            # close tab
            tab_id = random.randint(0, 5)
            tc = make_tool_call("browser_close_tab", {"tab_id": tab_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Close tab {tab_id}", "Close this tab", "Close the current tab"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "closed_tab_id": tab_id}),
                {"role": "assistant", "content": f"Tab {tab_id} closed."},
            ]
        else:
            # switch tab
            tab_id = random.randint(0, 5)
            tc = make_tool_call("browser_switch_tab", {"tab_id": tab_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Switch to tab {tab_id}", f"Go to tab {tab_id}", "Switch to the other tab"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "active_tab_id": tab_id, "url": f"https://{random.choice(WEBSITES)}"}),
                {"role": "assistant", "content": f"Switched to tab {tab_id}."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 10. browser_cookies / browser_history / browser_bookmarks (9 examples) ---
    for i in range(9):
        bond = BOND_STAGES[i % 4]
        if i < 3:
            tc = make_tool_call("browser_cookies", {"domain": random.choice(WEBSITES)})
            cookies = [{"name": f"cookie_{j}", "value": rand_id(length=12), "domain": random.choice(WEBSITES)} for j in range(random.randint(2, 5))]
            msgs = [
                {"role": "user", "content": random.choice(["Show cookies for this site", "What cookies are stored?", "List the browser cookies"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"cookies": cookies}),
                {"role": "assistant", "content": f"Found {len(cookies)} cookies: {', '.join(c['name'] for c in cookies)}."},
            ]
        elif i < 6:
            tc = make_tool_call("browser_history", {"limit": 10})
            history = [{"url": f"https://{random.choice(WEBSITES)}", "title": random.choice(PAGE_TITLES), "visited_at": f"2026-03-{random.randint(1,13):02d}T{random.randint(8,22):02d}:00"} for _ in range(5)]
            history_text = "\n".join(f"- {h['title']} ({h['visited_at']})" for h in history)
            msgs = [
                {"role": "user", "content": random.choice(["Show my browser history", "What sites did I visit recently?", "Browser history please"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"history": history}),
                {"role": "assistant", "content": f"Recent browsing history:\n{history_text}"},
            ]
        else:
            tc = make_tool_call("browser_bookmarks", {})
            bookmarks = [{"title": f"Bookmark: {random.choice(WEBSITES)}", "url": f"https://{random.choice(WEBSITES)}"} for _ in range(random.randint(3, 7))]
            bm_text = "\n".join(f"- {b['title']}" for b in bookmarks)
            msgs = [
                {"role": "user", "content": random.choice(["Show my bookmarks", "List bookmarks", "What's in my bookmarks?"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"bookmarks": bookmarks}),
                {"role": "assistant", "content": f"Your bookmarks:\n{bm_text}"},
            ]
        examples.append(make_example(bond, msgs))

    # --- 11. browser_download (6 examples) ---
    for i in range(6):
        bond = BOND_STAGES[i % 4]
        url = f"https://{random.choice(WEBSITES)}/files/{random.choice(['report.pdf', 'data.csv', 'archive.zip', 'image.png', 'manual.pdf', 'release.tar.gz'])}"
        filename = url.split("/")[-1]
        tc = make_tool_call("browser_download", {"url": url})
        msgs = [
            {"role": "user", "content": random.choice([f"Download {url}", f"Download the file at {url}", f"Grab {filename} from that page"])},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "filename": filename, "size_bytes": random.randint(10000, 50000000), "path": f"/home/yantrik/Downloads/{filename}"}),
            {"role": "assistant", "content": f"Downloaded {filename} to ~/Downloads."},
        ]
        examples.append(make_example(bond, msgs))

    # --- 12. browser_screenshot / browser_pdf (8 examples) ---
    for i in range(8):
        bond = BOND_STAGES[i % 4]
        if i < 4:
            tc = make_tool_call("browser_screenshot", {"path": f"/tmp/screenshot_{i}.png"})
            msgs = [
                {"role": "user", "content": random.choice(["Take a screenshot of the page", "Screenshot this page", "Capture the screen", "Save a screenshot"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "path": f"/tmp/screenshot_{i}.png", "size_bytes": random.randint(50000, 500000)}),
                {"role": "assistant", "content": f"Screenshot saved to /tmp/screenshot_{i}.png."},
            ]
        else:
            tc = make_tool_call("browser_pdf", {"path": f"/tmp/page_{i}.pdf"})
            msgs = [
                {"role": "user", "content": random.choice(["Save this page as PDF", "Export page to PDF", "Print to PDF", "Convert this page to PDF"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "path": f"/tmp/page_{i}.pdf", "pages": random.randint(1, 10)}),
                {"role": "assistant", "content": f"Page saved as PDF to /tmp/page_{i}.pdf ({random.randint(1,10)} pages)."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 13. browser_execute_js (8 examples) ---
    for i in range(8):
        bond = BOND_STAGES[i % 4]
        js = random.choice(JS_SNIPPETS)
        result_val = random.choice(["My Page Title", "42", "1250", "$29.99", "dark", "session=abc123", "https://example.com", "15"])
        tc = make_tool_call("browser_execute_js", {"script": js})
        msgs = [
            {"role": "user", "content": random.choice([
                f"Run this JavaScript: {js}", f"Execute JS: {js}",
                f"What does {js} return?", f"Run {js} in the browser console",
            ])},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "result": result_val}),
            {"role": "assistant", "content": f"Result: {result_val}"},
        ]
        examples.append(make_example(bond, msgs))

    # --- 14. browser_wait_for (6 examples) ---
    for i in range(6):
        bond = BOND_STAGES[i % 4]
        selector = random.choice(["#loading-spinner", ".results-container", "#modal-dialog", ".data-table", "#success-message", ".chart-rendered"])
        timeout = random.choice([5000, 10000, 15000, 30000])
        tc = make_tool_call("browser_wait_for", {"selector": selector, "timeout_ms": timeout})
        msgs = [
            {"role": "user", "content": random.choice([
                f"Wait for {selector} to appear", f"Wait until the {selector.lstrip('#.')} element loads",
                f"Hold on until {selector} shows up",
            ])},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "found": True, "elapsed_ms": random.randint(200, timeout - 500)}),
            {"role": "assistant", "content": f"Element {selector} appeared on the page."},
        ]
        examples.append(make_example(bond, msgs))

    # --- 15. browser_fill_form (6 examples) ---
    for i in range(6):
        bond = BOND_STAGES[i % 4]
        form_type = random.choice(list(FORM_FIELDS.keys()))
        fields = FORM_FIELDS[form_type]
        form_data = {f: v for f, v in fields}
        tc = make_tool_call("browser_fill_form", {"form_selector": f"#form-{form_type}", "fields": form_data})
        field_names = ", ".join(f[0] for f in fields)
        msgs = [
            {"role": "user", "content": random.choice([
                f"Fill in the {form_type} form", f"Complete the {form_type} form for me",
                f"Auto-fill the {form_type} form",
            ])},
            make_assistant_tool(None, [tc]),
            make_tool_result(tc["id"], {"status": "ok", "fields_filled": len(fields), "form": form_type}),
            {"role": "assistant", "content": f"Filled the {form_type} form ({field_names})."},
        ]
        examples.append(make_example(bond, msgs))

    # --- 16. browser_cleanup / browser_status (6 examples) ---
    for i in range(6):
        bond = BOND_STAGES[i % 4]
        if i < 3:
            tc = make_tool_call("browser_cleanup", {})
            msgs = [
                {"role": "user", "content": random.choice(["Close the browser", "Clean up the browser session", "Shut down the browser"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "tabs_closed": random.randint(1, 5), "session_cleared": True}),
                {"role": "assistant", "content": "Browser session closed and cleaned up."},
            ]
        else:
            tc = make_tool_call("browser_status", {})
            tabs = random.randint(1, 8)
            msgs = [
                {"role": "user", "content": random.choice(["Is the browser running?", "Browser status", "What's the browser state?"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "running", "tabs_open": tabs, "memory_mb": random.randint(200, 2000), "uptime_seconds": random.randint(60, 36000)}),
                {"role": "assistant", "content": f"Browser is running with {tabs} tab{'s' if tabs > 1 else ''} open, using {random.randint(200,2000)} MB of memory."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 17. Multi-tool browser workflows (50 examples = 25% of 200) ---
    for i in range(50):
        bond = BOND_STAGES[i % 4]
        workflow_type = i % 10

        if workflow_type == 0:
            # Search and open first result
            query = random.choice(SEARCH_QUERIES)
            result_url = f"https://{random.choice(WEBSITES)}/article/{rand_id()}"
            tc1 = make_tool_call("browser_search", {"query": query})
            tc2 = make_tool_call("browse", {"url": result_url})
            tc3 = make_tool_call("browser_see", {})
            content_text = f"Detailed article about {query}. Key points: setup, configuration, best practices."
            msgs = [
                {"role": "user", "content": f"Search for {query} and open the first result"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "results": [{"title": f"Guide: {query}", "url": result_url, "snippet": f"Complete guide to {query}..."}]}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "title": f"Guide: {query}", "url": result_url}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"elements": [f"Article: {query}", "Table of contents", "Code examples", "Related links"]}),
                {"role": "assistant", "content": f"Opened the top result for \"{query}\". The article covers setup, configuration, and best practices with code examples."},
            ]
        elif workflow_type == 1:
            # Navigate to GitHub repo and check issues
            repo = random.choice(GITHUB_REPOS)
            tc1 = make_tool_call("launch_browser", {"url": f"https://github.com/{repo}/issues"})
            tc2 = make_tool_call("browser_see", {})
            issues = [f"#{random.randint(100,9999)}: {random.choice(['Bug:', 'Feature:', 'Fix:'])} {rand_id('issue-', 8)}" for _ in range(random.randint(3, 6))]
            tc3 = make_tool_call("browser_click_element", {"selector": "#tab-issues"})
            msgs = [
                {"role": "user", "content": f"Check the open issues on {repo}"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": f"Issues - {repo}", "url": f"https://github.com/{repo}/issues"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"elements": issues}),
                {"role": "assistant", "content": f"Open issues on {repo}:\n" + "\n".join(f"- {issue}" for issue in issues)},
            ]
        elif workflow_type == 2:
            # Fill form and submit
            form_type = random.choice(["login", "contact", "signup"])
            fields = FORM_FIELDS[form_type]
            form_data = {f: v for f, v in fields}
            site = random.choice(WEBSITES)
            tc1 = make_tool_call("launch_browser", {"url": f"https://{site}/{form_type}"})
            tc2 = make_tool_call("browser_fill_form", {"form_selector": f"#form-{form_type}", "fields": form_data})
            tc3 = make_tool_call("browser_click_element", {"selector": "#btn-submit"})
            msgs = [
                {"role": "user", "content": f"Go to {site} and fill in the {form_type} form, then submit it"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": f"{form_type.title()} - {site}", "url": f"https://{site}/{form_type}"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "fields_filled": len(fields), "form": form_type}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "action": "clicked", "element": "btn-submit"}),
                {"role": "assistant", "content": f"Filled and submitted the {form_type} form on {site}."},
            ]
        elif workflow_type == 3:
            # Browse, scroll, screenshot
            site = random.choice(WEBSITES)
            tc1 = make_tool_call("launch_browser", {"url": f"https://{site}"})
            tc2 = make_tool_call("browser_scroll", {"direction": "down", "amount": 1000})
            tc3 = make_tool_call("browser_screenshot", {"path": f"/tmp/capture_{i}.png"})
            msgs = [
                {"role": "user", "content": f"Open {site}, scroll down, and take a screenshot"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": f"{site}", "url": f"https://{site}"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "scroll_position": 1000}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "path": f"/tmp/capture_{i}.png", "size_bytes": random.randint(50000, 300000)}),
                {"role": "assistant", "content": f"Opened {site}, scrolled down, and saved a screenshot to /tmp/capture_{i}.png."},
            ]
        elif workflow_type == 4:
            # Multi-tab workflow
            sites = random.sample(WEBSITES, 3)
            tc1 = make_tool_call("launch_browser", {"url": f"https://{sites[0]}"})
            tc2 = make_tool_call("browser_new_tab", {"url": f"https://{sites[1]}"})
            tc3 = make_tool_call("browser_new_tab", {"url": f"https://{sites[2]}"})
            tc4 = make_tool_call("browser_tabs", {})
            tabs = [{"id": j, "title": sites[j], "active": j == 2} for j in range(3)]
            msgs = [
                {"role": "user", "content": f"Open {sites[0]}, {sites[1]}, and {sites[2]} in separate tabs"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": sites[0], "url": f"https://{sites[0]}"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "tab_id": 1, "url": f"https://{sites[1]}"}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "tab_id": 2, "url": f"https://{sites[2]}"}),
                make_assistant_tool(None, [tc4]),
                make_tool_result(tc4["id"], {"tabs": tabs}),
                {"role": "assistant", "content": f"All three sites are open in separate tabs: {', '.join(sites)}."},
            ]
        elif workflow_type == 5:
            # Search, navigate, execute JS to extract data
            query = random.choice(SEARCH_QUERIES)
            target_url = f"https://{random.choice(WEBSITES)}/page/{rand_id()}"
            js_code = random.choice(JS_SNIPPETS)
            js_result = random.choice(["$149.99", "42 items", "3.5 stars", "Last updated: 2026-03-10"])
            tc1 = make_tool_call("browser_search", {"query": query})
            tc2 = make_tool_call("browse", {"url": target_url})
            tc3 = make_tool_call("browser_execute_js", {"script": js_code})
            msgs = [
                {"role": "user", "content": f"Search for {query}, go to the first result, and extract the main data"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "results": [{"title": query, "url": target_url, "snippet": "..."}]}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "title": query, "url": target_url}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "result": js_result}),
                {"role": "assistant", "content": f"Found the page for \"{query}\". Extracted data: {js_result}."},
            ]
        elif workflow_type == 6:
            # Navigate, wait for element, then interact
            site = random.choice(WEBSITES)
            selector = random.choice([".data-table", "#results-container", ".chart-rendered"])
            tc1 = make_tool_call("launch_browser", {"url": f"https://{site}/dashboard"})
            tc2 = make_tool_call("browser_wait_for", {"selector": selector, "timeout_ms": 10000})
            tc3 = make_tool_call("browser_see", {})
            elements = ["Dashboard header", "Data table with 25 rows", "Export button", "Filter dropdown", "Date range picker"]
            msgs = [
                {"role": "user", "content": f"Open {site} dashboard and wait for the data to load, then tell me what's there"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": f"{site} Dashboard", "url": f"https://{site}/dashboard"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "found": True, "elapsed_ms": random.randint(500, 3000)}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"elements": elements}),
                {"role": "assistant", "content": f"Dashboard loaded. I can see: {', '.join(elements)}."},
            ]
        elif workflow_type == 7:
            # Download file workflow
            site = random.choice(WEBSITES)
            filename = random.choice(["report.pdf", "dataset.csv", "archive.zip", "specs.docx"])
            download_url = f"https://{site}/downloads/{filename}"
            tc1 = make_tool_call("launch_browser", {"url": f"https://{site}/downloads"})
            tc2 = make_tool_call("browser_see", {})
            tc3 = make_tool_call("browser_download", {"url": download_url})
            msgs = [
                {"role": "user", "content": f"Go to {site}'s downloads page and download {filename}"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": f"Downloads - {site}", "url": f"https://{site}/downloads"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"elements": [f"Download: {filename}", "Download: other_file.zip", "Download: readme.txt"]}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "filename": filename, "size_bytes": random.randint(10000, 10000000), "path": f"/home/yantrik/Downloads/{filename}"}),
                {"role": "assistant", "content": f"Downloaded {filename} to ~/Downloads."},
            ]
        elif workflow_type == 8:
            # Navigate, type in search, click result
            site = random.choice(WEBSITES)
            search_term = random.choice(SEARCH_QUERIES)
            tc1 = make_tool_call("launch_browser", {"url": f"https://{site}"})
            tc2 = make_tool_call("browser_type_element", {"selector": "#input-search", "text": search_term})
            tc3 = make_tool_call("browser_click_element", {"selector": "#btn-search"})
            tc4 = make_tool_call("browser_see", {})
            results = [f"Result: {search_term} - page {j+1}" for j in range(3)]
            msgs = [
                {"role": "user", "content": f"Go to {site}, search for \"{search_term}\" and show me the results"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": site, "url": f"https://{site}"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "typed": search_term, "element": "input-search"}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "action": "clicked", "element": "btn-search"}),
                make_assistant_tool(None, [tc4]),
                make_tool_result(tc4["id"], {"elements": results}),
                {"role": "assistant", "content": f"Search results on {site} for \"{search_term}\":\n" + "\n".join(f"- {r}" for r in results)},
            ]
        else:  # workflow_type == 9
            # Save page as PDF, then cleanup
            site = random.choice(WEBSITES)
            path = random.choice(["/docs", "/api", "/guide", "/reference"])
            tc1 = make_tool_call("launch_browser", {"url": f"https://{site}{path}"})
            tc2 = make_tool_call("browser_pdf", {"path": f"/tmp/{site.replace('.','_')}.pdf"})
            tc3 = make_tool_call("browser_cleanup", {})
            msgs = [
                {"role": "user", "content": f"Open {site}{path}, save it as PDF, then close the browser"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "page_title": f"{site} - {path.lstrip('/')}", "url": f"https://{site}{path}"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "path": f"/tmp/{site.replace('.','_')}.pdf", "pages": random.randint(1, 15)}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "tabs_closed": 1, "session_cleared": True}),
                {"role": "assistant", "content": f"Saved {site}{path} as PDF and closed the browser."},
            ]

        examples.append(make_example(bond, msgs))

    # --- 18. Error handling (18 examples = ~9%) ---
    error_scenarios = [
        # Page not found
        lambda b: _browse_error(b, "page_not_found", "https://example.com/nonexistent", "Page not found (404)", "The page doesn't exist. The server returned a 404 error."),
        # Timeout
        lambda b: _browse_error(b, "timeout", "https://slow-site.com", "Connection timed out after 30s", "The site didn't respond within 30 seconds. It may be down or experiencing issues."),
        # Element not found
        lambda b: _browse_error_element(b),
        # SSL error
        lambda b: _browse_error(b, "ssl_error", "https://expired-cert.example.com", "SSL certificate has expired", "The site has an expired SSL certificate. Proceeding would be insecure."),
        # Browser not running
        lambda b: _browse_error_no_browser(b),
        # JavaScript error
        lambda b: _browse_error_js(b),
    ]
    for i in range(18):
        bond = BOND_STAGES[i % 4]
        scenario_fn = error_scenarios[i % len(error_scenarios)]
        examples.append(scenario_fn(bond))

    random.shuffle(examples)
    return examples[:200]


def _browse_error(bond, error_type, url, error_msg, response):
    tc = make_tool_call("browse", {"url": url})
    msgs = [
        {"role": "user", "content": f"Go to {url}"},
        make_assistant_tool(None, [tc]),
        make_tool_result(tc["id"], {"status": "error", "error": error_type, "message": error_msg}),
        {"role": "assistant", "content": response},
    ]
    return make_example(bond, msgs)


def _browse_error_element(bond):
    tc1 = make_tool_call("launch_browser", {"url": "https://example.com"})
    tc2 = make_tool_call("browser_click_element", {"selector": "#nonexistent-button"})
    msgs = [
        {"role": "user", "content": "Open example.com and click the submit button"},
        make_assistant_tool(None, [tc1]),
        make_tool_result(tc1["id"], {"status": "ok", "page_title": "Example Domain", "url": "https://example.com"}),
        make_assistant_tool(None, [tc2]),
        make_tool_result(tc2["id"], {"status": "error", "error": "element_not_found", "message": "No element matching selector #nonexistent-button"}),
        {"role": "assistant", "content": "I couldn't find a submit button on the page. The element #nonexistent-button doesn't exist. Would you like me to show you what elements are available?"},
    ]
    return make_example(bond, msgs)


def _browse_error_no_browser(bond):
    tc = make_tool_call("browser_see", {})
    msgs = [
        {"role": "user", "content": "What's on the current page?"},
        make_assistant_tool(None, [tc]),
        make_tool_result(tc["id"], {"status": "error", "error": "browser_not_running", "message": "No browser session is active"}),
        {"role": "assistant", "content": "No browser is currently open. Would you like me to launch one?"},
    ]
    return make_example(bond, msgs)


def _browse_error_js(bond):
    tc1 = make_tool_call("launch_browser", {"url": "https://example.com"})
    tc2 = make_tool_call("browser_execute_js", {"script": "document.querySelector('.missing').click()"})
    msgs = [
        {"role": "user", "content": "Open example.com and click the missing element via JS"},
        make_assistant_tool(None, [tc1]),
        make_tool_result(tc1["id"], {"status": "ok", "page_title": "Example Domain", "url": "https://example.com"}),
        make_assistant_tool(None, [tc2]),
        make_tool_result(tc2["id"], {"status": "error", "error": "js_error", "message": "TypeError: Cannot read property 'click' of null"}),
        {"role": "assistant", "content": "The JavaScript execution failed. The element .missing doesn't exist on the page, so the click threw a TypeError."},
    ]
    return make_example(bond, msgs)


# ===================================================================
# DELEGATE FAMILY (150 examples)
# ===================================================================

def gen_delegate_examples():
    examples = []

    # --- 1. queue_task / list_tasks / update_task / complete_task / cancel_task (30 examples) ---
    for i in range(30):
        bond = BOND_STAGES[i % 4]
        task_type = i % 5

        if task_type == 0:
            # queue_task
            desc = random.choice(TASK_DESCRIPTIONS)
            priority = random.choice(["low", "medium", "high", "urgent"])
            tc = make_tool_call("queue_task", {"description": desc, "priority": priority})
            task_id = rand_id("task_")
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Add a task: {desc}",
                    f"Queue up: {desc} with {priority} priority",
                    f"I need to {desc.lower()}, add it to my task list",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "task_id": task_id, "priority": priority}),
                {"role": "assistant", "content": f"Task queued ({priority} priority): {desc}."},
            ]
        elif task_type == 1:
            # list_tasks
            status_filter = random.choice(["all", "pending", "in_progress", "completed"])
            tc = make_tool_call("list_tasks", {"status": status_filter})
            tasks = [{"id": rand_id("task_"), "description": random.choice(TASK_DESCRIPTIONS), "status": random.choice(["pending", "in_progress"]), "priority": random.choice(["low", "medium", "high"])} for _ in range(random.randint(2, 6))]
            task_list = "\n".join(f"- [{t['priority']}] {t['description']} ({t['status']})" for t in tasks)
            msgs = [
                {"role": "user", "content": random.choice(["Show my tasks", "What's on my task list?", "List all pending tasks", "What do I need to work on?"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"tasks": tasks}),
                {"role": "assistant", "content": f"Your tasks:\n{task_list}"},
            ]
        elif task_type == 2:
            # update_task
            task_id = rand_id("task_")
            new_priority = random.choice(["low", "medium", "high", "urgent"])
            tc = make_tool_call("update_task", {"task_id": task_id, "priority": new_priority})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Change task {task_id} priority to {new_priority}",
                    f"Update {task_id} to {new_priority} priority",
                    f"Bump {task_id} to {new_priority}",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "task_id": task_id, "priority": new_priority}),
                {"role": "assistant", "content": f"Task {task_id} updated to {new_priority} priority."},
            ]
        elif task_type == 3:
            # complete_task
            task_id = rand_id("task_")
            desc = random.choice(TASK_DESCRIPTIONS)
            tc = make_tool_call("complete_task", {"task_id": task_id})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Mark {task_id} as done",
                    f"Complete task {task_id}",
                    f"I finished {desc.lower()}, mark it complete",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "task_id": task_id, "completed_at": "2026-03-13T14:30:00"}),
                {"role": "assistant", "content": f"Task {task_id} marked as completed."},
            ]
        else:
            # cancel_task
            task_id = rand_id("task_")
            tc = make_tool_call("cancel_task", {"task_id": task_id})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Cancel task {task_id}",
                    f"Remove {task_id} from my list",
                    f"Drop task {task_id}, I don't need it anymore",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "task_id": task_id, "cancelled": True}),
                {"role": "assistant", "content": f"Task {task_id} cancelled."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 2. create_recipe / list_recipes / run_recipe / pause_recipe / resume_recipe / cancel_recipe (24 examples) ---
    for i in range(24):
        bond = BOND_STAGES[i % 4]
        recipe_type = i % 6

        if recipe_type == 0:
            # create_recipe
            name = random.choice(RECIPE_NAMES)
            steps = [
                {"action": "check_email", "params": {"folder": "inbox"}},
                {"action": "summarize", "params": {"max_items": 5}},
                {"action": "notify", "params": {"channel": "desktop"}},
            ]
            tc = make_tool_call("create_recipe", {"name": name, "steps": steps, "description": f"Automated {name.replace('-', ' ')} workflow"})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Create a recipe called {name} that checks email, summarizes, and notifies me",
                    f"Set up an automation recipe: {name}",
                    f"Build a workflow named {name}",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "recipe_id": rand_id("recipe_"), "name": name, "steps_count": len(steps)}),
                {"role": "assistant", "content": f"Recipe \"{name}\" created with {len(steps)} steps."},
            ]
        elif recipe_type == 1:
            # list_recipes
            tc = make_tool_call("list_recipes", {})
            recipes = [{"id": rand_id("recipe_"), "name": random.choice(RECIPE_NAMES), "status": random.choice(["idle", "running", "paused"])} for _ in range(random.randint(2, 5))]
            recipe_list = "\n".join(f"- {r['name']} ({r['status']})" for r in recipes)
            msgs = [
                {"role": "user", "content": random.choice(["List my recipes", "Show all automation recipes", "What recipes do I have?"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"recipes": recipes}),
                {"role": "assistant", "content": f"Your recipes:\n{recipe_list}"},
            ]
        elif recipe_type == 2:
            # run_recipe
            name = random.choice(RECIPE_NAMES)
            recipe_id = rand_id("recipe_")
            tc = make_tool_call("run_recipe", {"recipe_id": recipe_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Run the {name} recipe", f"Execute recipe {recipe_id}", f"Start {name}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "recipe_id": recipe_id, "run_id": rand_id("run_"), "started_at": "2026-03-13T14:00:00"}),
                {"role": "assistant", "content": f"Recipe {name} is now running."},
            ]
        elif recipe_type == 3:
            # pause_recipe
            recipe_id = rand_id("recipe_")
            name = random.choice(RECIPE_NAMES)
            tc = make_tool_call("pause_recipe", {"recipe_id": recipe_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Pause the {name} recipe", f"Hold {name}", f"Pause recipe {recipe_id}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "recipe_id": recipe_id, "paused": True}),
                {"role": "assistant", "content": f"Recipe {name} paused. You can resume it anytime."},
            ]
        elif recipe_type == 4:
            # resume_recipe
            recipe_id = rand_id("recipe_")
            name = random.choice(RECIPE_NAMES)
            tc = make_tool_call("resume_recipe", {"recipe_id": recipe_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Resume the {name} recipe", f"Continue {name}", f"Unpause {recipe_id}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "recipe_id": recipe_id, "resumed": True}),
                {"role": "assistant", "content": f"Recipe {name} resumed."},
            ]
        else:
            # cancel_recipe
            recipe_id = rand_id("recipe_")
            name = random.choice(RECIPE_NAMES)
            tc = make_tool_call("cancel_recipe", {"recipe_id": recipe_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Cancel the {name} recipe", f"Stop and cancel {name}", f"Abort recipe {recipe_id}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "recipe_id": recipe_id, "cancelled": True}),
                {"role": "assistant", "content": f"Recipe {name} cancelled."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 3. create_schedule / list_schedules / update_schedule / cancel_schedule (16 examples) ---
    for i in range(16):
        bond = BOND_STAGES[i % 4]
        sched_type = i % 4

        if sched_type == 0:
            # create_schedule
            recipe = random.choice(RECIPE_NAMES)
            cron = random.choice(CRON_EXPRESSIONS)
            tc = make_tool_call("create_schedule", {"recipe_id": rand_id("recipe_"), "cron": cron, "name": f"scheduled-{recipe}"})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Schedule {recipe} to run {cron}",
                    f"Set up a recurring schedule for {recipe}",
                    f"Run {recipe} on a cron schedule: {cron}",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "schedule_id": rand_id("sched_"), "cron": cron, "next_run": "2026-03-14T09:00:00"}),
                {"role": "assistant", "content": f"Schedule created for {recipe} ({cron}). Next run: 2026-03-14 at 09:00."},
            ]
        elif sched_type == 1:
            # list_schedules
            tc = make_tool_call("list_schedules", {})
            schedules = [{"id": rand_id("sched_"), "name": f"scheduled-{random.choice(RECIPE_NAMES)}", "cron": random.choice(CRON_EXPRESSIONS), "active": random.choice([True, False])} for _ in range(random.randint(2, 4))]
            sched_list = "\n".join(f"- {s['name']} ({s['cron']}) {'active' if s['active'] else 'paused'}" for s in schedules)
            msgs = [
                {"role": "user", "content": random.choice(["Show my schedules", "List all scheduled tasks", "What's scheduled?"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"schedules": schedules}),
                {"role": "assistant", "content": f"Your schedules:\n{sched_list}"},
            ]
        elif sched_type == 2:
            # update_schedule
            sched_id = rand_id("sched_")
            new_cron = random.choice(CRON_EXPRESSIONS)
            tc = make_tool_call("update_schedule", {"schedule_id": sched_id, "cron": new_cron})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Change schedule {sched_id} to {new_cron}",
                    f"Update the schedule timing to {new_cron}",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "schedule_id": sched_id, "cron": new_cron}),
                {"role": "assistant", "content": f"Schedule updated to {new_cron}."},
            ]
        else:
            # cancel_schedule
            sched_id = rand_id("sched_")
            tc = make_tool_call("cancel_schedule", {"schedule_id": sched_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Cancel schedule {sched_id}", "Remove the scheduled task", f"Delete schedule {sched_id}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "schedule_id": sched_id, "cancelled": True}),
                {"role": "assistant", "content": f"Schedule {sched_id} cancelled."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 4. delegate_to_agent / agent_status / agent_cancel (12 examples) ---
    for i in range(12):
        bond = BOND_STAGES[i % 4]
        agent_type = i % 3

        if agent_type == 0:
            # delegate_to_agent
            agent = random.choice(AGENT_NAMES)
            task_desc = random.choice(TASK_DESCRIPTIONS)
            tc = make_tool_call("delegate_to_agent", {"agent": agent, "task": task_desc, "timeout_minutes": random.choice([5, 10, 30, 60])})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Have the {agent} work on: {task_desc}",
                    f"Delegate to {agent}: {task_desc}",
                    f"Send this to {agent}: {task_desc}",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "agent": agent, "delegation_id": rand_id("del_"), "started_at": "2026-03-13T14:00:00"}),
                {"role": "assistant", "content": f"Delegated to {agent}. It's working on: {task_desc}."},
            ]
        elif agent_type == 1:
            # agent_status
            agent = random.choice(AGENT_NAMES)
            del_id = rand_id("del_")
            progress = random.randint(10, 90)
            tc = make_tool_call("agent_status", {"delegation_id": del_id})
            msgs = [
                {"role": "user", "content": random.choice([f"How's the {agent} doing?", f"Status of delegation {del_id}", f"Check on {agent}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "in_progress", "agent": agent, "progress_percent": progress, "current_step": f"Processing step {random.randint(1,5)} of {random.randint(5,10)}"}),
                {"role": "assistant", "content": f"{agent} is {progress}% done, currently processing step {random.randint(1,5)}."},
            ]
        else:
            # agent_cancel
            agent = random.choice(AGENT_NAMES)
            del_id = rand_id("del_")
            tc = make_tool_call("agent_cancel", {"delegation_id": del_id})
            msgs = [
                {"role": "user", "content": random.choice([f"Cancel the {agent} task", f"Stop {agent}", f"Abort delegation {del_id}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "delegation_id": del_id, "cancelled": True}),
                {"role": "assistant", "content": f"Cancelled the {agent} delegation."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 5. claude_think / claude_code (10 examples) ---
    for i in range(10):
        bond = BOND_STAGES[i % 4]
        if i < 5:
            # claude_think
            question = random.choice([
                "What's the best architecture for a microservices gateway?",
                "How should I structure the database schema for a multi-tenant SaaS?",
                "What are the trade-offs between gRPC and REST?",
                "Should I use Rust or Go for this systems project?",
                "How do I design a rate limiter that handles burst traffic?",
            ])
            tc = make_tool_call("claude_think", {"prompt": question, "max_tokens": random.choice([500, 1000, 2000])})
            analysis = f"Analysis of the problem: considering scalability, maintainability, and performance trade-offs. Recommendation: start with a simple approach and iterate based on measured bottlenecks."
            msgs = [
                {"role": "user", "content": random.choice([f"Think deeply about: {question}", f"I need a thorough analysis: {question}", f"Reason through this: {question}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "response": analysis}),
                {"role": "assistant", "content": analysis},
            ]
        else:
            # claude_code
            task = random.choice([
                "Write a function to parse CSV files with headers",
                "Create a retry decorator with exponential backoff",
                "Implement a simple LRU cache",
                "Write a bash script to monitor disk usage",
                "Create a Python script to merge JSON files",
            ])
            lang = random.choice(CODE_LANGUAGES)
            tc = make_tool_call("claude_code", {"task": task, "language": lang})
            code_snippet = f"# Generated {lang} code for: {task}\n# ... implementation ..."
            msgs = [
                {"role": "user", "content": random.choice([f"Write code: {task}", f"Generate {lang} code for: {task}", f"Code this up: {task}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "language": lang, "code": code_snippet, "lines": random.randint(10, 50)}),
                {"role": "assistant", "content": f"Generated {lang} code ({random.randint(10,50)} lines) for: {task}."},
            ]
        examples.append(make_example(bond, msgs))

    # --- 6. script_write / script_run / script_patch / script_list / script_read / code_execute (24 examples) ---
    for i in range(24):
        bond = BOND_STAGES[i % 4]
        script_type = i % 6

        if script_type == 0:
            # script_write
            name = random.choice(SCRIPT_NAMES)
            lang = "python" if name.endswith(".py") else "bash"
            content = f"#!/usr/bin/env {lang}\n# Auto-generated script\nprint('Hello from {name}')" if lang == "python" else f"#!/bin/bash\n# Auto-generated script\necho 'Running {name}'"
            tc = make_tool_call("script_write", {"name": name, "content": content, "language": lang})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"Write a {lang} script called {name}",
                    f"Create script {name}",
                    f"Generate a {lang} script named {name}",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "path": f"/opt/yantrik/scripts/{name}", "size_bytes": len(content)}),
                {"role": "assistant", "content": f"Script {name} written to /opt/yantrik/scripts/."},
            ]
        elif script_type == 1:
            # script_run
            name = random.choice(SCRIPT_NAMES)
            tc = make_tool_call("script_run", {"name": name, "args": ["--verbose"]})
            output = f"Running {name}...\nProcessed 42 items.\nDone."
            msgs = [
                {"role": "user", "content": random.choice([f"Run {name}", f"Execute {name} with --verbose", f"Start script {name}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "exit_code": 0, "stdout": output, "stderr": "", "elapsed_seconds": random.uniform(0.1, 30.0)}),
                {"role": "assistant", "content": f"Script {name} completed successfully. Output: processed 42 items."},
            ]
        elif script_type == 2:
            # script_patch
            name = random.choice(SCRIPT_NAMES)
            tc = make_tool_call("script_patch", {"name": name, "find": "old_function()", "replace": "new_function()"})
            msgs = [
                {"role": "user", "content": random.choice([
                    f"In {name}, replace old_function() with new_function()",
                    f"Patch {name}: change old_function to new_function",
                ])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "replacements": 1, "path": f"/opt/yantrik/scripts/{name}"}),
                {"role": "assistant", "content": f"Patched {name}: replaced old_function() with new_function() (1 occurrence)."},
            ]
        elif script_type == 3:
            # script_list
            tc = make_tool_call("script_list", {})
            scripts = [{"name": random.choice(SCRIPT_NAMES), "language": random.choice(CODE_LANGUAGES), "size_bytes": random.randint(100, 10000)} for _ in range(random.randint(3, 8))]
            script_list = "\n".join(f"- {s['name']} ({s['language']}, {s['size_bytes']} bytes)" for s in scripts)
            msgs = [
                {"role": "user", "content": random.choice(["List my scripts", "Show all scripts", "What scripts do I have?"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"scripts": scripts}),
                {"role": "assistant", "content": f"Your scripts:\n{script_list}"},
            ]
        elif script_type == 4:
            # script_read
            name = random.choice(SCRIPT_NAMES)
            content = f"#!/usr/bin/env python3\nimport os\n\ndef main():\n    print('Running {name}')\n\nif __name__ == '__main__':\n    main()"
            tc = make_tool_call("script_read", {"name": name})
            msgs = [
                {"role": "user", "content": random.choice([f"Show me the contents of {name}", f"Read {name}", f"What's in {name}?"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "name": name, "content": content, "lines": 7}),
                {"role": "assistant", "content": f"Contents of {name}:\n```\n{content}\n```"},
            ]
        else:
            # code_execute
            lang = random.choice(CODE_LANGUAGES)
            code_snippets = {
                "python": "print(sum(range(100)))",
                "bash": "ls -la /tmp | wc -l",
                "rust": "fn main() { println!(\"{}\", 2 + 2); }",
                "javascript": "console.log(Array.from({length: 10}, (_, i) => i * i))",
                "sql": "SELECT COUNT(*) FROM users WHERE active = true;",
            }
            code = code_snippets.get(lang, "echo 'hello'")
            results = {"python": "4950", "bash": "15", "rust": "4", "javascript": "[0,1,4,9,16,25,36,49,64,81]", "sql": "42"}
            tc = make_tool_call("code_execute", {"code": code, "language": lang})
            msgs = [
                {"role": "user", "content": random.choice([f"Run this {lang}: {code}", f"Execute: {code}", f"What does this output: {code}"])},
                make_assistant_tool(None, [tc]),
                make_tool_result(tc["id"], {"status": "ok", "exit_code": 0, "stdout": results.get(lang, "ok"), "language": lang}),
                {"role": "assistant", "content": f"Output: {results.get(lang, 'ok')}"},
            ]
        examples.append(make_example(bond, msgs))

    # --- 7. Multi-tool delegate workflows (37 examples ~25%) ---
    for i in range(37):
        bond = BOND_STAGES[i % 4]
        wf = i % 8

        if wf == 0:
            # Queue task then create schedule for it
            desc = random.choice(TASK_DESCRIPTIONS)
            cron = random.choice(CRON_EXPRESSIONS)
            task_id = rand_id("task_")
            recipe_id = rand_id("recipe_")
            tc1 = make_tool_call("queue_task", {"description": desc, "priority": "high"})
            tc2 = make_tool_call("create_recipe", {"name": f"auto-{desc[:20].replace(' ','-').lower()}", "steps": [{"action": "execute_task", "params": {"task_id": task_id}}], "description": f"Automate: {desc}"})
            tc3 = make_tool_call("create_schedule", {"recipe_id": recipe_id, "cron": cron, "name": f"sched-{desc[:15].replace(' ','-').lower()}"})
            msgs = [
                {"role": "user", "content": f"Add \"{desc}\" as a recurring task on schedule {cron}"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "task_id": task_id, "priority": "high"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "recipe_id": recipe_id, "name": f"auto-task", "steps_count": 1}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "schedule_id": rand_id("sched_"), "cron": cron, "next_run": "2026-03-14T09:00:00"}),
                {"role": "assistant", "content": f"Set up \"{desc}\" as a recurring task on schedule {cron}. Next run: 2026-03-14 at 09:00."},
            ]
        elif wf == 1:
            # Write script then run it
            name = random.choice(SCRIPT_NAMES)
            lang = "python" if name.endswith(".py") else "bash"
            content = f"#!/usr/bin/env {lang}\nprint('Automated task complete')" if lang == "python" else f"#!/bin/bash\necho 'Automated task complete'"
            tc1 = make_tool_call("script_write", {"name": name, "content": content, "language": lang})
            tc2 = make_tool_call("script_run", {"name": name})
            msgs = [
                {"role": "user", "content": f"Write a {lang} script called {name} that prints 'Automated task complete', then run it"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "path": f"/opt/yantrik/scripts/{name}", "size_bytes": len(content)}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "exit_code": 0, "stdout": "Automated task complete\n", "stderr": "", "elapsed_seconds": 0.3}),
                {"role": "assistant", "content": f"Wrote and ran {name}. Output: Automated task complete."},
            ]
        elif wf == 2:
            # Delegate to agent then check status
            agent = random.choice(AGENT_NAMES)
            task = random.choice(TASK_DESCRIPTIONS)
            del_id = rand_id("del_")
            tc1 = make_tool_call("delegate_to_agent", {"agent": agent, "task": task, "timeout_minutes": 30})
            tc2 = make_tool_call("agent_status", {"delegation_id": del_id})
            msgs = [
                {"role": "user", "content": f"Delegate \"{task}\" to {agent} and tell me how it's going"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "agent": agent, "delegation_id": del_id, "started_at": "2026-03-13T14:00:00"}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "in_progress", "agent": agent, "progress_percent": 35, "current_step": "Analyzing data"}),
                {"role": "assistant", "content": f"Delegated to {agent}. It's 35% done, currently analyzing data."},
            ]
        elif wf == 3:
            # List tasks, complete one, list again
            task_id = rand_id("task_")
            desc = random.choice(TASK_DESCRIPTIONS)
            tasks_before = [{"id": task_id, "description": desc, "status": "in_progress", "priority": "high"},
                           {"id": rand_id("task_"), "description": random.choice(TASK_DESCRIPTIONS), "status": "pending", "priority": "medium"}]
            tc1 = make_tool_call("list_tasks", {"status": "all"})
            tc2 = make_tool_call("complete_task", {"task_id": task_id})
            tc3 = make_tool_call("list_tasks", {"status": "pending"})
            tasks_after = [tasks_before[1]]
            msgs = [
                {"role": "user", "content": f"Show my tasks, mark the first one done, then show what's left"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"tasks": tasks_before}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "task_id": task_id, "completed_at": "2026-03-13T15:00:00"}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"tasks": tasks_after}),
                {"role": "assistant", "content": f"Completed \"{desc}\". Remaining: 1 pending task - {tasks_after[0]['description']}."},
            ]
        elif wf == 4:
            # Create recipe, schedule it, run it immediately
            name = random.choice(RECIPE_NAMES)
            recipe_id = rand_id("recipe_")
            cron = random.choice(CRON_EXPRESSIONS)
            steps = [{"action": "check_status", "params": {}}, {"action": "send_report", "params": {"to": "user"}}]
            tc1 = make_tool_call("create_recipe", {"name": name, "steps": steps, "description": f"{name} workflow"})
            tc2 = make_tool_call("create_schedule", {"recipe_id": recipe_id, "cron": cron, "name": f"sched-{name}"})
            tc3 = make_tool_call("run_recipe", {"recipe_id": recipe_id})
            msgs = [
                {"role": "user", "content": f"Create a {name} recipe with status check and report steps, schedule it for {cron}, and run it now"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "recipe_id": recipe_id, "name": name, "steps_count": 2}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "schedule_id": rand_id("sched_"), "cron": cron}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "recipe_id": recipe_id, "run_id": rand_id("run_"), "started_at": "2026-03-13T14:00:00"}),
                {"role": "assistant", "content": f"Recipe \"{name}\" created, scheduled for {cron}, and running now."},
            ]
        elif wf == 5:
            # Think then write code
            question = "What's the most efficient sorting algorithm for nearly-sorted data?"
            tc1 = make_tool_call("claude_think", {"prompt": question, "max_tokens": 1000})
            tc2 = make_tool_call("script_write", {"name": "timsort_demo.py", "content": "#!/usr/bin/env python3\n# Timsort is optimal for nearly-sorted data\nimport random\ndata = sorted(range(1000))\nfor _ in range(10): data[random.randint(0,999)] = random.randint(0,999)\ndata.sort()  # Uses Timsort\nprint('Sorted', len(data), 'items')", "language": "python"})
            msgs = [
                {"role": "user", "content": f"Think about the best sort for nearly-sorted data, then write a demo script"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "response": "For nearly-sorted data, Timsort is optimal. It has O(n) best case for sorted input and O(n log n) worst case. Python's built-in sort uses Timsort."}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "path": "/opt/yantrik/scripts/timsort_demo.py", "size_bytes": 230}),
                {"role": "assistant", "content": "Timsort is the best choice for nearly-sorted data -- O(n) best case. Wrote a demo script to /opt/yantrik/scripts/timsort_demo.py."},
            ]
        elif wf == 6:
            # Script read, patch, run
            name = random.choice(SCRIPT_NAMES)
            tc1 = make_tool_call("script_read", {"name": name})
            tc2 = make_tool_call("script_patch", {"name": name, "find": "DEBUG = False", "replace": "DEBUG = True"})
            tc3 = make_tool_call("script_run", {"name": name, "args": ["--check"]})
            msgs = [
                {"role": "user", "content": f"Read {name}, enable debug mode, then run it with --check"},
                make_assistant_tool(None, [tc1]),
                make_tool_result(tc1["id"], {"status": "ok", "name": name, "content": f"#!/usr/bin/env python3\nDEBUG = False\ndef main():\n    pass\nmain()", "lines": 5}),
                make_assistant_tool(None, [tc2]),
                make_tool_result(tc2["id"], {"status": "ok", "replacements": 1, "path": f"/opt/yantrik/scripts/{name}"}),
                make_assistant_tool(None, [tc3]),
                make_tool_result(tc3["id"], {"status": "ok", "exit_code": 0, "stdout": "Debug mode enabled. Check passed.\n", "stderr": "", "elapsed_seconds": 1.2}),
                {"role": "assistant", "content": f"Enabled debug mode in {name} and ran it. Check passed."},
            ]
        else:  # wf == 7
            # Queue multiple tasks
            descs = random.sample(TASK_DESCRIPTIONS, 3)
            tcs = [make_tool_call("queue_task", {"description": d, "priority": random.choice(["low", "medium", "high"])}) for d in descs]
            msgs = [{"role": "user", "content": f"Add these tasks: {', '.join(descs)}"}]
            for j, tc in enumerate(tcs):
                msgs.append(make_assistant_tool(None, [tc]))
                msgs.append(make_tool_result(tc["id"], {"status": "ok", "task_id": rand_id("task_"), "priority": "medium"}))
            msgs.append({"role": "assistant", "content": f"Added {len(descs)} tasks to your list."})
            examples.append(make_example(bond, msgs))
            continue

        examples.append(make_example(bond, msgs))

    # --- 8. Error handling (14 examples ~9%) ---
    error_scenarios_delegate = [
        # Task already completed
        lambda b: _delegate_error(b, "complete_task", {"task_id": rand_id("task_")},
                                  {"status": "error", "error": "already_completed", "message": "Task is already marked as completed"},
                                  "Mark that task as done", "That task was already completed previously."),
        # Task not found
        lambda b: _delegate_error(b, "update_task", {"task_id": "task_nonexistent", "priority": "high"},
                                  {"status": "error", "error": "not_found", "message": "Task task_nonexistent does not exist"},
                                  "Update task_nonexistent to high priority", "Task task_nonexistent doesn't exist. Check the task ID and try again."),
        # Recipe execution failure
        lambda b: _delegate_error(b, "run_recipe", {"recipe_id": rand_id("recipe_")},
                                  {"status": "error", "error": "execution_failed", "message": "Step 2 failed: connection timeout"},
                                  "Run the backup recipe", "The recipe failed at step 2 due to a connection timeout. You may want to check the network connection and retry."),
        # Agent timeout
        lambda b: _delegate_error(b, "agent_status", {"delegation_id": rand_id("del_")},
                                  {"status": "error", "error": "agent_timeout", "message": "Agent exceeded timeout of 30 minutes"},
                                  "Check on the research agent", "The agent timed out after 30 minutes. The task may be too large. Consider breaking it into smaller pieces."),
        # Script execution error
        lambda b: _delegate_error(b, "script_run", {"name": "broken.py"},
                                  {"status": "error", "exit_code": 1, "stdout": "", "stderr": "SyntaxError: unexpected indent at line 15"},
                                  "Run broken.py", "Script failed with a SyntaxError at line 15: unexpected indent. The script needs to be fixed before running."),
        # Code execution error
        lambda b: _delegate_error(b, "code_execute", {"code": "import nonexistent_module", "language": "python"},
                                  {"status": "error", "exit_code": 1, "stdout": "", "stderr": "ModuleNotFoundError: No module named 'nonexistent_module'"},
                                  "Run: import nonexistent_module", "The module 'nonexistent_module' is not installed. You may need to install it first."),
        # Cancel already cancelled task
        lambda b: _delegate_error(b, "cancel_task", {"task_id": rand_id("task_")},
                                  {"status": "error", "error": "already_cancelled", "message": "Task was already cancelled"},
                                  "Cancel that task", "That task was already cancelled."),
    ]
    for i in range(14):
        bond = BOND_STAGES[i % 4]
        scenario_fn = error_scenarios_delegate[i % len(error_scenarios_delegate)]
        examples.append(scenario_fn(bond))

    random.shuffle(examples)
    return examples[:150]


def _delegate_error(bond, tool_name, args, error_result, user_msg, asst_response):
    tc = make_tool_call(tool_name, args)
    msgs = [
        {"role": "user", "content": user_msg},
        make_assistant_tool(None, [tc]),
        make_tool_result(tc["id"], error_result),
        {"role": "assistant", "content": asst_response},
    ]
    return make_example(bond, msgs)


# ===================================================================
# MAIN
# ===================================================================

def main():
    browse_examples = gen_browse_examples()
    delegate_examples = gen_delegate_examples()

    browse_path = OUT_DIR / "batch_tools_05_browse.jsonl"
    delegate_path = OUT_DIR / "batch_tools_06_delegate.jsonl"

    with open(browse_path, "w", encoding="utf-8") as f:
        for line in browse_examples:
            f.write(line + "\n")

    with open(delegate_path, "w", encoding="utf-8") as f:
        for line in delegate_examples:
            f.write(line + "\n")

    print(f"BROWSE:   {len(browse_examples)} examples -> {browse_path}")
    print(f"DELEGATE: {len(delegate_examples)} examples -> {delegate_path}")

    # Validate
    for path, expected in [(browse_path, 200), (delegate_path, 150)]:
        with open(path, encoding="utf-8") as f:
            lines = f.readlines()
        assert len(lines) == expected, f"{path}: expected {expected}, got {len(lines)}"
        for i, line in enumerate(lines):
            obj = json.loads(line)
            assert "messages" in obj, f"{path} line {i}: missing 'messages'"
            assert obj["messages"][0]["role"] == "system", f"{path} line {i}: first msg not system"
        print(f"  {path.name}: validated {len(lines)} lines OK")

    # Tool coverage check
    browse_tools_used = set()
    delegate_tools_used = set()
    for path, tool_set in [(browse_path, browse_tools_used), (delegate_path, delegate_tools_used)]:
        with open(path, encoding="utf-8") as f:
            for line in f:
                obj = json.loads(line)
                for msg in obj["messages"]:
                    if "tool_calls" in msg and msg.get("tool_calls"):
                        for tc in msg["tool_calls"]:
                            tool_set.add(tc["function"]["name"])

    print(f"\nBROWSE tools covered ({len(browse_tools_used)}): {sorted(browse_tools_used)}")
    print(f"DELEGATE tools covered ({len(delegate_tools_used)}): {sorted(delegate_tools_used)}")

    browse_expected = {"launch_browser", "browse", "browser_snapshot", "browser_click_element",
                       "browser_type_element", "browser_see", "browser_search", "browser_cleanup",
                       "browser_status", "browser_scroll", "browser_back", "browser_forward",
                       "browser_tabs", "browser_new_tab", "browser_close_tab", "browser_switch_tab",
                       "browser_cookies", "browser_history", "browser_bookmarks", "browser_download",
                       "browser_screenshot", "browser_pdf", "browser_execute_js", "browser_wait_for",
                       "browser_fill_form"}
    delegate_expected = {"queue_task", "list_tasks", "update_task", "complete_task", "cancel_task",
                         "create_recipe", "list_recipes", "run_recipe", "pause_recipe", "resume_recipe",
                         "cancel_recipe", "create_schedule", "list_schedules", "update_schedule",
                         "cancel_schedule", "delegate_to_agent", "agent_status", "agent_cancel",
                         "claude_think", "claude_code", "script_write", "script_run", "script_patch",
                         "script_list", "script_read", "code_execute"}

    browse_missing = browse_expected - browse_tools_used
    delegate_missing = delegate_expected - delegate_tools_used
    if browse_missing:
        print(f"WARNING: BROWSE missing tools: {sorted(browse_missing)}")
    if delegate_missing:
        print(f"WARNING: DELEGATE missing tools: {sorted(delegate_missing)}")

    # Bond stage distribution
    for path, name in [(browse_path, "BROWSE"), (delegate_path, "DELEGATE")]:
        bond_counts = {s: 0 for s in BOND_STAGES}
        with open(path, encoding="utf-8") as f:
            for line in f:
                obj = json.loads(line)
                system_msg = obj["messages"][0]["content"]
                for stage in BOND_STAGES:
                    if stage.upper() in system_msg:
                        bond_counts[stage] += 1
                        break
        total = sum(bond_counts.values())
        print(f"\n{name} bond distribution: {bond_counts} (total: {total})")
        for stage, count in bond_counts.items():
            pct = count / total * 100
            print(f"  {stage}: {count} ({pct:.1f}%)")


if __name__ == "__main__":
    main()
