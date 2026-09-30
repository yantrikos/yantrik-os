#!/usr/bin/env python3
"""The third sweep (2026-09-28): computer use and screen understanding, and compression of
weights, KV cache, context and tool schemas. Skips papers the first two sweeps scored.
Writes papers3.jsonl."""
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))

import json
import subprocess
import time
import urllib.parse
import xml.etree.ElementTree as ET

NS = {"a": "http://www.w3.org/2005/Atom"}
QUERIES = {
    # A. Computer use: seeing the screen and acting on it.
    "cua_agents": '(abs:"computer use" OR abs:"computer-use agent") OR (abs:"GUI agent" AND abs:desktop)',
    "screen_parsing": 'abs:"screen parsing" OR abs:"screen understanding" OR abs:OmniParser OR abs:"UI element detection"',
    "gui_grounding": 'abs:"GUI grounding" OR abs:ScreenSpot OR (abs:"visual grounding" AND abs:GUI)',
    "ocr_models": 'abs:OCR AND (abs:"vision-language" OR abs:lightweight OR abs:"end-to-end" OR abs:screenshot)',
    "set_of_marks": '(abs:"set-of-mark" OR abs:"visual prompting" OR abs:"marked elements") AND (abs:GUI OR abs:web OR abs:screen)',
    "visual_tokens": 'abs:"visual token" AND (abs:pruning OR abs:compression OR abs:reduction OR abs:merging)',
    "hires_screens": '(abs:"high-resolution" OR abs:zoom OR abs:crop) AND (abs:"GUI agent" OR abs:screenshot)',
    "a11y_hybrid": '(abs:"accessibility tree" OR abs:"UI tree" OR abs:"view hierarchy") AND (abs:screenshot OR abs:multimodal)',
    "gui_verification": 'abs:"GUI agent" AND (abs:verification OR abs:reflection OR abs:"error recovery" OR abs:"world model")',
    "small_gui_models": '(abs:small OR abs:lightweight OR abs:efficient OR abs:"on-device") AND (abs:"GUI agent" OR abs:"mobile agent")',
    "gui_safety": '(abs:"GUI agent" OR abs:"computer use") AND (abs:safety OR abs:injection OR abs:attack OR abs:pop-up)',
    # B. Compression: weights, KV cache, context, tool schemas.
    "weight_quant": 'abs:quantization AND abs:"language model" AND (abs:"2-bit" OR abs:"3-bit" OR abs:"4-bit" OR abs:"low-bit")',
    "kv_compression": 'abs:"KV cache" AND (abs:compression OR abs:eviction OR abs:quantization OR abs:pruning)',
    "kv_reuse": 'abs:"KV cache" AND (abs:reuse OR abs:sharing OR abs:"prefix caching" OR abs:offload* OR abs:"cache blending")',
    "prompt_compression": 'abs:"prompt compression" OR abs:"context compression" OR abs:"gist tokens"',
    "tool_schema_tokens": '(abs:tool OR abs:function) AND (abs:schema OR abs:description OR abs:documentation) AND (abs:compress* OR abs:"token cost" OR abs:"context length")',
    "agent_context": '(abs:"context management" OR abs:"context engineering" OR abs:"memory compression") AND (abs:agent OR abs:"LLM agent")',
    "speculative": 'abs:"speculative decoding" AND (abs:efficient OR abs:agent OR abs:"consumer" OR abs:"small")',
    "structured_decoding": '(abs:"constrained decoding" OR abs:"structured output" OR abs:"grammar-constrained") AND abs:LLM',
    "moe_offload": '(abs:"mixture-of-experts" OR abs:MoE) AND (abs:offload* OR abs:"consumer GPU" OR abs:"expert caching")',
}
PER_QUERY = 40

done = set()
for name in ("papers.jsonl", "papers2.jsonl"):
    path = os.path.join(DATA, name)
    if os.path.exists(path):
        done |= {json.loads(l)["id"].split("v")[0] for l in open(path, encoding="utf-8")}


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

with open(os.path.join(DATA, "papers3.jsonl"), "w", encoding="utf-8") as f:
    for p in seen.values():
        f.write(json.dumps(p) + "\n")
print("new papers:", len(seen))
