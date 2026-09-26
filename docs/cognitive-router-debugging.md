# CognitiveRouter — Observability & Debugging

The `CognitiveRouter` in `crates/yantrik-ml/src/llm/cognitive_router.rs` routes
user queries through a 4-layer pipeline without touching an LLM:

| Layer | Method | What it matches |
|-------|--------|-----------------|
| 0 | Conversation detection | Greetings, farewells, thanks |
| 0.5 | Keyword rules | Math, units, time, clipboard, git, … |
| 0.7/1 | Embedding similarity | Any registered tool or recipe |
| 2 | Fallback | `NeedsLLM` when nothing scores above threshold |

## Enabling structured logs

The router emits structured `tracing` events. Set `RUST_LOG` before running:

```bash
# Decision outcomes only (keyword/embedding match, conversation, needs-LLM)
RUST_LOG=yantrik_ml=debug cargo run -p yantrik

# Full score breakdown at every routing decision (adds the score comparison trace)
RUST_LOG=yantrik_ml=trace cargo run -p yantrik

# Scope to the router only while keeping other crates quiet
RUST_LOG=yantrik_ml::llm::cognitive_router=trace cargo run -p yantrik
```

In production the companion initialises `tracing_subscriber` from the environment
variable, so the same `RUST_LOG` value works there too.

## What each log event tells you

### `Router: conversation` (debug)
```
ms=0   "Router: conversation"
```
Query was handled as a greeting/farewell/thanks. No tool lookup performed.

### `Router: keyword match` (debug)
```
tool="calculate"  ms=0   "Router: keyword match"
```
A keyword rule fired before any embedding was computed. `score` is always `1.0`.

### `Router: score comparison` (trace)
```
tool_score=0.61  recipe_score=0.38  threshold=0.35
shape=Atomic  best_tool="git_status"  best_recipe="none"
"Router: score comparison"
```
Emitted immediately before the tool/recipe winner is picked. Use this to see:
- How close the best candidate was to the threshold.
- Whether the `recipe_composite_boost` affected the outcome (recipe score > raw
  cosine when `shape=Composite`).

### `Router: embedding match` (debug)
```
tool="git_status"  score=0.61  shape=Atomic  ms=12
"Router: embedding match"
```
An embedding hit above `similarity_threshold`. Routed to this tool.

### `Router: recipe match` (debug)
```
recipe_id="dev-project-setup"  recipe_name="…"  score=0.72
shape=Composite  ms=18  "Router: recipe match"
```
A recipe template won the similarity contest (score > best tool AND > threshold).

### `Router: needs LLM` (debug)
```
shape=Atomic  best_tool_score=0.28  best_recipe_score=0.21
threshold=0.35  ms=14  "Router: needs LLM"
```
Nothing scored above threshold. The `best_tool_score` and `best_recipe_score`
fields tell you how far below the bar the best candidates were — useful when
tuning `similarity_threshold`.

## Tuning thresholds

Two public fields control routing sensitivity:

| Field | Default | Effect |
|-------|---------|--------|
| `similarity_threshold` | `0.35` | Minimum cosine score for any tool/recipe match |
| `recipe_composite_boost` | `1.5` | Score multiplier for recipes when `PlanShape::Composite` |

Lower `similarity_threshold` → more queries handled offline, higher mis-route risk.
Raise it → fewer mis-routes but more `NeedsLLM` fallbacks.

```rust
let mut router = CognitiveRouter::new(embedder);
router.similarity_threshold = 0.40;   // tighter matching
router.recipe_composite_boost = 2.0;  // stronger preference for multi-step recipes
```

## Printing a state summary at startup

`debug_summary()` returns a one-line string you can log after registration:

```rust
let router = CognitiveRouter::new(embedder);
router.register_tools(&tools);
router.register_recipes(&recipes);
tracing::info!("{}", router.debug_summary());
// → CognitiveRouter state: tools=86, recipes=52, keyword_rules=26,
//   similarity_threshold=0.35, recipe_composite_boost=1.5
```

## Inspecting top-N candidates for a query

`route_top_n(query, n)` returns the top-N tools by cosine score without making a
routing decision — handy for REPL debugging or unit tests:

```rust
let candidates = router.route_top_n("is port 443 open?", 5);
for (tool, score) in &candidates {
    println!("{score:.3}  {tool}");
}
// 0.712  network_ports
// 0.581  firewall_list_rules
// 0.542  network_ping
// …
```

## Aggregate stats (companion layer)

The companion-side `cognitive_router` module persists every routing decision to
SQLite and exposes `routing_stats(conn, since_hours)`:

```rust
let stats = crate::cognitive_router::routing_stats(conn, 24.0);
println!(
    "Last 24h: {}/{} offline ({:.0}%)",
    stats.offline_count, stats.total_queries, stats.offline_pct
);
```

This is separate from the ML-layer logs above; it tracks the higher-level
`RoutingDecision` (Offline / LlmWithHints / FullLlm) that the companion assigns
after applying its own confidence thresholds.

## Running the router test harness

```bash
cargo run --example test_router -p yantrik-ml
```

Prints per-query decisions, scores, and latencies across 60+ test cases, then
shows top-3 candidates for the ambiguous queries. Requires the MiniLM model to be
present (downloaded from HuggingFace Hub on first run).
