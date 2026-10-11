#!/usr/bin/env python3
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")  # F:/yantrik/bin/jev on the build workstation; reads JEV_API_KEY

import concurrent.futures as cf, json, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score import SYSTEM

PROPOSALS = {
    "tool_schema_compiler": "Compile each app's JSON action schemas into token-efficient structured text before handing them to small/local models (TSCG, arXiv 2605.04107: Phi-4 14B from 0% to 84% tool accuracy at 20 tools). Applies to yos-mcp and the companion's tool prompts; deterministic, no model changes.",
    "effect_contracts": "Each control-surface action declares its intended effect and the evidence that proves it (e.g. save_as -> file exists with this content hash; add_event -> event id present), and the surface verifies it after acting, answering verified/unverified (ADF-EA capability contracts, arXiv 2609.30691). Turns false 'done' claims into checked facts.",
    "overclaim_audit": "At the end of every agent turn the desktop compares what the final reply claims was done against the turn's recorded actions and their verified effects, and marks unsupported claims to the person (OverclaimBench, arXiv 2609.20812).",
    "verifiable_action_card": "Harden approval cards per the Verifiable Action Card (arXiv 2609.18411): build the card only from the ground-truth pending action (never model text), show provenance of the triggering content, default-deny, and re-verify the exact action at dispatch.",
    "dormant_injection_taint": "Carry taint across time: content an agent read from the web, email or files stays labelled in memory and later turns, so a dormant conditional injection (arXiv 2609.22510, 16-34% success on production agents) that fires later is still gated as untrusted-origin.",
    "memory_admission": "Admission control for memory writes: a claim repeated or paraphrased by the same source lineage is not new evidence; writes from turns that read untrusted content are held as unverified (Epistemic admission, arXiv 2609.30813; scope-before-persist, 2609.29144).",
    "step_level_escalation": "When a small local model is mid-task and the step looks risky or it is stalling, escalate that one step to a stronger model, then return (R2V step-level routing, arXiv 2605.16604).",
    "ifc_egress": "Information-flow control for disclosure: data from the person's memory or files carries a label, and sending it out (email, web form, another mind) is checked by code against the label and recipient, never by the model's judgment (arXiv 2609.14003).",
}

QUESTIONS = {
    "impact": {"type": "score", "instructions": "How much would this proposal improve Yantrik OS for its person, given the system and its known weak spots?",
               "criteria": ["none", "small", "clear improvement to one area", "large: fixes a known weak spot", "very large: changes how trustworthy the system is"]},
    "effort": {"type": "score", "instructions": "How much work is this proposal for this system as described?",
               "criteria": ["a day or two", "about a week", "a few weeks", "a month or more", "a quarter or more"]},
    "overlap": {"type": "noul", "instructions": "Is this mostly already covered by what the system description says exists (tokens, approval cards, path policies, grants, planned taint)?"},
}


def ask(item):
    name, text = item
    req = {"state": {"system": SYSTEM, "proposal": text}, "questions": QUESTIONS}
    out = subprocess.run([sys.executable, JEV], input=json.dumps(req), capture_output=True, text=True, timeout=120)
    return name, json.loads(out.stdout)["answers"]


with cf.ThreadPoolExecutor(8) as pool:
    results = dict(pool.map(ask, PROPOSALS.items()))
rows = []
for name, a in results.items():
    impact, effort, overlap = a["impact"]["score"], a["effort"]["score"], a["overlap"]["noul"]
    rows.append(((impact * (1 - 0.5 * overlap)) / (1 + effort), impact, effort, overlap, name))
for r in sorted(rows, reverse=True):
    print(f"{r[0]:.2f}  impact {r[1]:.2f}  effort {r[2]:.2f}  covered {r[3]:.2f}  {r[4]}")
