#!/usr/bin/env python3
"""Score the third sweep with Jev against what Yantrik OS does today for computer use and for
compression, and against Pranab's own tools, so an idea already built is not rediscovered.
Writes scored3.jsonl."""
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")

import concurrent.futures as cf
import json
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score import SYSTEM as BASE

SYSTEM = BASE + (
    " COMPUTER USE TODAY: apps publish typed describe/act surfaces, so minds rarely need pixels; for "
    "windows with no surface (third-party apps, games, web pages without the browser bridge) there is "
    "no OCR, no crops, no element ids with bounds, and no screenshot tool for minds; an AT-SPI "
    "accessibility service lists elements without coordinates and is off by default; planned (#257): "
    "a compact text scene of the screen, then element ids+bounds, crops on request, OCR last. The "
    "browser bridge gives numbered element refs. HARDWARE: a host with 2x RTX 3090 Ti (24 GB each) "
    "serving Ollama/llama.cpp, and desktops/VMs that are often CPU-only. OWN TOOLS ALREADY BUILT: "
    "ContextCache (tool schemas compiled into a reusable KV cache on disk, TTFT ~200ms at 50 tools vs "
    "5.6s), ToolFormerMicro (tool schemas compressed to 8 gist tokens via gated cross-attention), Tool "
    "Context Distillation (tool schemas distilled into weights with QLoRA), yantrik-inference (many "
    "typed decisions read from one batched forward pass over a shared prefix, 8-bit KV cache), Tier "
    "(adaptive tool routing by model size), and a published cost model for prefix-cache "
    "fragmentation in multi-tenant tool use."
)

QUESTIONS = {
    "value": {
        "type": "score",
        "instructions": "How much could adopting this paper's main idea improve the system in `system`, "
                        "given what it and its own tools already do?",
        "criteria": [
            "none: unrelated to anything the system does",
            "slight: related area, but nothing the system would change",
            "moderate: a useful idea for one part of the system",
            "strong: fills a stated gap (computer use without surfaces, or cost/latency on the stated hardware)",
            "major: a technique the system should adopt now; fills a stated gap with measured evidence",
        ],
    },
    "area": {
        "type": "choice",
        "instructions": "Which part would this paper's idea most improve?",
        "criteria": {
            "screen_understanding": "turning a window's pixels or UI tree into something an agent can read: OCR, screen parsing, accessibility",
            "gui_grounding": "locating and acting on the right element: coordinates, element ids, action verification, recovery",
            "vision_efficiency": "making screenshots cheaper for models: fewer visual tokens, crops, resolution",
            "gui_safety": "keeping computer-use agents safe: pop-ups, injected screen text, unsafe clicks",
            "weight_compression": "quantizing or pruning model weights to fit or speed up the stated hardware",
            "kv_context": "KV cache compression or reuse, prompt or context compression, tool-schema tokens",
            "decoding": "faster or more reliable decoding: speculative, constrained or structured output",
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
    "covered": {
        "type": "noul",
        "instructions": "Is this paper's core technique already covered by the system's own tools listed in `system` (ContextCache, ToolFormerMicro, Tool Context Distillation, yantrik-inference, Tier) or by what it does today?",
    },
    "fits_hardware": {
        "type": "noul",
        "instructions": "Could this technique run on the stated hardware (two 24 GB consumer GPUs, or CPU-only desktops) without datacenter GPUs or retraining a large model from scratch?",
    },
}


def ask(paper):
    request = {
        "state": {"system": SYSTEM, "paper": {"title": paper["title"], "abstract": paper["abstract"]}},
        "questions": QUESTIONS,
    }
    out = subprocess.run([sys.executable, JEV], input=json.dumps(request),
                         capture_output=True, text=True, timeout=180)
    if out.returncode != 0:
        return {**paper, "error": out.stderr[-300:]}
    return {**paper, "jev": json.loads(out.stdout).get("answers", {})}


if __name__ == "__main__":
    papers = [json.loads(l) for l in open(os.path.join(DATA, "papers3.jsonl"), encoding="utf-8")]
    with cf.ThreadPoolExecutor(max_workers=10) as pool:
        results = list(pool.map(ask, papers))
    with open(os.path.join(DATA, "scored3.jsonl"), "w", encoding="utf-8") as f:
        for r in results:
            f.write(json.dumps(r) + "\n")
    errors = [r for r in results if "error" in r]
    print("scored:", len(results) - len(errors), "errors:", len(errors))
    if errors:
        print(errors[0]["error"])
    # Ranked: value, held down by what is already built, lifted by evidence and a fit to the hardware.
    rows = []
    for r in results:
        a = r.get("jev")
        if not a:
            continue
        v = a["value"]["score"]
        rank = v * (0.6 + 0.4 * a["actionable"]["noul"]) * (0.7 + 0.3 * a["evidence"]["noul"]) \
            * (1 - 0.6 * a["covered"]["noul"]) * (0.5 + 0.5 * a["fits_hardware"]["noul"])
        rows.append({**r, "rank": round(rank, 4), "area_jev": a["area"].get("choice")})
    rows.sort(key=lambda r: r["rank"], reverse=True)
    json.dump(rows, open(os.path.join(DATA, "ranked3.json"), "w", encoding="utf-8"), indent=1)
    for r in rows[:40]:
        print(f'{r["rank"]:.3f}  {r["area_jev"]:<22} {r["id"]:<14} {r["title"][:90]}')
