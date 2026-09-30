# Research sweeps

How we read what's being published and decide what's worth building. Papers are fetched from arXiv. **Jev** (TypeSafe System One, `jev-latest`) scores each abstract against what Yantrik OS is. The strongest are read by hand, turned into concrete proposals, and ranked by Jev again on impact, effort and how much of each we already have.

| Step | Script | Writes |
|---|---|---|
| Fetch the first seven areas (security, capabilities, memory, memory privacy, OS agents, reliability, small-model tools) | `fetch.py` | `data/papers.jsonl` |
| Fetch the wider sweep: 18 more areas, skipping papers already scored | `fetch2.py` | `data/papers2.jsonl` |
| Score each paper: `value` (0–4), `area`, `actionable` and `evidence` | `score.py`, `score2.py` | `data/scored*.jsonl` |
| Rank proposals on impact, effort and overlap | `proposals.py`, `proposals2.py` | stdout |

`data/ranked.json` and `data/ranked2.json` are the 2026-09-28 results: 782 papers, January–September 2026, each with Jev's scores and its abstract, best first.

**Running it**
- `JEV_CLI` is the path to the `jev` command. It reads `JEV_API_KEY` and never prints it.
- `RESEARCH_DIR` is where data goes. The default is `data/` here.
- arXiv is fetched with `curl`, because Python's `urllib` gets HTTP 406 from it on the build workstation.

**2026-09-28 outcome:** issues #451–#458 and #460–#462, tracked in #459.
