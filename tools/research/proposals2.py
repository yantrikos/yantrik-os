#!/usr/bin/env python3
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")  # F:/yantrik/bin/jev on the build workstation; reads JEV_API_KEY

import concurrent.futures as cf, json, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score import SYSTEM
from proposals import QUESTIONS

PROPOSALS = {
    "approval_closure": "Approvals cover the transitive effects of what they approve, not only the entry call: a card for 'install package' or an MCP call names the effect classes its workflow can exercise (files written, network reached, hooks run), and the record binds to that closure (Agent Approval Laundering, arXiv 2609.28586).",
    "recoverable_execution": "Checkpoint an agent's context and the app state it touches at each step so a long task can rewind to before a mistaken step instead of compounding it; ties to filesystem snapshots (AgentRewind, arXiv 2608.14380).",
    "forget_derived": "Forgetting purges what was derived from the forgotten item too: summaries, beliefs consolidated from it, pending plans, cached context, so the mind behaves as if it never saw it (Execution-State Unlearning, arXiv 2609.04875).",
    "memory_provenance_firewall": "Memory consolidation may never raise an item's authority: an observation from a web page stays web-origin after it is summarised into 'user history' or a workflow, and recalled low-authority items cannot trigger acts (Provenance Laundering firewall, arXiv 2607.29167; DualView stored IPI, arXiv 2607.03821).",
    "explicit_control_state": "For local models, keep the task's stage, accumulated evidence and runtime feedback as explicit structured state the loop maintains and shows the model each step, instead of leaving it implicit in a growing transcript (LocalLSTC, arXiv 2608.25777: Qwen3.5-9B lost 23 points on OSWorld without it).",
    "no_false_success_gate": "An unattended task may only claim 'done' when a falsifiable gate for its goal ran and passed; otherwise it reports what is unverified (Goal-Autopilot, arXiv 2606.11688, No-False-Success theorem).",
    "deterministic_harness": "Wrap small models in a deterministic execution layer: finite-state control, forced tool choice where the next step is known, output validation and bounded retry (Harness Engineering, arXiv 2608.26197).",
    "skill_boundaries": "Recipes and learned skills carry the boundary where they stop applying (learned from failures too), so a task that resembles a past success but needs different tools is not pushed down the old path (arXiv 2608.22339).",
}

with cf.ThreadPoolExecutor(8) as pool:
    def ask(item):
        name, text = item
        req = {"state": {"system": SYSTEM, "proposal": text}, "questions": QUESTIONS}
        out = subprocess.run([sys.executable, JEV], input=json.dumps(req), capture_output=True, text=True, timeout=120)
        return name, json.loads(out.stdout)["answers"]
    results = dict(pool.map(ask, PROPOSALS.items()))
rows = []
for name, a in results.items():
    i, e, o = a["impact"]["score"], a["effort"]["score"], a["overlap"]["noul"]
    rows.append(((i * (1 - 0.5 * o)) / (1 + e), i, e, o, name))
for r in sorted(rows, reverse=True):
    print(f"{r[0]:.2f}  impact {r[1]:.2f}  effort {r[2]:.2f}  covered {r[3]:.2f}  {r[4]}")
json.dump({k: PROPOSALS[k] for k in PROPOSALS}, open(DATA + "/proposals2.json", "w"), indent=1)
