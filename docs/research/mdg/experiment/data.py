"""Turn worlds into model inputs for each condition.

Every condition is a list of POSITIONS; each position is a list of token ids
(length 1 for ordinary token sequences, length k for packed facts/sentences).
"""
import random, re
import worlds as W

CONDITIONS = ["plain", "compact", "compact_bpe", "mdg_seq", "mdg_seq_bpe", "mdg_packed", "compact_packed"]
N_MERGES = 200   # domain-BPE budget, identical for compact text and MDG codes
_MERGES = {}

def merges(kind):
    """Domain BPE trained on a fixed corpus of 600 worlds (seed 777), separately for compact text and MDG codes."""
    if kind not in _MERGES:
        import bpe
        rng = random.Random(777); streams = []
        for _ in range(600):
            w = W.World(rng, **world_kwargs(rng))
            streams.append([x for s in W.compact_sents(w) for x in s] if kind == "compact"
                           else [x for f in W.mdg_facts(w) for x in f])
        _MERGES[kind] = bpe.train(streams, N_MERGES)
    return _MERGES[kind]

def _words(text):
    return re.findall(r"\d+:\d+|[a-z]+|[.,?]", text.lower())

def example(w, q, cond, rng):
    """Return (positions: list[list[str]], answer:str)."""
    if cond == "plain":
        ctx = [[x] for x in _words(W.render_plain(w, rng))]
        qq = [[x] for x in _words(W.q_plain(q))]
    elif cond == "compact":
        ctx = [[x] for s in W.compact_sents(w) for x in s]
        qq = [[x] for x in W.q_compact(q)]
    elif cond in ("compact_bpe", "mdg_seq_bpe"):
        import bpe
        if cond == "compact_bpe":
            c, qs, m = [x for s in W.compact_sents(w) for x in s], W.q_compact(q), merges("compact")
        else:
            c, qs, m = [x for f in W.mdg_facts(w) for x in f], W.q_mdg(q), merges("mdg")
        ctx = [[x] for x in bpe.apply(c, m)]
        qq = [[x] for x in bpe.apply(qs, m)]
    elif cond == "mdg_seq":
        ctx = [[x] for f in W.mdg_facts(w) for x in f]
        qq = [[x] for x in W.q_mdg(q)]
    elif cond == "mdg_packed":
        ctx = [f for f in W.mdg_facts(w)]
        qq = [W.q_mdg(q)]
    elif cond == "compact_packed":
        ctx = [s[:-1] for s in W.compact_sents(w)]   # one position per sentence, '.' dropped
        qq = [W.q_compact(q)[:-1]]
    return ctx + [["<sep>"]] + qq + [["<read>"]], q[2]

def build_vocab():
    """Closed vocabulary over all conditions (word-level = the most compressive domain tokenizer
    possible for this closed world: every name/place/time is a single token)."""
    V = {"<pad>": 0, "<sep>": 1, "<read>": 2}
    rng = random.Random(12345)
    for i in range(300):
        w = W.World(rng, **world_kwargs(rng))
        for q in w.questions(rng):
            for c in CONDITIONS:
                pos, _ = example(w, q, c, rng)
                for p in pos:
                    for x in p:
                        V.setdefault(x, len(V))
    A = {a: i for i, a in enumerate(W.answer_vocab())}
    return V, A

def world_kwargs(rng):
    return dict(n_people=6, n_places=5, n_objects=3, n_devices=2, n_events=16)
