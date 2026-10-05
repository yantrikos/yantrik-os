"""Aggregate results/*.json into a table and evaluate the pre-registered kill criteria."""
import json, glob, collections, math
import numpy as np

R = collections.defaultdict(list)
for f in sorted(glob.glob("results/*.json")):
    r = json.load(open(f)); R[r["cond"]].append(r)
ORDER = ["plain", "compact", "compact_bpe", "mdg_seq", "mdg_seq_bpe", "mdg_packed", "compact_packed"]
S = {}
print("| condition | seeds | accuracy % (mean ± sd) | per-seed | positions | fwd MFLOPs/ex | KV floats/ex |")
print("|---|---|---|---|---|---|---|")
for c in ORDER:
    if c not in R: continue
    a = np.array([r["acc"] for r in R[c]]) * 100
    sd = a.std(ddof=1) if len(a) > 1 else float("nan")
    S[c] = dict(acc=a.mean(), sd=sd, n=len(a), pos=R[c][0]["mean_positions"],
                per={k: np.mean([r["per_type"][k] for r in R[c]]) * 100 for k in R[c][0]["per_type"]})
    print(f"| {c} | {len(a)} | {a.mean():.1f} ± {sd:.1f} | {', '.join(f'{x:.1f}' for x in a)} | {S[c]['pos']:.1f} | "
          f"{R[c][0]['fwd_flops_per_example']/1e6:.1f} | {R[c][0]['kv_floats_per_example']:.0f} |")
print()
types = list(next(iter(S.values()))["per"])
print("| condition | " + " | ".join(types) + " |"); print("|---" * (len(types) + 1) + "|")
for c in S:
    print(f"| {c} | " + " | ".join(f"{S[c]['per'][t]:.1f}" for t in types) + " |")
print()

def pick(cands):
    cands = [c for c in cands if c in S]
    best = max(S[c]["acc"] for c in cands)
    ok = [c for c in cands if S[c]["acc"] >= best - 1.0]
    return min(ok, key=lambda c: S[c]["pos"])
if all(c in S for c in ["compact", "compact_bpe", "mdg_seq", "mdg_seq_bpe", "mdg_packed", "compact_packed"]):
    T = pick(["compact", "compact_bpe"]); M = pick(["mdg_seq", "mdg_seq_bpe", "mdg_packed"])
    k1 = S[M]["acc"] >= S[T]["acc"] - 1.0 and S[M]["pos"] <= 0.70 * S[T]["pos"]
    print(f"T* = {T} ({S[T]['acc']:.1f}%, {S[T]['pos']:.1f} pos);  M* = {M} ({S[M]['acc']:.1f}%, {S[M]['pos']:.1f} pos)")
    print(f"K1: acc(M*) - acc(T*) = {S[M]['acc']-S[T]['acc']:+.1f} pt; pos ratio = {S[M]['pos']/S[T]['pos']:.2f} -> {'PASS' if k1 else 'FAIL'}")
    k2 = S["compact_packed"]["acc"] >= S["mdg_packed"]["acc"] - 1.0
    print(f"K2: compact_packed {S['compact_packed']['acc']:.1f} vs mdg_packed {S['mdg_packed']['acc']:.1f} -> "
          f"{'packing explains it (not MDG-specific)' if k2 else 'MDG packed beats packed text'}")
    for a, b in [("mdg_seq", "compact_bpe"), ("mdg_seq_bpe", "compact_bpe"), ("mdg_seq", "compact"), ("mdg_packed", "compact_packed")]:
        d = S[a]["acc"] - S[b]["acc"]; noise = 2 * max(S[a]["sd"], S[b]["sd"])
        print(f"K3/compare {a} vs {b}: Δacc {d:+.1f} pt (2·sd = {noise:.1f}), positions {S[a]['pos']:.1f} vs {S[b]['pos']:.1f}"
              f" -> {'no detectable difference' if abs(d) < noise else ('higher' if d > 0 else 'lower')}")
