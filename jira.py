#!/usr/bin/env python3
"""Yantrik Jira CLI — lightweight replacement for Jira MCP.

Usage:
  python jira.py search <JQL>                        # Search issues
  python jira.py get <ISSUE-KEY>                     # Get issue details
  python jira.py create-epic <TITLE> [--desc TEXT] [--priority P]
  python jira.py create-task <PARENT-KEY> <TITLE> [--desc TEXT] [--priority P]
  python jira.py transition <ISSUE-KEY> <STATUS>     # e.g. "Done", "In Progress"
  python jira.py comment <ISSUE-KEY> <TEXT>           # Add comment
  python jira.py transitions <ISSUE-KEY>             # List available transitions
  python jira.py list-open                           # All open tasks
  python jira.py list-epics                          # All epics
  python jira.py batch-transition <STATUS> <KEY1> <KEY2> ...  # Bulk transition

Examples:
  python jira.py search "status = Open ORDER BY created DESC"
  python jira.py create-epic "New Feature" --desc "Description here" --priority High
  python jira.py transition YOS-100 Done
  python jira.py batch-transition Done YOS-100 YOS-101 YOS-102
"""

import json
import requests
import time
import sys
import io
from base64 import b64encode

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8', errors='replace')
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding='utf-8', errors='replace')

# --- Config ---
JIRA_BASE = "https://spranab.atlassian.net"
EMAIL = "developer@pranab.co.in"
API_TOKEN = "ATATT3xFfGF0g3cPAD_wVzPFFWuCRh489x2bT5nBpMr_DVZOlkmPk_nkx0VnZZqHzoy8SQhYlY936KIdKmYEgRmKMtE5VdD56Xuo63MbW3go2yirOTSd3au2Mma09lBBZWJ4XeF0C84X6Enr-lIBzk5oRIRPkhF56U84XYDqB824_zkHQNIfuwE=A2A47348"
PROJECT_KEY = "YOS"
TIMEOUT = 60

auth_str = b64encode(f"{EMAIL}:{API_TOKEN}".encode()).decode()
HEADERS = {
    "Authorization": f"Basic {auth_str}",
    "Content-Type": "application/json",
    "Accept": "application/json",
}

SESSION = requests.Session()
SESSION.headers.update(HEADERS)


def api(method, path, **kwargs):
    """Make Jira API call with retry."""
    url = f"{JIRA_BASE}{path}" if path.startswith("/") else path
    kwargs.setdefault("timeout", TIMEOUT)
    for attempt in range(3):
        try:
            resp = SESSION.request(method, url, **kwargs)
            if resp.status_code == 429:
                wait = int(resp.headers.get("Retry-After", 10))
                print(f"  Rate limited, waiting {wait}s...", file=sys.stderr)
                time.sleep(wait)
                continue
            return resp
        except (requests.exceptions.Timeout, requests.exceptions.ConnectionError) as e:
            wait = 5 * (attempt + 1)
            print(f"  Retry {attempt+1}/3: {e.__class__.__name__}, waiting {wait}s...", file=sys.stderr)
            time.sleep(wait)
    return None


def adf_text(text):
    """Create ADF document from plain text."""
    if not text:
        return None
    return {
        "type": "doc",
        "version": 1,
        "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": text}]}
        ],
    }


def adf_to_text(adf):
    """Extract plain text from ADF document."""
    if not adf:
        return ""
    texts = []
    def walk(node):
        if isinstance(node, dict):
            if node.get("type") == "text":
                texts.append(node.get("text", ""))
            for child in node.get("content", []):
                walk(child)
        elif isinstance(node, list):
            for item in node:
                walk(item)
    walk(adf)
    return " ".join(texts)


# ============================================================
# Commands
# ============================================================

def cmd_search(jql, max_results=50):
    """Search issues by JQL."""
    from urllib.parse import quote
    encoded_jql = quote(jql)
    fields = "summary,status,priority,issuetype,parent,assignee"
    resp = api("GET", f"/rest/api/3/search/jql?jql={encoded_jql}&maxResults={max_results}&fields={fields}")
    if not resp or resp.status_code != 200:
        print(f"ERROR: {resp.status_code if resp else 'TIMEOUT'} - {resp.text[:300] if resp else ''}")
        return []

    data = resp.json()
    issues = data.get("issues", [])
    total = data.get("total", 0)

    print(f"Found {total} issues (showing {len(issues)}):\n")
    for issue in issues:
        f = issue["fields"]
        key = issue["key"]
        summary = f["summary"]
        status = f["status"]["name"]
        priority = f.get("priority", {}).get("name", "?")
        itype = f["issuetype"]["name"]
        parent = f.get("parent", {}).get("key", "") if f.get("parent") else ""
        parent_str = f" [{parent}]" if parent else ""
        print(f"  {key:10s} {status:15s} {priority:8s} {itype:6s}{parent_str} {summary}")

    return issues


def cmd_get(issue_key):
    """Get issue details."""
    resp = api("GET", f"/rest/api/3/issue/{issue_key}")
    if not resp or resp.status_code != 200:
        print(f"ERROR: {resp.status_code if resp else 'TIMEOUT'}")
        return None

    data = resp.json()
    f = data["fields"]
    print(f"Key:         {data['key']}")
    print(f"Summary:     {f['summary']}")
    print(f"Type:        {f['issuetype']['name']}")
    print(f"Status:      {f['status']['name']}")
    print(f"Priority:    {f.get('priority', {}).get('name', '?')}")
    if f.get("parent"):
        print(f"Parent:      {f['parent']['key']} — {f['parent']['fields']['summary']}")
    if f.get("assignee"):
        print(f"Assignee:    {f['assignee']['displayName']}")
    desc = adf_to_text(f.get("description"))
    if desc:
        print(f"Description: {desc[:500]}")
    return data


def cmd_create_epic(title, description=None, priority="Medium"):
    """Create an epic."""
    payload = {
        "fields": {
            "project": {"key": PROJECT_KEY},
            "summary": title[:255],
            "issuetype": {"name": "Epic"},
            "priority": {"name": priority},
        }
    }
    if description:
        payload["fields"]["description"] = adf_text(description)

    resp = api("POST", "/rest/api/3/issue", json=payload)
    if resp and resp.status_code == 201:
        key = resp.json()["key"]
        print(f"Created epic: {key} — {title}")
        return key
    else:
        print(f"ERROR: {resp.status_code if resp else 'TIMEOUT'} - {resp.text[:200] if resp else ''}")
        return None


def cmd_create_task(parent_key, title, description=None, priority="Medium"):
    """Create a task under an epic."""
    payload = {
        "fields": {
            "project": {"key": PROJECT_KEY},
            "parent": {"key": parent_key},
            "summary": title[:255],
            "issuetype": {"name": "Task"},
            "priority": {"name": priority},
        }
    }
    if description:
        payload["fields"]["description"] = adf_text(description)

    resp = api("POST", "/rest/api/3/issue", json=payload)
    if resp and resp.status_code == 201:
        key = resp.json()["key"]
        print(f"Created task: {key} — {title} (parent: {parent_key})")
        return key
    else:
        print(f"ERROR: {resp.status_code if resp else 'TIMEOUT'} - {resp.text[:200] if resp else ''}")
        return None


def cmd_transitions(issue_key):
    """List available transitions for an issue."""
    resp = api("GET", f"/rest/api/3/issue/{issue_key}/transitions")
    if not resp or resp.status_code != 200:
        print(f"ERROR: {resp.status_code if resp else 'TIMEOUT'}")
        return []

    transitions = resp.json().get("transitions", [])
    print(f"Transitions for {issue_key}:")
    for t in transitions:
        print(f"  ID: {t['id']:5s}  Name: {t['name']:20s}  → {t['to']['name']}")
    return transitions


def cmd_transition(issue_key, status_name):
    """Transition an issue to a new status."""
    # First get available transitions
    resp = api("GET", f"/rest/api/3/issue/{issue_key}/transitions")
    if not resp or resp.status_code != 200:
        print(f"ERROR getting transitions: {resp.status_code if resp else 'TIMEOUT'}")
        return False

    transitions = resp.json().get("transitions", [])
    target = None
    for t in transitions:
        if t["name"].lower() == status_name.lower() or t["to"]["name"].lower() == status_name.lower():
            target = t
            break

    if not target:
        names = [f"{t['name']} (→{t['to']['name']})" for t in transitions]
        print(f"ERROR: No transition to '{status_name}'. Available: {', '.join(names)}")
        return False

    resp = api("POST", f"/rest/api/3/issue/{issue_key}/transitions", json={
        "transition": {"id": target["id"]}
    })
    if resp and resp.status_code == 204:
        print(f"  {issue_key} → {target['to']['name']}")
        return True
    else:
        print(f"ERROR: {resp.status_code if resp else 'TIMEOUT'} - {resp.text[:200] if resp else ''}")
        return False


def cmd_comment(issue_key, text):
    """Add a comment to an issue."""
    resp = api("POST", f"/rest/api/3/issue/{issue_key}/comment", json={
        "body": adf_text(text),
    })
    if resp and resp.status_code == 201:
        print(f"Comment added to {issue_key}")
        return True
    else:
        print(f"ERROR: {resp.status_code if resp else 'TIMEOUT'} - {resp.text[:200] if resp else ''}")
        return False


def cmd_batch_transition(status_name, keys):
    """Transition multiple issues."""
    success = 0
    for key in keys:
        if cmd_transition(key, status_name):
            success += 1
        time.sleep(0.3)
    print(f"\nTransitioned {success}/{len(keys)} issues to {status_name}")


def cmd_list_open():
    """List all open tasks."""
    return cmd_search(f"project = {PROJECT_KEY} AND status != Done ORDER BY priority DESC, created ASC", max_results=100)


def cmd_list_epics():
    """List all epics."""
    return cmd_search(f"project = {PROJECT_KEY} AND issuetype = Epic ORDER BY created DESC", max_results=100)


# ============================================================
# CLI
# ============================================================

def parse_flag(args, flag, default=None):
    """Extract --flag value from args list."""
    for i, a in enumerate(args):
        if a == flag and i + 1 < len(args):
            val = args[i + 1]
            args.pop(i)
            args.pop(i)
            return val
    return default


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    cmd = sys.argv[1]
    args = sys.argv[2:]

    if cmd == "search":
        if not args:
            print("Usage: jira.py search <JQL>")
            sys.exit(1)
        cmd_search(" ".join(args))

    elif cmd == "get":
        if not args:
            print("Usage: jira.py get <ISSUE-KEY>")
            sys.exit(1)
        cmd_get(args[0])

    elif cmd == "create-epic":
        if not args:
            print("Usage: jira.py create-epic <TITLE> [--desc TEXT] [--priority P]")
            sys.exit(1)
        desc = parse_flag(args, "--desc")
        priority = parse_flag(args, "--priority", "Medium")
        cmd_create_epic(" ".join(args), desc, priority)

    elif cmd == "create-task":
        if len(args) < 2:
            print("Usage: jira.py create-task <PARENT-KEY> <TITLE> [--desc TEXT] [--priority P]")
            sys.exit(1)
        parent = args.pop(0)
        desc = parse_flag(args, "--desc")
        priority = parse_flag(args, "--priority", "Medium")
        cmd_create_task(parent, " ".join(args), desc, priority)

    elif cmd == "transitions":
        if not args:
            print("Usage: jira.py transitions <ISSUE-KEY>")
            sys.exit(1)
        cmd_transitions(args[0])

    elif cmd == "transition":
        if len(args) < 2:
            print("Usage: jira.py transition <ISSUE-KEY> <STATUS>")
            sys.exit(1)
        cmd_transition(args[0], " ".join(args[1:]))

    elif cmd == "comment":
        if len(args) < 2:
            print("Usage: jira.py comment <ISSUE-KEY> <TEXT>")
            sys.exit(1)
        cmd_comment(args[0], " ".join(args[1:]))

    elif cmd == "batch-transition":
        if len(args) < 2:
            print("Usage: jira.py batch-transition <STATUS> <KEY1> <KEY2> ...")
            sys.exit(1)
        cmd_batch_transition(args[0], args[1:])

    elif cmd == "list-open":
        cmd_list_open()

    elif cmd == "list-epics":
        cmd_list_epics()

    else:
        print(f"Unknown command: {cmd}")
        print(__doc__)
        sys.exit(1)


if __name__ == "__main__":
    main()
