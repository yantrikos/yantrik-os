#!/usr/bin/env python3
"""Fetch recent arXiv papers for the areas Yantrik OS works in, as JSON lines."""
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")  # F:/yantrik/bin/jev on the build workstation; reads JEV_API_KEY

import json
import time
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

NS = {"a": "http://www.w3.org/2005/Atom"}
QUERIES = {
    "agent_security": 'abs:"prompt injection" AND (abs:agent OR abs:agents)',
    "agent_capabilities": '(abs:"capability" OR abs:"information flow" OR abs:"taint") AND abs:"LLM agent"',
    "agent_memory": '(abs:"long-term memory" OR abs:"agent memory" OR abs:"memory system") AND (abs:LLM OR abs:agent)',
    "memory_privacy": '(abs:memory) AND (abs:privacy OR abs:isolation OR abs:"access control") AND abs:LLM',
    "os_agents": '(abs:"computer use" OR abs:"OS agent" OR abs:"GUI agent" OR abs:"desktop agent")',
    "agent_reliability": '(abs:"agent" AND (abs:"self-verification" OR abs:"stall" OR abs:"loop detection" OR abs:"task completion" OR abs:"false claims"))',
    "small_model_tools": '(abs:"tool calling" OR abs:"function calling") AND (abs:"small language model" OR abs:"small models")',
}
PER_QUERY = 25


def fetch(query):
    params = urllib.parse.urlencode({
        "search_query": query, "start": 0, "max_results": PER_QUERY,
        "sortBy": "submittedDate", "sortOrder": "descending",
    })
    import subprocess
    body = subprocess.run(["curl", "-s", "--max-time", "60", "https://export.arxiv.org/api/query?" + params],
                          capture_output=True, check=True).stdout
    root = ET.fromstring(body)
    for e in root.findall("a:entry", NS):
        yield {
            "id": e.findtext("a:id", default="", namespaces=NS).rsplit("/", 1)[-1],
            "title": " ".join(e.findtext("a:title", default="", namespaces=NS).split()),
            "abstract": " ".join(e.findtext("a:summary", default="", namespaces=NS).split()),
            "published": e.findtext("a:published", default="", namespaces=NS)[:10],
        }


seen = {}
for area, q in QUERIES.items():
    try:
        for p in fetch(q):
            key = p["id"].split("v")[0]
            if key not in seen:
                p["area"] = area
                seen[key] = p
        print(area, "ok", flush=True)
    except Exception as e:  # keep going; one area failing is not the run failing
        print(area, "failed:", e, flush=True)
    time.sleep(3)  # arXiv asks for a pause between calls

with open(DATA + "/papers.jsonl", "w", encoding="utf-8") as f:
    for p in seen.values():
        f.write(json.dumps(p) + "\n")
print("papers:", len(seen))
