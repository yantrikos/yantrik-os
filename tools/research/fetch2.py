#!/usr/bin/env python3
"""The wider sweep: more areas, more papers, skipping any already scored. Writes papers2.jsonl."""
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")  # F:/yantrik/bin/jev on the build workstation; reads JEV_API_KEY

import json
import subprocess
import time
import urllib.parse
import xml.etree.ElementTree as ET

NS = {"a": "http://www.w3.org/2005/Atom"}
QUERIES = {
    "ipi_defense": 'abs:"indirect prompt injection" AND (abs:defense OR abs:defence OR abs:mitigat*)',
    "agent_sandbox": '(abs:sandbox OR abs:isolation OR abs:"least privilege") AND abs:"LLM agent"',
    "human_oversight": '(abs:"human-in-the-loop" OR abs:"human oversight" OR abs:approval OR abs:confirmation) AND abs:"agent"',
    "memory_consolidation": '(abs:"memory consolidation" OR abs:forgetting OR abs:"episodic memory") AND (abs:"LLM agent" OR abs:"language agent")',
    "proactive_assistant": '(abs:proactive OR abs:"personal assistant") AND (abs:LLM OR abs:agent)',
    "ui_structured": '(abs:"accessibility tree" OR abs:"structured UI" OR abs:"UI representation" OR abs:"API-based agent")',
    "gui_grounding": 'abs:"GUI grounding" OR (abs:"GUI agent" AND abs:grounding)',
    "tool_retrieval": '(abs:"tool retrieval" OR abs:"tool selection" OR abs:"tool learning") AND abs:LLM',
    "ondevice_inference": '(abs:"on-device" OR abs:"edge device" OR abs:"consumer GPU") AND abs:"language model" AND (abs:inference OR abs:latency)',
    "agent_distillation": '(abs:distillation OR abs:"trajectory") AND abs:"small language model" AND abs:agent',
    "skill_library": '(abs:"skill library" OR abs:"procedural memory" OR abs:"workflow memory" OR abs:"reusable skills") AND abs:agent',
    "multi_agent_orchestration": '(abs:orchestration OR abs:"multi-agent") AND abs:LLM AND (abs:reliab* OR abs:coordination)',
    "voice_agents": '(abs:"voice assistant" OR abs:"spoken dialogue" OR abs:"speech agent") AND abs:LLM',
    "personal_data_privacy": '(abs:"personal data" OR abs:"contextual integrity") AND abs:LLM AND abs:privacy',
    "agent_uncertainty": '(abs:"uncertainty" OR abs:calibration OR abs:abstention) AND abs:"LLM agent"',
    "mcp": 'abs:"Model Context Protocol"',
    "desktop_benchmarks": '(abs:OSWorld OR abs:"desktop environment" OR abs:"operating system") AND abs:agent AND abs:benchmark',
    "planning_long_horizon": '(abs:"long-horizon" OR abs:"task decomposition") AND abs:"LLM agent" AND abs:planning',
}
PER_QUERY = 40

done = {json.loads(l)["id"].split("v")[0] for l in open(DATA + "/papers.jsonl", encoding="utf-8")}


def fetch(query):
    params = urllib.parse.urlencode({"search_query": query, "start": 0, "max_results": PER_QUERY,
                                     "sortBy": "submittedDate", "sortOrder": "descending"})
    body = subprocess.run(["curl", "-s", "--max-time", "60", "https://export.arxiv.org/api/query?" + params],
                          capture_output=True, check=True).stdout
    for e in ET.fromstring(body).findall("a:entry", NS):
        yield {
            "id": e.findtext("a:id", default="", namespaces=NS).rsplit("/", 1)[-1],
            "title": " ".join(e.findtext("a:title", default="", namespaces=NS).split()),
            "abstract": " ".join(e.findtext("a:summary", default="", namespaces=NS).split()),
            "published": e.findtext("a:published", default="", namespaces=NS)[:10],
        }


seen = {}
for area, q in QUERIES.items():
    try:
        n = 0
        for p in fetch(q):
            key = p["id"].split("v")[0]
            if key not in seen and key not in done:
                p["area"] = area
                seen[key] = p
                n += 1
        print(area, n, flush=True)
    except Exception as e:
        print(area, "failed:", e, flush=True)
    time.sleep(3)

with open(DATA + "/papers2.jsonl", "w", encoding="utf-8") as f:
    for p in seen.values():
        f.write(json.dumps(p) + "\n")
print("new papers:", len(seen))
