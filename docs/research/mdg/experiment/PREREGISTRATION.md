# MDG R1 — pre-registration (written and committed before the main runs)

**Question (spec §25/§32):** on a task where the MDG ground truth is exact, does a model
reading MDG reach the same accuracy as a model reading text, using ≥30% fewer sequence
positions than the *strongest* text baseline?

## Conditions (identical model, identical training examples, identical test set)
| id | input | positions per example (approx.) |
|---|---|---|
| plain | verbose English: pronouns, paraphrase, clock times, filler; word-level tokens | ~290 |
| compact | compact English: resolved references, canonical names, explicit time ids, one fact per clause; word-level | ~170 |
| compact_bpe | compact English + domain-trained BPE (200 merges over word units) | ~77 |
| mdg_seq | MDG typed tuples, one atomic code per slot | ~113 |
| mdg_seq_bpe | MDG codes + same-budget domain BPE (200 merges) | ~72 |
| mdg_packed | MDG, one position per fact (slot-factorised embedding) | 30 |
| compact_packed | compact English, one position per sentence (same packing mechanism) | 30 |

Word-level tokenisation is already the most compressive domain tokenizer for single words in this closed vocabulary (every name, place and time is one token).

## Primary comparison and kill criteria (fixed now)
Let A(c) = mean test accuracy over 3 seeds, P(c) = mean positions.
Best text baseline T* = the text condition among {compact, compact_bpe} with the fewest positions whose
accuracy is within 1 pt of the best text accuracy. Best MDG condition M* is chosen the same way from {mdg_seq, mdg_seq_bpe, mdg_packed}.

* **K1 (central hypothesis):** supported only if A(M*) ≥ A(T*) − 1.0 pt AND P(M*) ≤ 0.70 · P(T*).
  Otherwise the hypothesis is **not supported** on this task.
* **K2 (is it the MDG?):** if K1 passes only via packing, it is attributed to MDG only if
  compact_packed does NOT reach A ≥ A(mdg_packed) − 1.0 pt. If compact_packed matches, the saving is
  attributed to fact-level packing (available to any cleanly segmented text), not to MDG.
* **K3 (tokenizer control):** mdg_seq vs compact_bpe and mdg_seq_bpe vs compact_bpe are reported as-is; if a
  BPE'd text baseline has fewer positions than an MDG sequence at equal accuracy, that is reported as
  evidence against an MDG advantage at the serialisation level.
* Accuracy differences smaller than 2× the seed std are reported as "no detectable difference".

## Fixed protocol
* Generator: `worlds.py` (6 people, 5 places, 3 objects, 2 devices, 16 timed events). Test set: 2000 worlds, all
  question types (~11.8k questions), generator seed 10^6. Train seeds 0,1,2.
* Model: pre-LN Transformer encoder, d=64, 3 layers, 4 heads, FFN 256, learned positions, no dropout;
  AdamW lr 1e-3, wd 0.01, 200-step warmup + cosine; batch 32; 2000 steps (64k fresh training worlds).
* Matched: parameters, examples seen, optimizer, schedule. Reported, not matched: FLOPs, KV size, wall-clock.
* One learnability pilot on `mdg_packed` only (to check the model trains at all at this size) is permitted
  before the main runs, and is disclosed in the report.
