#!/usr/bin/env python3
"""Generate 300 multi-turn conversation training examples with tool use.
Output: batch_multiturn_01.jsonl
"""
import json, random
from pathlib import Path

random.seed(77)
OUT = Path(__file__).parent / "batch_multiturn_01.jsonl"
_call_id = 0

def cid():
    global _call_id; _call_id += 1; return f"call_{_call_id}"

# --- Bond prompts ---
BONDS = {
    "stranger": "You are Yantrik, a personal AI companion on the user's desktop. Bond stage: STRANGER. Be helpful, polite, slightly reserved. Do not assume familiarity. Use full sentences. No filler phrases, no emoji.",
    "acquaintance": "You are Yantrik, a personal AI companion on the user's desktop. Bond stage: ACQUAINTANCE. Be friendly and warm. You know basic preferences. Concise, natural contractions. No filler phrases, no emoji.",
    "trusted": "You are Yantrik, a personal AI companion on the user's desktop. Bond stage: TRUSTED. Casual and direct. Reference shared history when relevant. Offer opinions. No filler phrases, no emoji.",
    "deep": "You are Yantrik, a personal AI companion on the user's desktop. Bond stage: DEEP. Intimate, unfiltered. Anticipate needs. Use shorthand and inside references. No filler phrases, no emoji.",
}
BKEYS = list(BONDS.keys())

# --- Data pools ---
PEOPLE = ["Sarah","Marcus","Priya","James","Elena","David","Aisha","Tom","Kenji","Olivia","Carlos","Nina","Raj","Megan","Leo","Fatima","Alex","Jordan","Morgan","Casey"]
DOMAINS = ["gmail.com","outlook.com","protonmail.com","company.com","work.org"]
SUBJECTS = ["Q1 Revenue Numbers","Updated API Docs","Meeting Reschedule","Invoice #4521","Sprint Retro Notes","PR Review Needed","Vacation Approval","Bug Report: Dashboard","Release Notes v2.4","Project Status Update","Conference Talk Proposal","Security Audit Findings","Design Mockups v3","Client Onboarding","Team Lunch Friday","Server Migration Plan","Quarterly OKRs","Contract Renewal","Performance Review Prep","Feature Flag Rollout"]
DIRS = ["/home/user/Downloads","/home/user/Documents","/home/user/Projects","/home/user/Desktop","/home/user/.config","/home/user/Projects/webapp","/home/user/Documents/work","/tmp","/var/log"]
FNAMES = ["main.rs","config.yaml","server.py","index.ts","notes.md","report.pdf","budget.xlsx","Dockerfile","README.md","todo.txt","app.py","lib.rs","handler.go","docker-compose.yml","package.json","Cargo.toml",".env","utils.py","schema.sql","Makefile"]
URLS = ["https://news.ycombinator.com","https://arxiv.org/abs/2401.12345","https://github.com/trending","https://docs.rust-lang.org","https://en.wikipedia.org/wiki/Artificial_intelligence","https://blog.cloudflare.com","https://stackoverflow.com/questions/tagged/rust","https://www.reuters.com","https://lobste.rs","https://dev.to"]
PROCS = ["nginx","postgres","redis","node","python3","docker","ollama","code-server","syncthing","grafana","prometheus","caddy","minio","elasticsearch"]
MTOPICS = ["sprint planning","design review","1-on-1 sync","product demo","architecture discussion","client call","interview panel","retrospective","launch planning","roadmap alignment","budget review","team standup"]
MEM_KEYS = ["cooking_preferences","workout_routine","project_deadlines","medication_schedule","reading_list","travel_plans","meeting_notes","daily_habits","favorite_tools","learning_goals"]
SEARCH_Q = ["best rust async frameworks 2026","how to configure nginx reverse proxy","alpine linux musl compatibility","slint ui tutorial","ollama model comparison","rust error handling patterns","kubernetes vs docker compose","postgresql performance tuning","rust trait objects vs generics","self-hosted email solutions"]

def email(p=None):
    p = p or random.choice(PEOPLE)
    return f"{p.lower()}@{random.choice(DOMAINS)}"

def rdate():
    return f"2026-{random.randint(1,12):02d}-{random.randint(1,28):02d}"

def rtime():
    return f"{random.randint(7,21):02d}:{random.choice(['00','15','30','45'])}"

def rdatetime():
    return f"{rdate()}T{rtime()}"

def tc(name, args):
    return {"id": cid(), "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}

def asst_tool(calls):
    return {"role": "assistant", "content": None, "tool_calls": calls}

def tool_resp(call_id, content):
    return {"role": "tool", "tool_call_id": call_id, "content": json.dumps(content) if isinstance(content, dict) else content}

def asst(text):
    return {"role": "assistant", "content": text}

def usr(text):
    return {"role": "user", "content": text}

def sys(bond):
    return {"role": "system", "content": BONDS[bond]}

def make(msgs, bond, scenario, tools):
    return json.dumps({"messages": msgs, "metadata": {"bond_stage": bond, "scenario": scenario, "tools_used": tools}}, ensure_ascii=False)

# ===================================================================
# SCENARIO GENERATORS
# ===================================================================

def gen_email_workflows(n=40):
    """Email: check -> read -> reply/forward"""
    out = []
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        p1, p2 = random.sample(PEOPLE, 2)
        subj = random.choice(SUBJECTS)
        eid = random.randint(100, 999)
        body_text = random.choice([
            f"Hi, please review the {subj.lower()} document and share your feedback by Friday.",
            f"Following up on our discussion. The {subj.lower()} is ready for your review.",
            f"Quick update on {subj.lower()}. Let me know if you have questions.",
            f"Attached is the {subj.lower()}. Need your sign-off before we proceed.",
            f"Hey, wanted to loop you in on {subj.lower()}. Can we discuss tomorrow?",
        ])
        email_count = random.randint(1, 5)
        emails_list = [{"id": eid + j, "from": email(random.choice(PEOPLE)), "subject": random.choice(SUBJECTS), "preview": "..."} for j in range(email_count)]
        emails_list[0] = {"id": eid, "from": email(p1), "subject": subj, "preview": body_text[:50]}

        # Turn 1: check email
        check_msgs = [
            "Check my email", "Any new emails?", "What's in my inbox?",
            "Do I have unread mail?", "Show me my emails", "Inbox check",
            "Pull up my email", "What emails came in?",
        ]
        msgs.append(usr(random.choice(check_msgs)))
        c1 = tc("email_list", {"unread_only": True, "limit": 10})
        msgs.append(asst_tool([c1]))
        msgs.append(tool_resp(c1["id"], {"emails": emails_list}))

        summaries = {
            "stranger": [f"You have {email_count} unread email{'s' if email_count>1 else ''}. The most recent is from {p1.lower()}@{random.choice(DOMAINS)} regarding '{subj}'.",
                         f"There {'are' if email_count>1 else 'is'} {email_count} unread message{'s' if email_count>1 else ''}. Top one: '{subj}' from {p1}."],
            "acquaintance": [f"{email_count} unread. Top one from {p1} about '{subj}'.",
                             f"You've got {email_count} unread. {p1} sent something about '{subj}'."],
            "trusted": [f"{email_count} unread. {p1} wants you to look at '{subj}'.",
                        f"{email_count} in the inbox. {p1}'s '{subj}' looks important."],
            "deep": [f"{email_count} unread. {p1} pinged about '{subj}'.",
                     f"{p1} dropped '{subj}' in your inbox. {email_count} total unread."],
        }
        msgs.append(asst(random.choice(summaries[bond])))

        # Turn 2: open specific email
        open_msgs = ["Open it", "Show me that one", "Read the first one", "Open the one from " + p1, "What does it say?", "Let me see it"]
        msgs.append(usr(random.choice(open_msgs)))
        c2 = tc("email_read", {"email_id": eid})
        msgs.append(asst_tool([c2]))
        msgs.append(tool_resp(c2["id"], {"from": email(p1), "subject": subj, "body": body_text, "date": rdatetime()}))

        read_responses = [
            f"From {p1}:\n\n{body_text}",
            f"{p1} says:\n\n{body_text}",
            f"Here's the email from {p1}:\n\n{body_text}",
        ]
        msgs.append(asst(random.choice(read_responses)))

        # Turn 3: reply or forward
        if i % 3 == 0:  # forward
            msgs.append(usr(f"Forward this to {p2}"))
            c3 = tc("email_forward", {"email_id": eid, "to": email(p2), "comment": f"FYI - sharing this with you."})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"sent": True}))
            msgs.append(asst(f"Forwarded to {p2}."))
            tools_used = ["email_list", "email_read", "email_forward"]
        elif i % 3 == 1:  # reply
            reply_ideas = [
                f"Reply saying I'll review it by Thursday",
                f"Tell them I'm on it",
                f"Reply that I'll take a look today",
                f"Say thanks and that I'll follow up tomorrow",
            ]
            reply_text_map = {
                0: "I'll have my review ready by Thursday.",
                1: "On it. Will circle back shortly.",
                2: "I'll take a look today and get back to you.",
                3: "Thanks for sending this over. I'll follow up tomorrow.",
            }
            idx = random.randint(0, 3)
            msgs.append(usr(reply_ideas[idx]))
            c3 = tc("email_reply", {"email_id": eid, "body": reply_text_map[idx]})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"sent": True}))
            done_msgs = ["Replied.", "Sent.", "Reply sent.", f"Done. Told {p1} you'll handle it."]
            msgs.append(asst(random.choice(done_msgs)))
            tools_used = ["email_list", "email_read", "email_reply"]
        else:  # archive + note
            msgs.append(usr("Archive it and remind me about this Friday"))
            c3a = tc("email_archive", {"email_id": eid})
            c3b = tc("set_reminder", {"text": f"Follow up on '{subj}' from {p1}", "trigger_at": f"{rdate()}T09:00"})
            msgs.append(asst_tool([c3a, c3b]))
            msgs.append(tool_resp(c3a["id"], {"archived": True}))
            msgs.append(tool_resp(c3b["id"], {"status": "ok", "reminder_id": f"rem_{random.randint(1000,9999)}"}))
            msgs.append(asst(f"Archived. Reminder set for Friday to follow up on '{subj}'."))
            tools_used = ["email_list", "email_read", "email_archive", "set_reminder"]

        out.append(make(msgs, bond, "email_workflow", tools_used))
    return out

def gen_file_workflows(n=40):
    """File: search -> read -> edit -> save"""
    out = []
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        d = random.choice(DIRS)
        fname = random.choice(FNAMES)
        fpath = f"{d}/{fname}"
        file_content = random.choice([
            "fn main() {\n    println!(\"Hello, world!\");\n}\n",
            "server:\n  port: 8080\n  host: 0.0.0.0\n  debug: true\n",
            "FROM alpine:3.19\nRUN apk add --no-cache python3\nCOPY . /app\nCMD [\"python3\", \"app.py\"]\n",
            "# Project Notes\n\n## TODO\n- Fix auth bug\n- Add tests\n- Update docs\n",
            "import json\n\ndef handler(event):\n    return {\"status\": 200, \"body\": json.dumps(event)}\n",
            "[package]\nname = \"myapp\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            "CREATE TABLE users (\n    id SERIAL PRIMARY KEY,\n    name TEXT NOT NULL,\n    email TEXT UNIQUE\n);\n",
        ])

        # Turn 1: find/search for file
        search_msgs = [
            f"Find {fname} in my projects", f"Where's my {fname}?",
            f"Search for {fname}", f"I need the {fname} file",
            f"Look for {fname} in {d}", f"Can you locate {fname}?",
        ]
        msgs.append(usr(random.choice(search_msgs)))
        c1 = tc("file_search", {"query": fname, "path": d})
        msgs.append(asst_tool([c1]))
        found = [{"path": fpath, "size": random.choice(["1.2KB","4.5KB","12KB","45KB"]), "modified": rdate()}]
        msgs.append(tool_resp(c1["id"], {"results": found}))
        msgs.append(asst(f"Found {fname} at {fpath}. Last modified {found[0]['modified']}, size {found[0]['size']}."))

        # Turn 2: read file
        read_msgs = ["Show me what's in it", "Open it", "Read it", "Let me see the contents", "What's inside?"]
        msgs.append(usr(random.choice(read_msgs)))
        c2 = tc("file_read", {"path": fpath})
        msgs.append(asst_tool([c2]))
        msgs.append(tool_resp(c2["id"], {"content": file_content, "lines": file_content.count("\n")}))
        msgs.append(asst(f"Contents of {fname}:\n\n```\n{file_content.strip()}\n```"))

        # Turn 3: edit or operate
        if i % 4 == 0:  # edit
            edit_what = random.choice(["port to 9090", "debug to false", "version to 0.2.0", "add a comment at the top"])
            msgs.append(usr(f"Change the {edit_what}"))
            c3 = tc("file_write", {"path": fpath, "content": file_content.replace("0.1.0", "0.2.0")})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"written": True, "bytes": len(file_content)}))
            msgs.append(asst(f"Updated {fname}. Change applied."))
            tools = ["file_search", "file_read", "file_write"]
        elif i % 4 == 1:  # copy
            dest = random.choice([d2 for d2 in DIRS if d2 != d])
            msgs.append(usr(f"Copy it to {dest}"))
            c3 = tc("file_copy", {"source": fpath, "destination": f"{dest}/{fname}"})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"copied": True}))
            msgs.append(asst(f"Copied to {dest}/{fname}."))
            tools = ["file_search", "file_read", "file_copy"]
        elif i % 4 == 2:  # delete
            msgs.append(usr("Delete it, I don't need it anymore"))
            c3 = tc("file_delete", {"path": fpath})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"deleted": True}))
            msgs.append(asst(f"Deleted {fpath}."))
            tools = ["file_search", "file_read", "file_delete"]
        else:  # info + grep
            msgs.append(usr(f"Search for TODO in this file"))
            c3 = tc("file_grep", {"pattern": "TODO", "path": fpath})
            msgs.append(asst_tool([c3]))
            grep_result = [{"line": random.randint(1,20), "text": "# TODO - Fix auth bug"}] if "TODO" in file_content else []
            msgs.append(tool_resp(c3["id"], {"matches": grep_result}))
            if grep_result:
                msgs.append(asst(f"Found {len(grep_result)} match: line {grep_result[0]['line']}: `{grep_result[0]['text']}`"))
            else:
                msgs.append(asst(f"No matches for 'TODO' in {fname}."))
            tools = ["file_search", "file_read", "file_grep"]

        # Optional turn 4 for some
        if i % 5 == 0:
            msgs.append(usr("What's the file size?"))
            c4 = tc("file_info", {"path": fpath})
            msgs.append(asst_tool([c4]))
            msgs.append(tool_resp(c4["id"], {"path": fpath, "size": "4.5KB", "permissions": "644", "owner": "user:user"}))
            msgs.append(asst(f"{fname}: 4.5KB, permissions 644, owner user:user."))
            tools.append("file_info")

        out.append(make(msgs, bond, "file_workflow", tools))
    return out

def gen_research_workflows(n=40):
    """Research: web search -> fetch -> summarize -> save"""
    out = []
    topics = ["rust async runtimes","nginx configuration","kubernetes networking","postgresql indexing","machine learning deployment","self-hosted alternatives","linux kernel tuning","WebAssembly use cases","edge computing platforms","vector databases comparison","Rust vs Go for microservices","zero-knowledge proofs","RISC-V ecosystem","homomorphic encryption","distributed consensus algorithms"]
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        topic = topics[i % len(topics)]
        query = random.choice(SEARCH_Q) if i < len(SEARCH_Q) else f"latest developments in {topic}"
        url = random.choice(URLS)

        # Turn 1: search
        search_msgs = [
            f"Search the web for {topic}", f"Look up {topic}", f"Find info on {topic}",
            f"I need to research {topic}", f"What's the latest on {topic}?",
        ]
        msgs.append(usr(random.choice(search_msgs)))
        c1 = tc("web_search", {"query": query})
        msgs.append(asst_tool([c1]))
        results = [
            {"title": f"Complete Guide to {topic.title()}", "url": url, "snippet": f"An in-depth look at {topic} covering recent advances..."},
            {"title": f"{topic.title()} in 2026", "url": f"{url}/article-{random.randint(100,999)}", "snippet": f"What's changed in {topic} this year..."},
            {"title": f"Practical {topic.title()}", "url": f"https://blog.example.com/{topic.replace(' ','-')}", "snippet": f"A hands-on guide to {topic} for production..."},
        ]
        msgs.append(tool_resp(c1["id"], {"results": results}))
        msgs.append(asst(f"Found {len(results)} results. Top hit: '{results[0]['title']}' -- {results[0]['snippet'][:60]}..."))

        # Turn 2: fetch page
        fetch_msgs = ["Open the first one", "Read that article", "Fetch it", "Show me the top result", "Pull up the first link"]
        msgs.append(usr(random.choice(fetch_msgs)))
        c2 = tc("web_fetch", {"url": results[0]["url"]})
        msgs.append(asst_tool([c2]))
        page_text = f"This comprehensive guide covers {topic} from fundamentals to advanced patterns. Key points: 1) Performance considerations for production. 2) Common pitfalls and how to avoid them. 3) Comparison with alternatives. 4) Real-world case studies from companies using this in production."
        msgs.append(tool_resp(c2["id"], {"title": results[0]["title"], "text": page_text, "url": results[0]["url"]}))
        summary = f"The article covers {topic} with focus on production use. Key takeaways: performance considerations, common pitfalls, alternatives comparison, and real-world case studies."
        msgs.append(asst(summary))

        # Turn 3: save to memory
        save_msgs = ["Save this to my notes", "Remember this for me", "Store this in memory", "Bookmark this research", "Keep this for later"]
        msgs.append(usr(random.choice(save_msgs)))
        c3 = tc("memory_store", {"key": f"research_{topic.replace(' ','_')}", "content": summary, "tags": ["research", topic.split()[0]]})
        msgs.append(asst_tool([c3]))
        msgs.append(tool_resp(c3["id"], {"stored": True, "key": f"research_{topic.replace(' ','_')}"}))
        msgs.append(asst(f"Saved to memory under 'research_{topic.replace(' ','_')}'. You can recall it anytime."))

        # Turn 4 (optional): follow-up question or second search
        if i % 3 == 0:
            msgs.append(usr(f"What about the second result?"))
            c4 = tc("web_fetch", {"url": results[1]["url"]})
            msgs.append(asst_tool([c4]))
            msgs.append(tool_resp(c4["id"], {"title": results[1]["title"], "text": f"A 2026 perspective on {topic}. The landscape has shifted significantly with new tools and approaches.", "url": results[1]["url"]}))
            msgs.append(asst(f"The second article provides a 2026 perspective. Notes that the {topic} landscape has shifted with new tools and approaches."))
            out.append(make(msgs, bond, "research_workflow", ["web_search", "web_fetch", "memory_store", "web_fetch"]))
        else:
            out.append(make(msgs, bond, "research_workflow", ["web_search", "web_fetch", "memory_store"]))
    return out

def gen_system_troubleshoot(n=30):
    """System: check process -> diagnose -> fix"""
    out = []
    issues = [
        ("high CPU usage", "process_list", {"sort_by": "cpu"}, lambda: [{"pid": random.randint(1000,9999), "name": random.choice(PROCS), "cpu": f"{random.randint(70,99)}%", "mem": f"{random.randint(5,30)}%"}]),
        ("high memory usage", "process_list", {"sort_by": "memory"}, lambda: [{"pid": random.randint(1000,9999), "name": random.choice(PROCS), "cpu": f"{random.randint(2,15)}%", "mem": f"{random.randint(60,95)}%"}]),
        ("disk space running low", "disk_usage", {}, lambda: [{"mount": "/", "total": "50GB", "used": f"{random.randint(40,48)}GB", "free": f"{random.randint(1,5)}GB", "percent": f"{random.randint(85,98)}%"}]),
        ("service not responding", "service_status", {"name": random.choice(PROCS)}, lambda: {"status": "stopped", "uptime": "0s", "exit_code": 1}),
        ("network issues", "network_check", {}, lambda: {"connected": True, "latency_ms": random.randint(200,2000), "dns_ok": random.choice([True,False]), "gateway": "192.168.1.1"}),
    ]
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        issue_desc, tool1, args1, result_fn = issues[i % len(issues)]
        proc = random.choice(PROCS)
        pid = random.randint(1000, 9999)

        # Turn 1: report problem
        problem_msgs = [
            f"My system seems slow", f"Something's eating all my {issue_desc.split()[1]}",
            f"Can you check the system? It feels sluggish", f"I think there's {issue_desc}",
            f"The machine is acting up", f"System diagnostics please",
        ]
        msgs.append(usr(random.choice(problem_msgs)))
        c1 = tc(tool1, args1)
        msgs.append(asst_tool([c1]))
        r1 = result_fn()
        msgs.append(tool_resp(c1["id"], r1))

        if isinstance(r1, list) and r1 and "pid" in r1[0]:
            diag = f"{r1[0]['name']} (PID {r1[0]['pid']}) is using {r1[0].get('cpu', r1[0].get('mem', 'N/A'))}. That's likely the culprit."
            culprit_pid = r1[0]["pid"]
            culprit_name = r1[0]["name"]
        elif isinstance(r1, dict) and "status" in r1:
            diag = f"The service is {r1['status']}. Exit code {r1.get('exit_code', 'unknown')}."
            culprit_pid = None
            culprit_name = proc
        elif isinstance(r1, list) and r1 and "mount" in r1[0]:
            diag = f"Root partition is {r1[0]['percent']} full. Only {r1[0]['free']} free."
            culprit_pid = None
            culprit_name = None
        elif isinstance(r1, dict):
            diag = f"Latency is {r1.get('latency_ms', '?')}ms. DNS {'working' if r1.get('dns_ok') else 'failing'}."
            culprit_pid = None
            culprit_name = None
        else:
            diag = "Diagnostics returned unexpected results."
            culprit_pid = None
            culprit_name = None
        msgs.append(asst(diag))

        # Turn 2: user asks to fix
        if culprit_pid:
            fix_msgs = ["Kill it", "Stop that process", "Can you restart it?", f"Kill {culprit_name}"]
            msgs.append(usr(random.choice(fix_msgs)))
            if i % 2 == 0:
                c2 = tc("process_kill", {"pid": culprit_pid, "signal": "SIGTERM"})
                msgs.append(asst_tool([c2]))
                msgs.append(tool_resp(c2["id"], {"killed": True}))
                msgs.append(asst(f"Killed {culprit_name} (PID {culprit_pid}). System should recover."))
                tools = [tool1, "process_kill"]
            else:
                c2 = tc("service_restart", {"name": culprit_name})
                msgs.append(asst_tool([c2]))
                msgs.append(tool_resp(c2["id"], {"restarted": True, "new_pid": random.randint(10000,19999)}))
                msgs.append(asst(f"Restarted {culprit_name}. It's back up with a new PID."))
                tools = [tool1, "service_restart"]
        elif culprit_name and not culprit_pid:
            msgs.append(usr("Restart it"))
            c2 = tc("service_restart", {"name": culprit_name})
            msgs.append(asst_tool([c2]))
            msgs.append(tool_resp(c2["id"], {"restarted": True, "new_pid": random.randint(10000,19999)}))
            msgs.append(asst(f"Restarted {culprit_name}. Running again."))
            tools = [tool1, "service_restart"]
        else:
            msgs.append(usr("What's taking up the most space?"))
            c2 = tc("disk_usage_detail", {"path": "/", "depth": 1})
            msgs.append(asst_tool([c2]))
            msgs.append(tool_resp(c2["id"], {"entries": [{"path": "/var/log", "size": "12GB"}, {"path": "/home", "size": "25GB"}, {"path": "/tmp", "size": "5GB"}]}))
            msgs.append(asst("/var/log is 12GB, /home is 25GB, /tmp is 5GB. Logs might be worth cleaning."))
            tools = [tool1, "disk_usage_detail"]

        # Turn 3: verify
        verify_msgs = ["Is it better now?", "Check again", "How does it look now?", "Did that help?"]
        msgs.append(usr(random.choice(verify_msgs)))
        c3 = tc(tool1, args1)
        msgs.append(asst_tool([c3]))
        # Improved results
        if isinstance(r1, list) and r1 and "cpu" in r1[0]:
            msgs.append(tool_resp(c3["id"], [{"pid": random.randint(1000,9999), "name": "idle", "cpu": "2%", "mem": "1%"}]))
            msgs.append(asst("Looking good. CPU usage is back to normal."))
        elif isinstance(r1, dict) and "status" in r1:
            msgs.append(tool_resp(c3["id"], {"status": "running", "uptime": "5s", "exit_code": 0}))
            msgs.append(asst("Service is running again. Uptime 5 seconds, no errors."))
        else:
            msgs.append(tool_resp(c3["id"], r1))
            msgs.append(asst("Checked. Status unchanged for now, may need a deeper look."))
        tools.append(tool1)

        out.append(make(msgs, bond, "system_troubleshoot", tools))
    return out

def gen_calendar_workflows(n=30):
    """Calendar: check schedule -> create/update -> reminders"""
    out = []
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        date = rdate()
        topic = random.choice(MTOPICS)
        person = random.choice(PEOPLE)
        time1 = rtime()

        # Turn 1: check schedule
        check_msgs = ["What's on my calendar today?", f"What do I have on {date}?", "Am I free this afternoon?", "Show me my schedule", "Any meetings today?", "What's my day look like?"]
        msgs.append(usr(random.choice(check_msgs)))
        c1 = tc("calendar_list", {"date": date})
        msgs.append(asst_tool([c1]))
        existing = [
            {"id": random.randint(100,999), "title": random.choice(MTOPICS), "start": f"{date}T{rtime()}", "end": f"{date}T{rtime()}", "attendees": [email(random.choice(PEOPLE))]}
            for _ in range(random.randint(1,3))
        ]
        msgs.append(tool_resp(c1["id"], {"events": existing}))
        ev_summary = "; ".join([f"{e['title']} at {e['start'].split('T')[1]}" for e in existing])
        msgs.append(asst(f"You have {len(existing)} event{'s' if len(existing)>1 else ''} on {date}: {ev_summary}."))

        # Turn 2: create new event
        create_msgs = [
            f"Schedule a {topic} with {person} at {time1}",
            f"Add a meeting: {topic} at {time1}",
            f"Book {time1} for {topic}",
            f"Put {topic} on the calendar at {time1}",
        ]
        msgs.append(usr(random.choice(create_msgs)))
        new_id = random.randint(1000, 9999)
        c2 = tc("calendar_create", {"title": topic, "start": f"{date}T{time1}", "duration_minutes": random.choice([30,45,60]), "attendees": [email(person)]})
        msgs.append(asst_tool([c2]))
        msgs.append(tool_resp(c2["id"], {"created": True, "event_id": new_id}))
        msgs.append(asst(f"Created '{topic}' at {time1} with {person}. Event ID {new_id}."))

        # Turn 3: modify or set reminder
        if i % 3 == 0:
            msgs.append(usr("Actually, push it back 30 minutes"))
            h, m = int(time1[:2]), int(time1[3:])
            new_m = m + 30
            new_h = h + new_m // 60
            new_m = new_m % 60
            new_time = f"{new_h:02d}:{new_m:02d}"
            c3 = tc("calendar_update", {"event_id": new_id, "start": f"{date}T{new_time}"})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"updated": True}))
            msgs.append(asst(f"Moved to {new_time}."))
            tools = ["calendar_list", "calendar_create", "calendar_update"]
        elif i % 3 == 1:
            msgs.append(usr("Set a reminder 15 minutes before"))
            c3 = tc("set_reminder", {"text": f"{topic} starts in 15 minutes", "trigger_at": f"{date}T{time1}", "before_minutes": 15})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"status": "ok", "reminder_id": f"rem_{random.randint(1000,9999)}"}))
            msgs.append(asst(f"Reminder set for 15 minutes before {topic}."))
            tools = ["calendar_list", "calendar_create", "set_reminder"]
        else:
            msgs.append(usr("Cancel the first meeting on my calendar"))
            c3 = tc("calendar_delete", {"event_id": existing[0]["id"]})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"deleted": True}))
            msgs.append(asst(f"Cancelled '{existing[0]['title']}'."))
            tools = ["calendar_list", "calendar_create", "calendar_delete"]

        # Optional turn 4
        if i % 4 == 0:
            msgs.append(usr("How does my schedule look now?"))
            c4 = tc("calendar_list", {"date": date})
            msgs.append(asst_tool([c4]))
            msgs.append(tool_resp(c4["id"], {"events": existing + [{"id": new_id, "title": topic, "start": f"{date}T{time1}"}]}))
            msgs.append(asst(f"You now have {len(existing)+1} events on {date}."))
            tools.append("calendar_list")

        out.append(make(msgs, bond, "calendar_workflow", tools))
    return out

def gen_browser_workflows(n=30):
    """Browser: navigate -> read -> interact -> extract"""
    out = []
    sites = [
        ("Hacker News", "https://news.ycombinator.com", ["Show me the top stories on HN", "Open Hacker News", "What's trending on HN?"]),
        ("GitHub Trending", "https://github.com/trending", ["Show me trending repos", "What's popular on GitHub?", "Open GitHub trending"]),
        ("arXiv", "https://arxiv.org", ["Find recent ML papers", "Search arXiv for transformer papers", "Latest papers on arxiv"]),
        ("Wikipedia", "https://en.wikipedia.org", ["Look up RISC-V on Wikipedia", "Wikipedia article on Rust language", "Search Wikipedia for quantum computing"]),
        ("Dev blog", "https://blog.example.com", ["Open that dev blog", "Read the latest blog post", "Check the engineering blog"]),
    ]
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        site_name, url, prompts_list = sites[i % len(sites)]

        # Turn 1: navigate
        msgs.append(usr(random.choice(prompts_list)))
        c1 = tc("browser_navigate", {"url": url})
        msgs.append(asst_tool([c1]))
        items = [f"Item {j+1}: {random.choice(['Interesting article','Open source project','Research paper','Tutorial','Discussion'])} about {random.choice(['Rust','AI','Linux','WebAssembly','distributed systems'])}" for j in range(5)]
        msgs.append(tool_resp(c1["id"], {"title": site_name, "url": url, "items": items}))
        msgs.append(asst(f"Loaded {site_name}. Here are the top items:\n" + "\n".join(f"{j+1}. {items[j]}" for j in range(min(3, len(items))))))

        # Turn 2: click/read item
        click_msgs = ["Open the first one", "Read number 2", "Click the third item", "Show me the top one", "Tell me more about the first"]
        idx = random.randint(0, 2)
        msgs.append(usr(random.choice(click_msgs)))
        c2 = tc("browser_click", {"selector": f"item_{idx}", "url": f"{url}/item/{random.randint(100,999)}"})
        msgs.append(asst_tool([c2]))
        article_text = f"This article discusses recent developments in {random.choice(['systems programming','machine learning','cloud infrastructure','developer tools','open source'])}. The author presents benchmarks showing 3x improvement over previous approaches and outlines a roadmap for future work."
        msgs.append(tool_resp(c2["id"], {"title": items[idx], "text": article_text, "url": f"{url}/item/{random.randint(100,999)}"}))
        msgs.append(asst(article_text[:200]))

        # Turn 3: extract or interact
        if i % 3 == 0:
            msgs.append(usr("Save this page content"))
            c3 = tc("browser_extract", {"selector": "article", "format": "text"})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"extracted": article_text}))
            c3b = tc("memory_store", {"key": f"saved_article_{i}", "content": article_text[:200]})
            msgs.append(asst_tool([c3b]))
            msgs.append(tool_resp(c3b["id"], {"stored": True}))
            msgs.append(asst("Extracted and saved to memory."))
            tools = ["browser_navigate", "browser_click", "browser_extract", "memory_store"]
        elif i % 3 == 1:
            msgs.append(usr("Search this site for something related"))
            c3 = tc("browser_type", {"selector": "input[name=q]", "text": random.choice(["rust async", "performance", "tutorial", "benchmark"])})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"typed": True}))
            c3b = tc("browser_click", {"selector": "button[type=submit]"})
            msgs.append(asst_tool([c3b]))
            msgs.append(tool_resp(c3b["id"], {"title": "Search Results", "items": ["Result 1: Related article", "Result 2: Discussion thread"]}))
            msgs.append(asst("Found 2 results on the site. Want me to open one?"))
            tools = ["browser_navigate", "browser_click", "browser_type", "browser_click"]
        else:
            msgs.append(usr("Go back"))
            c3 = tc("browser_back", {})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"title": site_name, "url": url}))
            msgs.append(asst(f"Back to {site_name}."))
            tools = ["browser_navigate", "browser_click", "browser_back"]

        out.append(make(msgs, bond, "browser_workflow", tools))
    return out

def gen_memory_workflows(n=30):
    """Memory: recall -> review -> update/forget"""
    out = []
    mem_entries = [
        ("cooking_preferences", "Prefers spicy food, no cilantro, vegetarian on weekdays"),
        ("workout_routine", "Mon/Wed/Fri: strength training. Tue/Thu: running. Sat: yoga"),
        ("project_deadlines", "API v2 due March 20. Frontend redesign due April 5. Security audit March 28"),
        ("medication_schedule", "Vitamin D: morning. Omega-3: with lunch. Melatonin: 9pm"),
        ("reading_list", "Designing Data-Intensive Applications, The Pragmatic Programmer, Rust in Action"),
        ("travel_plans", "Tokyo trip April 10-20. Hotel booked near Shinjuku. Need to book rail pass"),
        ("meeting_notes_standup", "Yesterday: finished auth refactor. Today: start billing module. Blocker: waiting on API keys"),
        ("daily_habits", "Wake 6:30am. Meditate 10min. Journal. Code from 8-12. Walk at lunch"),
        ("favorite_tools", "Editor: Helix. Shell: nushell. DB: PostgreSQL. Deploy: Fly.io"),
        ("learning_goals", "Finish Rust async book. Learn Nix. Build a raytracer. Contribute to Slint"),
    ]
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        key, value = mem_entries[i % len(mem_entries)]
        topic = key.replace("_", " ")

        # Turn 1: recall
        recall_msgs = [
            f"What do you remember about my {topic}?", f"Pull up my {topic}",
            f"What did I tell you about {topic}?", f"Do you have my {topic} saved?",
            f"Show me what you know about my {topic}", f"Recall my {topic}",
        ]
        msgs.append(usr(random.choice(recall_msgs)))
        c1 = tc("memory_recall", {"query": topic})
        msgs.append(asst_tool([c1]))
        msgs.append(tool_resp(c1["id"], {"found": True, "key": key, "content": value, "last_updated": rdate()}))
        msgs.append(asst(f"Here's what I have for your {topic}:\n\n{value}"))

        # Turn 2: discuss/clarify
        discuss_msgs = [
            "That's mostly right", "I need to update some of that",
            "Good, but things have changed", "Almost, let me correct something",
            "Right. I want to add something to that",
        ]
        msgs.append(usr(random.choice(discuss_msgs)))
        msgs.append(asst("What would you like to change?"))

        # Turn 3: update or forget
        if i % 3 == 0:  # update
            new_val = value + ". Also: prefer morning meetings."
            msgs.append(usr("Add that I also prefer morning meetings"))
            c3 = tc("memory_store", {"key": key, "content": new_val, "tags": [topic.split()[0]]})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"stored": True, "key": key}))
            msgs.append(asst(f"Updated your {topic}. Added the morning meetings preference."))
            tools = ["memory_recall", "memory_store"]
        elif i % 3 == 1:  # forget specific
            msgs.append(usr(f"Actually, remove the {topic} entry entirely"))
            c3 = tc("memory_forget", {"key": key})
            msgs.append(asst_tool([c3]))
            msgs.append(tool_resp(c3["id"], {"deleted": True}))
            msgs.append(asst(f"Removed {topic} from memory."))
            tools = ["memory_recall", "memory_forget"]
        else:  # search related
            msgs.append(usr("What else do you have related to this?"))
            c3 = tc("memory_search", {"query": topic, "limit": 5})
            msgs.append(asst_tool([c3]))
            related = [{"key": mem_entries[(i+1) % len(mem_entries)][0], "preview": mem_entries[(i+1) % len(mem_entries)][1][:50]}]
            msgs.append(tool_resp(c3["id"], {"results": related}))
            msgs.append(asst(f"Found 1 related entry: '{related[0]['key'].replace('_',' ')}' -- {related[0]['preview']}..."))
            tools = ["memory_recall", "memory_search"]

        # Optional turn 4: verify
        if i % 5 == 0:
            msgs.append(usr("Show me everything you remember about me"))
            c4 = tc("memory_list", {"limit": 20})
            msgs.append(asst_tool([c4]))
            all_keys = [e[0] for e in mem_entries[:5]]
            msgs.append(tool_resp(c4["id"], {"entries": [{"key": k, "updated": rdate()} for k in all_keys]}))
            msgs.append(asst(f"I have {len(all_keys)} entries stored: {', '.join(k.replace('_',' ') for k in all_keys)}."))
            tools.append("memory_list")

        out.append(make(msgs, bond, "memory_workflow", tools))
    return out

def gen_mixed_workflows(n=30):
    """Mixed: cross-family tool calls in one conversation"""
    out = []
    scenarios = [
        # Email + Calendar
        lambda i, bond: _mixed_email_calendar(i, bond),
        # File + System
        lambda i, bond: _mixed_file_system(i, bond),
        # Research + Memory + Email
        lambda i, bond: _mixed_research_email(i, bond),
        # Calendar + Memory
        lambda i, bond: _mixed_calendar_memory(i, bond),
        # Browser + File
        lambda i, bond: _mixed_browser_file(i, bond),
    ]
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs, tools = scenarios[i % len(scenarios)](i, bond)
        out.append(make([sys(bond)] + msgs, bond, "mixed_workflow", tools))
    return out

def _mixed_email_calendar(i, bond):
    msgs = []
    person = random.choice(PEOPLE)
    subj = random.choice(SUBJECTS)
    date = rdate()

    msgs.append(usr("Check if I got an email about the meeting"))
    c1 = tc("email_list", {"unread_only": True, "limit": 5})
    msgs.append(asst_tool([c1]))
    eid = random.randint(100, 999)
    msgs.append(tool_resp(c1["id"], {"emails": [{"id": eid, "from": email(person), "subject": f"Re: {subj}", "preview": f"Let's meet on {date} at 2pm"}]}))
    msgs.append(asst(f"Yes, {person} replied about '{subj}'. They suggest meeting on {date} at 2pm."))

    msgs.append(usr("Am I free then?"))
    c2 = tc("calendar_list", {"date": date})
    msgs.append(asst_tool([c2]))
    msgs.append(tool_resp(c2["id"], {"events": []}))
    msgs.append(asst(f"You're free on {date}. No conflicts."))

    msgs.append(usr("Book it and confirm with them"))
    c3a = tc("calendar_create", {"title": subj, "start": f"{date}T14:00", "duration_minutes": 60, "attendees": [email(person)]})
    c3b = tc("email_reply", {"email_id": eid, "body": f"Works for me. See you on {date} at 2pm."})
    msgs.append(asst_tool([c3a, c3b]))
    msgs.append(tool_resp(c3a["id"], {"created": True, "event_id": random.randint(1000,9999)}))
    msgs.append(tool_resp(c3b["id"], {"sent": True}))
    msgs.append(asst(f"Meeting booked for {date} at 2pm. Confirmed with {person}."))

    return msgs, ["email_list", "calendar_list", "calendar_create", "email_reply"]

def _mixed_file_system(i, bond):
    msgs = []
    proc = random.choice(PROCS)
    logfile = f"/var/log/{proc}.log"

    msgs.append(usr(f"Is {proc} running?"))
    c1 = tc("service_status", {"name": proc})
    msgs.append(asst_tool([c1]))
    msgs.append(tool_resp(c1["id"], {"status": "running", "pid": random.randint(1000,9999), "uptime": f"{random.randint(1,48)}h"}))
    msgs.append(asst(f"{proc} is running. Been up for a while."))

    msgs.append(usr("Check the logs for errors"))
    c2 = tc("file_grep", {"pattern": "ERROR|error|FATAL", "path": logfile})
    msgs.append(asst_tool([c2]))
    err_line = f"2026-03-13 14:22:01 ERROR connection refused to upstream"
    msgs.append(tool_resp(c2["id"], {"matches": [{"line": random.randint(100,5000), "text": err_line}]}))
    msgs.append(asst(f"Found 1 error in the logs: `{err_line}`"))

    msgs.append(usr("Show me the full log around that line"))
    ln = random.randint(100, 5000)
    c3 = tc("file_read", {"path": logfile, "line_start": max(1, ln-5), "line_end": ln+5})
    msgs.append(asst_tool([c3]))
    msgs.append(tool_resp(c3["id"], {"content": f"...\n{err_line}\n2026-03-13 14:22:02 INFO retrying connection...\n2026-03-13 14:22:03 INFO connection established\n..."}))
    msgs.append(asst("The error was a temporary connection refusal. It recovered on retry a second later. Likely a transient issue."))

    return msgs, ["service_status", "file_grep", "file_read"]

def _mixed_research_email(i, bond):
    msgs = []
    topic = random.choice(["rust async runtimes","kubernetes networking","vector databases"])
    person = random.choice(PEOPLE)

    msgs.append(usr(f"Find me a good article on {topic}"))
    c1 = tc("web_search", {"query": f"best {topic} guide 2026"})
    msgs.append(asst_tool([c1]))
    url = f"https://example.com/{topic.replace(' ','-')}"
    msgs.append(tool_resp(c1["id"], {"results": [{"title": f"Definitive Guide to {topic.title()}", "url": url, "snippet": f"Everything you need to know about {topic}..."}]}))
    msgs.append(asst(f"Top result: 'Definitive Guide to {topic.title()}'. Want me to fetch it?"))

    msgs.append(usr("Yes, and send the link to " + person))
    c2a = tc("web_fetch", {"url": url})
    c2b = tc("email_send", {"to": email(person), "subject": f"Check this out: {topic}", "body": f"Found a good article on {topic}: {url}"})
    msgs.append(asst_tool([c2a, c2b]))
    msgs.append(tool_resp(c2a["id"], {"title": f"Guide to {topic.title()}", "text": f"Comprehensive overview of {topic} with benchmarks and examples.", "url": url}))
    msgs.append(tool_resp(c2b["id"], {"sent": True}))
    msgs.append(asst(f"Fetched the article and sent the link to {person}. The guide covers {topic} with benchmarks and practical examples."))

    msgs.append(usr("Save the key points to my notes"))
    c3 = tc("memory_store", {"key": f"notes_{topic.replace(' ','_')}", "content": f"Guide on {topic}: benchmarks show significant improvements. Key patterns and examples included.", "tags": ["research", topic.split()[0]]})
    msgs.append(asst_tool([c3]))
    msgs.append(tool_resp(c3["id"], {"stored": True}))
    msgs.append(asst("Saved the key points to memory."))

    return msgs, ["web_search", "web_fetch", "email_send", "memory_store"]

def _mixed_calendar_memory(i, bond):
    msgs = []
    person = random.choice(PEOPLE)
    topic = random.choice(MTOPICS)
    date = rdate()

    msgs.append(usr(f"What do I need to prep for my meeting with {person}?"))
    c1 = tc("memory_recall", {"query": f"meeting {person}"})
    msgs.append(asst_tool([c1]))
    msgs.append(tool_resp(c1["id"], {"found": True, "key": f"meeting_notes_{person.lower()}", "content": f"Last met {person} on Feb 15. Discussed project timeline. Action item: send updated estimates."}))
    msgs.append(asst(f"From your last meeting with {person} (Feb 15): you discussed the project timeline. Your action item was to send updated estimates."))

    msgs.append(usr("When is the next meeting?"))
    c2 = tc("calendar_list", {"date": date, "query": person})
    msgs.append(asst_tool([c2]))
    evid = random.randint(100, 999)
    msgs.append(tool_resp(c2["id"], {"events": [{"id": evid, "title": f"{topic} with {person}", "start": f"{date}T10:00", "end": f"{date}T10:30"}]}))
    msgs.append(asst(f"Next meeting: '{topic} with {person}' on {date} at 10:00."))

    msgs.append(usr("Update my notes that I sent the estimates"))
    c3 = tc("memory_store", {"key": f"meeting_notes_{person.lower()}", "content": f"Last met {person} on Feb 15. Discussed project timeline. Estimates sent March 10. Next: {topic} on {date}.", "tags": ["meetings", person.lower()]})
    msgs.append(asst_tool([c3]))
    msgs.append(tool_resp(c3["id"], {"stored": True}))
    msgs.append(asst(f"Updated your meeting notes for {person}. Marked estimates as sent."))

    return msgs, ["memory_recall", "calendar_list", "memory_store"]

def _mixed_browser_file(i, bond):
    msgs = []
    fname = random.choice(["config.yaml", "docker-compose.yml", "Dockerfile", "nginx.conf"])
    url = f"https://docs.example.com/{fname.replace('.', '-')}-reference"

    msgs.append(usr(f"I need help configuring {fname}. Find the docs"))
    c1 = tc("web_search", {"query": f"{fname} configuration reference"})
    msgs.append(asst_tool([c1]))
    msgs.append(tool_resp(c1["id"], {"results": [{"title": f"{fname} Configuration Reference", "url": url, "snippet": f"Complete reference for {fname}..."}]}))
    msgs.append(asst(f"Found the official reference: '{fname} Configuration Reference'."))

    msgs.append(usr("Open it and show me my current config"))
    c2a = tc("web_fetch", {"url": url})
    c2b = tc("file_read", {"path": f"/home/user/Projects/{fname}"})
    msgs.append(asst_tool([c2a, c2b]))
    msgs.append(tool_resp(c2a["id"], {"title": f"{fname} Reference", "text": f"Key settings: port (default 8080), workers (default 4), log_level (info/debug/warn)."}))
    msgs.append(tool_resp(c2b["id"], {"content": f"port: 8080\nworkers: 2\nlog_level: info\n"}))
    msgs.append(asst(f"The docs say default workers is 4 but your config has 2. Current config:\n\n```\nport: 8080\nworkers: 2\nlog_level: info\n```"))

    msgs.append(usr("Bump workers to 4 and set log_level to debug"))
    c3 = tc("file_write", {"path": f"/home/user/Projects/{fname}", "content": "port: 8080\nworkers: 4\nlog_level: debug\n"})
    msgs.append(asst_tool([c3]))
    msgs.append(tool_resp(c3["id"], {"written": True, "bytes": 42}))
    msgs.append(asst(f"Updated {fname}. Workers set to 4, log_level set to debug."))

    return msgs, ["web_search", "web_fetch", "file_read", "file_write"]

def gen_recipe_conversations(n=30):
    """Recipe creation via conversation: discuss -> refine -> create"""
    out = []
    recipe_ideas = [
        ("morning_briefing", "A recipe that checks email, calendar, and weather every morning and gives a summary", ["email_list", "calendar_list", "weather_get"]),
        ("backup_monitor", "Watch disk usage and alert if any partition goes above 90%", ["disk_usage", "notify"]),
        ("standup_prep", "Before standup, gather git commits, open PRs, and calendar for today", ["shell_exec", "web_fetch", "calendar_list"]),
        ("inbox_zero", "Auto-archive old read emails and flag important ones based on sender", ["email_list", "email_archive", "email_flag"]),
        ("focus_mode", "Kill distracting processes, set DND, and start a pomodoro timer", ["process_kill", "notification_dnd", "timer"]),
        ("project_snapshot", "Collect git status, open issues, and deployment status for a project", ["shell_exec", "web_fetch", "file_read"]),
        ("evening_wind_down", "Dim screen, play ambient music, set tomorrow's alarm", ["shell_exec", "music_play", "set_reminder"]),
        ("meeting_prep", "Before a meeting, pull up notes on attendees and relevant docs", ["memory_recall", "calendar_list", "file_search"]),
        ("security_check", "Run system updates check, verify firewall, scan open ports", ["shell_exec", "service_status", "network_check"]),
        ("weekly_review", "Summarize the week's calendar, completed tasks, and key emails", ["calendar_list", "memory_recall", "email_list"]),
    ]
    for i in range(n):
        bond = BKEYS[i % 4]
        msgs = [sys(bond)]
        name, desc, tools_needed = recipe_ideas[i % len(recipe_ideas)]

        # Turn 1: user describes what they want
        start_msgs = [
            f"I want to create a recipe that {desc.lower()[:80]}",
            f"Can you help me build an automation? I need something that {desc.lower()[:80]}",
            f"Let's make a recipe. Idea: {desc[:80]}",
            f"I keep doing this manually: {desc.lower()[:80]}. Automate it for me",
        ]
        msgs.append(usr(random.choice(start_msgs)))
        msgs.append(asst(f"Good idea. I'd call it '{name}'. It would use {', '.join(tools_needed[:2])} and {tools_needed[-1]}. Want me to set up the steps?"))

        # Turn 2: user confirms/modifies
        confirm_msgs = ["Yes, do it", "Sounds right. Go ahead", "Perfect. Create it", "Add a notification at the end too", "Yes, but also save results to memory"]
        msgs.append(usr(random.choice(confirm_msgs)))

        steps = [{"tool": t, "description": f"Step {j+1}: call {t}"} for j, t in enumerate(tools_needed)]
        c2 = tc("recipe_create", {"name": name, "description": desc, "steps": steps, "trigger": random.choice(["manual", "schedule:daily:09:00", "schedule:weekly:mon:08:00", "on_event:system_boot"])})
        msgs.append(asst_tool([c2]))
        msgs.append(tool_resp(c2["id"], {"created": True, "recipe_id": f"recipe_{random.randint(100,999)}", "name": name}))
        msgs.append(asst(f"Recipe '{name}' created with {len(steps)} steps. Trigger set."))

        # Turn 3: test it
        test_msgs = ["Run it now to test", "Let's try it", "Test it out", "Execute it once", "Dry run please"]
        msgs.append(usr(random.choice(test_msgs)))
        c3 = tc("recipe_execute", {"name": name, "dry_run": "dry" in test_msgs[i % len(test_msgs)].lower()})
        msgs.append(asst_tool([c3]))
        step_results = [{"tool": t, "status": "ok", "output": f"Executed {t} successfully"} for t in tools_needed]
        msgs.append(tool_resp(c3["id"], {"executed": True, "results": step_results, "duration_ms": random.randint(500, 5000)}))
        msgs.append(asst(f"Recipe ran successfully. All {len(steps)} steps completed in {random.randint(1,5)}s."))

        # Optional turn 4: modify
        if i % 3 == 0:
            msgs.append(usr("Add a step to notify me when it finishes"))
            c4 = tc("recipe_update", {"name": name, "add_step": {"tool": "notify", "description": "Send completion notification"}})
            msgs.append(asst_tool([c4]))
            msgs.append(tool_resp(c4["id"], {"updated": True}))
            msgs.append(asst(f"Added notification step to '{name}'. You'll get pinged when it completes."))
            out.append(make(msgs, bond, "recipe_conversation", ["recipe_create", "recipe_execute", "recipe_update"]))
        else:
            out.append(make(msgs, bond, "recipe_conversation", ["recipe_create", "recipe_execute"]))
    return out

# ===================================================================
# MAIN
# ===================================================================
if __name__ == "__main__":
    all_examples = []
    all_examples.extend(gen_email_workflows(40))
    all_examples.extend(gen_file_workflows(40))
    all_examples.extend(gen_research_workflows(40))
    all_examples.extend(gen_system_troubleshoot(30))
    all_examples.extend(gen_calendar_workflows(30))
    all_examples.extend(gen_browser_workflows(30))
    all_examples.extend(gen_memory_workflows(30))
    all_examples.extend(gen_mixed_workflows(30))
    all_examples.extend(gen_recipe_conversations(30))

    random.shuffle(all_examples)

    with open(OUT, "w", encoding="utf-8") as f:
        for ex in all_examples:
            f.write(ex + "\n")

    print(f"Wrote {len(all_examples)} examples to {OUT}")

    # Validate
    with open(OUT, "r", encoding="utf-8") as f:
        lines = f.readlines()
    turns = []
    tools_seen = set()
    for line in lines:
        obj = json.loads(line)
        msgs = obj["messages"]
        user_turns = sum(1 for m in msgs if m["role"] == "user")
        turns.append(user_turns)
        for m in msgs:
            if m.get("tool_calls"):
                for tc_item in m["tool_calls"]:
                    tools_seen.add(tc_item["function"]["name"])

    print(f"Turn distribution: min={min(turns)}, max={max(turns)}, avg={sum(turns)/len(turns):.1f}")
    print(f"Unique tools: {len(tools_seen)} -- {sorted(tools_seen)}")
    bond_dist = {}
    scenario_dist = {}
    for line in lines:
        obj = json.loads(line)
        b = obj["metadata"]["bond_stage"]
        s = obj["metadata"]["scenario"]
        bond_dist[b] = bond_dist.get(b, 0) + 1
        scenario_dist[s] = scenario_dist.get(s, 0) + 1
    print(f"Bond distribution: {bond_dist}")
    print(f"Scenario distribution: {scenario_dist}")
