#!/usr/bin/env python3
"""Rank the third sweep's proposals with Jev: impact, effort, and overlap with what the system and
Pranab's own tools already do. Read by hand from ranked3.json; each names its papers."""
import os
DATA = os.environ.get("RESEARCH_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "data"))
JEV = os.environ.get("JEV_CLI", "jev")

import concurrent.futures as cf, json, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score3 import SYSTEM

QUESTIONS = {
    "impact": {"type": "score", "instructions": "How much would this proposal improve the system for its person, given `system`, its known weak spots and its stated gaps?",
               "criteria": ["none", "small", "clear improvement to one area", "large: fills a stated gap or fixes a known weak spot", "very large: changes what the agents can do or how fast/cheaply they run"]},
    "effort": {"type": "score", "instructions": "How much work is this proposal for this system as described?",
               "criteria": ["a day or two", "about a week", "a few weeks", "a month or more", "a quarter or more"]},
    "overlap": {"type": "noul", "instructions": "Is this mostly already covered by what `system` says exists, including the own tools it lists?"},
}

PROPOSALS = {
    # A. Computer use
    "screen_scene": "`yos screen`: one compact text scene of any window, element ids with boxes and text, fused from the app's own surface, AT-SPI with extents, and for windows with neither, a fast pixel widget detector plus OCR; the mind acts on ids, never raw coordinates (TargetFinder arXiv 2607.19907 beats OmniParser at ms latency; GUI-Lens 2608.03270 shows OCR+detected components as coordinate references lift general VLMs; ScreenParse 2602.14276; Screen2AX 2507.16704).",
    "look_again_crops": "Crop on demand: the mind sees a cheap low-resolution glance plus the scene, and asks for a high-resolution crop of a region only when it needs detail; a small reranker picks the crop so grounding costs one VLM call (RankGround 2609.18690, GapSight 2608.21762, GUI-Lens coarse-to-fine).",
    "screenshot_token_budget": "When a screenshot must go to a VLM, spend tokens where the information is: downsample flat regions and keep text-dense ones sharp (adaptive quadtree), preserving positions; training-free, done before the model sees it (AQuaUI 2605.19260, FocusUI 2601.03928, PACE 2608.27206).",
    "local_ocr_stack": "An OCR stack sized to the hardware: a CPU-friendly detector+recogniser for screen text on every desktop, and a document-parsing VLM on the GPU host for PDFs and scans, served behind one `ocr` surface with boxes (Jina-OCR-v1 2609.03181 on low-budget GPUs; SmolDocling 256M key-value extraction 2608.20868; multilingual STR 2609.24058).",
    "own_grounding_model": "Train a ~2-3B GUI grounding model on our own look for free: harvest element boxes from our Slint apps (testing backend element tree) and web pages (CDP DOM) with no human labels, then self-family distillation on one 3090 Ti (WinDOM 2606.25964, ZonUI-3B 2506.23491).",
    "screen_text_is_untrusted": "Text read off the screen (OCR or AT-SPI of foreign apps and pages) enters the mind as untrusted, tainted content like web pages, and pop-ups or text asking the agent to act cannot trigger acts; red-team with self-improving attacks (SIR 2608.30207, HazardAuditor 2609.15134, BraveGuard 2606.01166).",
    "gui_postconditions": "Every GUI act on a window without a surface declares its expected effect and is verified by re-reading the scene; a mismatch stops the run at the first mistake instead of compounding it (Locating Hidden Failures 2609.17930; conflict-aware termination 2609.03438).",
    # B. Compression
    "prefix_order_optimizer": "Order the reusable prompt pieces (system, role, tool schemas, memory scopes) per request to maximise shared prefixes across minds, roles and tiers, instead of one global order; extends ContextCache and the cache-aware cost model (Prefix Sharing Is a Sorting Problem 2609.13692).",
    "kv_edit_repair": "When a cached tool schema, memory item or document changes, repair the cached KV by recomputing a contiguous window after the edit instead of re-prefilling everything; ContextCache invalidation becomes incremental (Budgeted Repair of Stale KV Caches 2609.17983; KVEraser 2606.17034).",
    "erase_injection_from_kv": "When content in the context is found to be a prompt injection after prefill, erase its span from the KV cache so the rest of the turn is not steered by it, without re-prefilling the suffix (KVEraser 2606.17034).",
    "persistent_session_kv": "Keep a mind's conversation KV resident across turns with agent-phase-aware eviction (think/act/tool), and quantise it (8-bit now, lower later), so a long task is not re-prefilled every turn on the 3090s (AgentKV 2609.14872; SPECTRA 2608.07915; ValueDiff 2609.23314).",
    "typed_compaction": "Compact a mind's context by type: rules, constraints and grants are kept verbatim and replicated, episodes are summarised, bulk goes to retrieval; no safety rule is ever paraphrased away (The Compaction Cliff 2608.22752; What Gist Compression Loses 2608.11775).",
    "failure_record_rewrite": "Small models repeat the exact tool call they just watched fail; the harness replaces the verbatim failed call in the transcript with a short constraint and the error's cause, instead of showing the call again (Feedback That Backfires 2608.23651: repeat probability 0.06 -> 0.54).",
    "local_speculative_decoding": "Speculative decoding on the GPU host with a small same-family draft model (or quantised self-draft), and fast grammar-constrained decoding for tool-call JSON, to cut latency of local 4-27B models (SpecQuant 2609.21704; parser-stack GCD 2608.03065; ASPIRE 2609.17943).",
    "moe_expert_paging": "Run larger MoE models on one 24 GB GPU or CPU-only desktops by paging routed experts (working-set paging, predicted SSD prefetch) instead of whole-layer offload (WiSP 2606.21868; Edge0 2609.18063; tensor-level CPU-GPU scheduling 2607.10183).",
    "sub2bit_weights": "Post-training quantisation below 3 bits (ternary with salient residuals) to fit bigger models on the host GPUs (QTEA 2609.00224; SchurQuant 2608.15567).",
}

with cf.ThreadPoolExecutor(8) as pool:
    def ask(item):
        name, text = item
        req = {"state": {"system": SYSTEM, "proposal": text}, "questions": QUESTIONS}
        out = subprocess.run([sys.executable, JEV], input=json.dumps(req), capture_output=True, text=True, timeout=180)
        return name, json.loads(out.stdout)["answers"]
    results = dict(pool.map(ask, PROPOSALS.items()))
rows = []
for name, a in results.items():
    i, e, o = a["impact"]["score"], a["effort"]["score"], a["overlap"]["noul"]
    rows.append(((i * (1 - 0.5 * o)) / (1 + e), i, e, o, name))
for r in sorted(rows, reverse=True):
    print(f"{r[0]:.2f}  impact {r[1]:.2f}  effort {r[2]:.2f}  covered {r[3]:.2f}  {r[4]}")
json.dump({"proposals": PROPOSALS, "scores": {r[4]: {"rank": r[0], "impact": r[1], "effort": r[2], "covered": r[3]} for r in rows}},
          open(os.path.join(DATA, "proposals3.json"), "w", encoding="utf-8"), indent=1)
