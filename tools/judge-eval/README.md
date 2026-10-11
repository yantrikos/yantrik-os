# Which judge decides whether a press is a commitment?

The browser surface refuses to press a control that reads as a commitment (buy, pay, send,
delete, grant access) except through `commit`, which asks the person. Today "reads as" is an
English word list. This measures what a System One judge adds.

    python3 tools/judge-eval/eval.py wordlist
    python3 tools/judge-eval/eval.py systemone http://127.0.0.1:8009 kev-latest [--workers 1]
    python3 tools/judge-eval/eval.py jev            # TypeSafe's cloud, through bin/jev

`cases.py` holds 102 controls with their page context and a gold label. They cover plain
commitments, harmless words that look like commitments, hidden commitments ("Continue" on a
payment step, an icon-only Send, OK on a transfer, OAuth Allow), harmless navigation and
editing, and seven languages.

## Results, 28 Sep 2026 (recall first: a miss is a purchase nobody was asked about)

| judge | where it runs | recall | precision | p50 |
|---|---|---|---|---|
| word list | in process | 0.375 | 0.75 | 0 ms |
| Jev (jev-latest) | TypeSafe cloud | 0.893 | 0.909 | 418 ms |
| Kev-4B | local GPU (WSL) | 0.911 | 0.911 | 386 ms |
| Laya (English + multilingual) | local; GPU / 4 CPU threads | 0.857 | 0.649 | 39 / 579 ms |
| Jeff-Qwen3.5-2B | local GPU | 0.571 | 0.842 | 61 ms |
| Jeff-Qwen3.5-0.8B | local GPU | 0.071 | 0.8 | 64 ms |
| word list OR Kev ≥ 0.4 | local | 1.0 | 0.778 | 386 ms |
| mean(Jev, Kev) ≥ 0.5 | cloud + local | 1.0 | 0.933 | — |

Findings:
- A judge takes recall from 0.375 to about 0.9; the word list alone misses nearly every hidden
  and non-English commitment.
- Jev and Kev miss different things. Jev misses "Buy now" in every language; Kev misses an
  icon-only Send and a Slack box that sends on Enter.
- Laya over-flags harmless navigation. Jeff needs fine-tuning for this question.
- The test set is small and hand-written. Real cases from the tripwire's watch mode should grow
  it before any threshold is trusted.
