#!/usr/bin/env python3
"""Ask Jev, per paper, whether and where it could improve Yantrik OS. Writes scored.jsonl."""
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")  # F:/yantrik/bin/jev on the build workstation; reads JEV_API_KEY

import concurrent.futures as cf
import json
import subprocess
import sys

SYSTEM = (
    "Yantrik OS: a Linux desktop OS (Rust, Slint UI) whose apps each publish a typed control surface "
    "(describe state as JSON, act by named actions with risk grades, revisions to refuse stale acts), "
    "driven by AI minds instead of screenshots. Minds: a first-party 'Yantrik Mind' agent (own system "
    "account, sandboxed, keeps a YantrikDB memory of the person: beliefs, memories, scopes, sensitivity "
    "classes) and third-party harnesses (Hermes, Pi, OpenClaw, DeepSeek) attached over a socket. "
    "Security: kernel-verified caller identity (SO_PEERCRED), per-agent tokens, approval cards for "
    "sensitive/dangerous acts, mind modes (ask/auto/bypass), path policies keeping agents in the home "
    "folder, per-mind memory grants validated per call, taint/plan checks planned. Models are often "
    "small or local (4B-27B via Ollama) as well as cloud. Known weak spots: small models loop or "
    "describe steps instead of acting, false 'done' claims, prompt injection from web pages and files, "
    "memory privacy between minds and people, long-running agent tasks."
)

QUESTIONS = {
    "value": {
        "type": "score",
        "instructions": "How much could adopting this paper's main idea improve Yantrik OS as described in `system`?",
        "criteria": [
            "none: unrelated to anything the system does",
            "slight: related area, but nothing the system would change",
            "moderate: a useful idea for one part of the system",
            "strong: would clearly fix a known weak spot or harden a boundary",
            "major: a technique the system should adopt; directly addresses a stated weak spot with evidence",
        ],
    },
    "area": {
        "type": "choice",
        "instructions": "Which part of Yantrik OS would this paper's idea most improve?",
        "criteria": {
            "agent_security": "agent security: prompt injection, capabilities, information flow, approvals",
            "memory": "the person's long-term memory: structure, retrieval, privacy, isolation between minds",
            "agent_reliability": "agent reliability: loops, stalls, false completion claims, verification, planning",
            "control_surface": "how agents perceive and operate apps and the desktop (structured state vs screenshots)",
            "small_models": "getting small or local models to call tools and finish tasks",
            "none": "none of these",
        },
    },
    "actionable": {
        "type": "noul",
        "instructions": "Does the paper propose a concrete technique (not only a benchmark, survey or position) that this system could implement in a few weeks?",
    },
    "evidence": {
        "type": "noul",
        "instructions": "Does the abstract report a measured improvement over a baseline for its technique?",
    },
}


def ask(paper):
    request = {
        "state": {"system": SYSTEM, "paper": {"title": paper["title"], "abstract": paper["abstract"]}},
        "questions": QUESTIONS,
    }
    out = subprocess.run([sys.executable, JEV], input=json.dumps(request),
                         capture_output=True, text=True, timeout=120)
    if out.returncode != 0:
        return {**paper, "error": out.stderr[-300:]}
    answers = json.loads(out.stdout).get("answers", {})
    return {**paper, "jev": answers}


if __name__ == "__main__":
    papers = [json.loads(l) for l in open(DATA + "/papers.jsonl", encoding="utf-8")]
    if len(sys.argv) > 1:
        papers = papers[: int(sys.argv[1])]
    with cf.ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(ask, papers))
    with open(DATA + "/scored.jsonl", "w", encoding="utf-8") as f:
        for r in results:
            f.write(json.dumps(r) + "\n")
    errors = [r for r in results if "error" in r]
    print("scored:", len(results) - len(errors), "errors:", len(errors))
    if errors:
        print(errors[0]["error"])
