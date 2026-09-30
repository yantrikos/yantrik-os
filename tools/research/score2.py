#!/usr/bin/env python3
"""Score the wider sweep with Jev: the same questions, with the areas the new queries reach."""
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")  # F:/yantrik/bin/jev on the build workstation; reads JEV_API_KEY

import concurrent.futures as cf
import json
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import score

score.QUESTIONS["area"]["criteria"] = {
    "agent_security": "agent security: prompt injection, capabilities, sandboxing, information flow, approvals",
    "memory": "the person's long-term memory: structure, consolidation, retrieval, privacy, isolation between minds",
    "agent_reliability": "agent reliability: loops, stalls, false completion claims, verification, planning, uncertainty",
    "control_surface": "how agents perceive and operate apps and the desktop (structured state, GUI grounding)",
    "small_models": "small or local models: tool calling, distillation, efficient on-device inference",
    "tools": "tool selection and retrieval, skill libraries, MCP",
    "assistant_ux": "the person's experience: proactive help, voice, human oversight and approvals",
    "none": "none of these",
}

papers = [json.loads(l) for l in open(DATA + "/papers2.jsonl", encoding="utf-8")]
with cf.ThreadPoolExecutor(max_workers=10) as pool:
    results = list(pool.map(score.ask, papers))
with open(DATA + "/scored2.jsonl", "w", encoding="utf-8") as f:
    for r in results:
        f.write(json.dumps(r) + "\n")
errors = [r for r in results if "error" in r]
print("scored:", len(results) - len(errors), "errors:", len(errors))
