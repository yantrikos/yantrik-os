"""Tiny domain-trained BPE over word units (merges adjacent words into one token).
Used to give COMPACT NL the 'domain-trained tokenizer' help: frequent multiword
phrases like 'goes to', 'because of', '. t5' can become single positions."""
from collections import Counter

def train(streams, n_merges):
    seqs = [list(s) for s in streams]; merges = []
    for _ in range(n_merges):
        c = Counter()
        for s in seqs:
            c.update(zip(s, s[1:]))
        if not c: break
        (a, b), n = c.most_common(1)[0]
        if n < 2: break
        merges.append((a, b)); seqs = [_merge(s, a, b) for s in seqs]
    return merges

def _merge(s, a, b):
    out = []; i = 0
    while i < len(s):
        if i + 1 < len(s) and s[i] == a and s[i + 1] == b:
            out.append(a + "_" + b); i += 2
        else:
            out.append(s[i]); i += 1
    return out

def apply(s, merges):
    s = list(s)
    for a, b in merges:
        s = _merge(s, a, b)
    return s
