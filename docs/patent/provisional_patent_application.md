# PROVISIONAL PATENT APPLICATION

## COGNITIVE MEMORY ENGINE WITH INSTINCT-DRIVEN PROACTIVE BEHAVIOR, UNIFIED IN-PROCESS COMPANION RUNTIME, AND CONTRADICTION-AWARE ADAPTIVE RETRIEVAL

---

**Applicant:** Pranab Sarkar
**Filing Date:** February 2026
**Docket No.:** SARKAR-2026-001

---

## CROSS-REFERENCE TO RELATED APPLICATIONS

This application claims priority as a provisional patent application under 35 U.S.C. §111(b).

---

## FIELD OF THE INVENTION

The present invention relates generally to artificial intelligence systems and, more specifically, to cognitive memory systems for AI companions that combine proactive behavior generation, unified in-process inference, and contradiction-aware memory lifecycle management with adaptive retrieval scoring.

---

## BACKGROUND OF THE INVENTION

### Problem Statement

Current AI assistants and chatbots are fundamentally reactive — they respond only when prompted by user input. They lack the ability to independently identify situations requiring proactive outreach, manage competing behavioral drives, or maintain long-term memory with self-correcting contradiction resolution.

Existing retrieval-augmented generation (RAG) systems suffer from several limitations:

1. **Reactive-only behavior**: No mechanism for proactive, self-initiated communication based on internal cognitive state evaluation.

2. **Multi-process overhead**: Typical deployments require separate processes for LLM inference, embedding generation, vector search, and application logic, resulting in high memory consumption (~1GB+), network serialization overhead, and deployment complexity.

3. **Naive retrieval scoring**: Most systems rely solely on cosine similarity for memory retrieval, ignoring temporal decay, emotional salience, entity relationships, and the interaction between importance and relevance.

4. **No contradiction management**: When contradictory information is stored (e.g., "I work at Company A" followed later by "I work at Company B"), existing systems either return both without resolution or silently overwrite, with no formal detection, classification, or resolution protocol.

5. **No memory consolidation**: Over time, memory stores grow unboundedly with redundant entries. No existing system performs automatic clustering and consolidation of semantically similar memories while preserving knowledge graph connectivity.

### Prior Art Limitations

**Generative Agents (Park et al., 2023)** introduced memory streams with recency × importance × relevance scoring, but used simple multiplicative scoring without relevance-gated importance, temporal decay with configurable half-life, valence-aware boosting, or knowledge graph proximity signals. The system had no proactive messaging capability, no contradiction detection, and no memory consolidation.

**MemGPT (Packer et al., 2023)** introduced tiered memory management with LLM-directed paging, but relied on simple timer-based heartbeats rather than multi-instinct evaluation, had no urgency scoring or deduplication, and provided no contradiction detection or adaptive scoring.

**SOAR (Laird, 2012) and ACT-R (Anderson et al., 2004)** cognitive architectures provide theoretical frameworks for cognitive modeling but operate in symbolic rather than statistical domains and do not address the specific combination of vector-based retrieval, priority-queued proactive behavior, and contradiction-aware lifecycle management.

**LangChain, LlamaIndex, and similar frameworks** provide orchestration layers connecting separate services but do not unify inference, storage, and agent logic into a single process with zero-copy data handoff.

**Vector databases (ChromaDB, Pinecone, Weaviate, FAISS)** provide vector similarity search but offer no memory lifecycle management, contradiction detection, adaptive scoring, or proactive behavior generation.

---

## SUMMARY OF THE INVENTION

The present invention provides a cognitive memory engine and AI companion system comprising three novel subsystems that operate synergistically:

**1. Instinct-Urge Cognitive Scheduler** — A biologically-inspired proactive behavior system where multiple independent instinct modules evaluate companion cognitive state and emit typed urge objects with urgency scores into a managed priority queue featuring composite-key deduplication with urgency boosting, capacity enforcement, time-to-live expiry, and LLM-driven urge bundling for coherent proactive message generation.

**2. Unified In-Process Companion Runtime** — A single compiled binary integrating large language model inference, embedding generation, vector similarity search, relational memory storage, knowledge graph operations, background cognitive maintenance, and interactive agent logic within a shared address space, with zero-copy data handoff between subsystems and a deterministic memory budget suitable for deployment on memory-constrained devices.

**3. Contradiction-Aware Memory Lifecycle with Adaptive Retrieval Scoring** — A memory system featuring multi-signal retrieval scoring with relevance-gated importance amplification, query-aware valence boosting, and knowledge graph proximity; automatic contradiction detection with typed conflict classification and formal resolution protocols; agglomerative clustering-based memory consolidation with CRDT-safe membership tracking and entity edge transfer; and adaptive weight evolution from user feedback.

---

## DETAILED DESCRIPTION OF PREFERRED EMBODIMENTS

### I. INSTINCT-URGE COGNITIVE SCHEDULER

#### I.A. System Overview

The Instinct-Urge Cognitive Scheduler implements a biologically-inspired architecture for generating proactive communications in an AI companion. The system comprises:

- A plurality of independent **instinct modules**, each implementing a common interface for evaluating companion cognitive state
- An **urge queue** implemented as a SQLite-backed priority queue with composite-key deduplication, urgency boosting, capacity enforcement, and time-to-live expiry
- A **background cognition loop** with adaptive evaluation intervals
- A **proactive message generator** that bundles high-urgency urges into coherent natural-language messages via LLM inference

Unlike reactive chatbots that only respond to user input, this system generates proactive communications driven by internal cognitive state evaluation — analogous to biological drives that compete for behavioral control.

#### I.B. Data Structures

##### I.B.1. Urge Specification (Creation Input)

An UrgeSpec represents a request to create or boost an urge:

| Field | Type | Description |
|-------|------|-------------|
| instinct_name | String | Identifier of the source instinct module |
| reason | String | Human-readable justification for the urge |
| urgency | Float [0.0, 1.0] | Urgency score where 0.0 means ignore and 1.0 means act immediately |
| suggested_message | String | Optional pre-composed message text |
| action | Optional String | Optional action identifier |
| context | JSON Object | Arbitrary context data for downstream processing |
| cooldown_key | String | Composite deduplication key |

##### I.B.2. Urge Record (Stored State)

A stored Urge extends the UrgeSpec with lifecycle tracking:

| Field | Type | Description |
|-------|------|-------------|
| urge_id | UUID7 | Time-ordered unique identifier |
| instinct_name | String | Source instinct |
| reason | String | Justification |
| urgency | Float [0.0, 1.0] | Current urgency (may increase via boosting) |
| suggested_message | String | Message text |
| action | Optional String | Action identifier |
| context | JSON Object | Context data |
| cooldown_key | String | Deduplication key |
| status | Enum | One of: pending, delivered, suppressed, expired |
| created_at | Float | Unix timestamp of creation |
| delivered_at | Optional Float | Unix timestamp of delivery |
| expires_at | Optional Float | Unix timestamp of expiry |
| boost_count | Integer | Number of times this urge was boosted via deduplication |

##### I.B.3. Companion State (Evaluation Input)

The CompanionState snapshot captures the current cognitive state for instinct evaluation:

| Field | Type | Description |
|-------|------|-------------|
| last_interaction_ts | Float | Unix timestamp of last user message |
| current_ts | Float | Current time |
| session_active | Boolean | Whether a conversation session is active |
| conversation_turn_count | Integer | Number of turns in current session |
| recent_valence_avg | Optional Float | Average emotional valence from recent memories |
| pending_triggers | Array of JSON | Triggers from the memory cognition loop |
| active_patterns | Array of JSON | Discovered behavioral patterns |
| open_conflicts_count | Integer | Number of unresolved memory conflicts |
| memory_count | Integer | Total active memories |
| config_user_name | String | User's configured name |

#### I.C. Persistent Storage Schema

The urge queue uses a relational table with three indices for efficient operations:

```sql
CREATE TABLE urges (
    urge_id TEXT PRIMARY KEY,
    instinct_name TEXT NOT NULL,
    reason TEXT NOT NULL,
    urgency REAL NOT NULL,
    suggested_message TEXT DEFAULT '',
    action TEXT,
    context TEXT DEFAULT '{}',
    cooldown_key TEXT NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at REAL NOT NULL,
    delivered_at REAL,
    expires_at REAL,
    boost_count INTEGER DEFAULT 0
);

CREATE INDEX idx_urges_status ON urges(status);
CREATE INDEX idx_urges_cooldown ON urges(cooldown_key, status);
CREATE INDEX idx_urges_urgency ON urges(status, urgency DESC);
```

The `idx_urges_cooldown` index enables O(1) deduplication lookups. The `idx_urges_urgency` index enables efficient highest-urgency selection.

#### I.D. Core Algorithms

##### I.D.1. Push with Deduplication and Boosting

The push algorithm implements three-phase urge insertion:

**Phase 1 — Composite-Key Deduplication Check:**

```
PUSH(connection, urge_spec):
    now = current_unix_timestamp()

    existing = QUERY("SELECT urge_id, urgency FROM urges
                      WHERE cooldown_key = urge_spec.cooldown_key
                      AND status IN ('pending', 'delivered')")

    IF existing IS NOT NULL:
        new_urgency = MIN(existing.urgency + BOOST_INCREMENT, 1.0)
        UPDATE urges SET urgency = new_urgency,
                         boost_count = boost_count + 1
                     WHERE urge_id = existing.urge_id
        RETURN NULL  // Existing urge boosted; no new urge created
```

The cooldown_key is a composite string that encodes the deduplication scope. For example, `"check_in"` deduplicates all check-in urges globally, while `"follow_up:{trigger_id}"` deduplicates per-trigger.

The boost mechanism transforms duplicate urge creation into urgency escalation. With a default BOOST_INCREMENT of 0.1, repeated urges progressively increase urgency from their initial value toward 1.0, expressing "this concern keeps recurring" through urgency escalation rather than queue flooding.

**Phase 2 — Capacity Enforcement:**

```
    pending_count = COUNT(*) FROM urges WHERE status = 'pending'
    IF pending_count >= MAX_PENDING:
        lowest = QUERY("SELECT urge_id FROM urges
                        WHERE status = 'pending'
                        ORDER BY urgency ASC LIMIT 1")
        UPDATE urges SET status = 'expired' WHERE urge_id = lowest
```

The capacity enforcement ensures bounded memory usage by expiring the lowest-urgency pending urge when the queue reaches MAX_PENDING (default: 20).

**Phase 3 — New Urge Creation:**

```
    urge_id = generate_uuid7()
    expires_at = now + (EXPIRY_HOURS * 3600)
    INSERT INTO urges (urge_id, instinct_name, reason, urgency,
                       suggested_message, action, context, cooldown_key,
                       status, created_at, expires_at, boost_count)
           VALUES (urge_id, urge_spec.*, 'pending', now, expires_at, 0)
    RETURN urge_id
```

##### I.D.2. Selection Algorithm

```
POP_FOR_INTERACTION(connection, limit):
    urges = QUERY("SELECT * FROM urges
                   WHERE status = 'pending'
                   ORDER BY urgency DESC
                   LIMIT limit")

    FOR EACH urge IN urges:
        UPDATE urges SET status = 'delivered',
                         delivered_at = current_timestamp()
                     WHERE urge_id = urge.urge_id

    RETURN urges
```

The selection algorithm retrieves the highest-urgency pending urges, atomically transitioning them to 'delivered' status with delivery timestamps.

##### I.D.3. Expiry Algorithm

```
EXPIRE_OLD(connection):
    now = current_unix_timestamp()
    UPDATE urges SET status = 'expired'
           WHERE status = 'pending' AND expires_at < now
    RETURN count_of_expired_urges
```

Default time-to-live is 48 hours (EXPIRY_HOURS = 48.0).

##### I.D.4. Suppression

```
SUPPRESS(connection, urge_id):
    UPDATE urges SET status = 'suppressed'
           WHERE urge_id = urge_id
           AND status IN ('pending', 'delivered')
    RETURN changes > 0
```

Suppression allows user or system override of the urgency-based selection policy.

#### I.E. Urge Lifecycle State Machine

```
[New UrgeSpec] ─→ PUSH(cooldown_key)
                    ├── [Match Found] ─→ BOOST existing urgency (return NULL)
                    └── [No Match] ─→ INSERT new urge
                                       ↓
                                    PENDING
                                   ↙   ↓    ↘
                               POP  EXPIRE  SUPPRESS
                                ↓      ↓       ↓
                          DELIVERED  EXPIRED  SUPPRESSED
                                ↓
                          SUPPRESS or EXPIRE
                                ↓
                       SUPPRESSED or EXPIRED
```

States 'pending' and 'delivered' participate in deduplication lookups. States 'suppressed' and 'expired' are terminal — suppressed/expired urges with the same cooldown_key will not block new urge creation.

#### I.F. Instinct Modules

##### I.F.1. Instinct Interface

Each instinct module implements a common interface with two evaluation modes:

```
INTERFACE Instinct:
    name() → String
    evaluate(state: CompanionState) → Array of UrgeSpec      // Background periodic
    on_interaction(state, user_text) → Array of UrgeSpec      // Per-message reactive
```

The dual-mode evaluation enables both background cognitive planning (periodic think cycles) and real-time reactive behavior (triggered by user messages).

##### I.F.2. Implemented Instincts

**Check-In Instinct** — Generates check-in urges when the user has been idle beyond a configurable threshold:

```
Trigger: hours_since_interaction >= check_in_hours (default: 8.0)
Urgency: MIN((hours_since - threshold) / (threshold × 2.0), 0.8)
Cooldown Key: "check_in"
```

The urgency grows linearly from 0 at the threshold to 0.8 (cap) at 3× the threshold.

**Emotional Awareness Instinct** — Responds to negative emotional valence trends detected by the memory cognition loop:

```
Trigger: pending_trigger with type = "valence_trend" AND direction = "negative"
Urgency: MIN(|valence_delta|, 0.9)
Cooldown Key: "emotional_awareness:negative"
```

**Follow-Up Instinct** — Generates follow-up urges for decaying important memories:

```
Trigger: pending_trigger with type = "decay_review" (max 2)
Urgency: MIN(trigger.urgency OR 0.4, 0.7)
Cooldown Key: "follow_up:{trigger_id}"
```

**Reminder Instinct** — Surfaces memories in the "reminder" domain:

```
Trigger: trigger with domain = "reminder" OR reason contains "remind"
Urgency: MAX(trigger.urgency OR 0.5, 0.6)
Cooldown Key: "reminder:{trigger_id}"
```

**Pattern Surfacing Instinct** — Brings discovered behavioral patterns to user attention:

```
Trigger: pending_trigger with type = "pattern_discovered" (max 2)
Urgency: trigger.urgency OR 0.3
Cooldown Key: "pattern:{pattern_type}:{description[0..30]}"
```

**Conflict Alerting Instinct** — Alerts when memory conflicts exceed a threshold:

```
Trigger: open_conflicts_count >= conflict_alert_threshold (default: 5)
Urgency: MIN(open_conflicts_count / 10.0, 0.8)
Cooldown Key: "conflict_alert"
```

Each instinct defines asymmetric urgency bounds and custom scoring appropriate to its domain.

#### I.G. Background Cognition Loop

The background cognition loop runs on an adaptive schedule with three tiers:

| User Activity State | Evaluation Interval |
|---------------------|---------------------|
| Active (idle < 5 min) | Every 5 minutes |
| Normal (idle < 1 hour) | Every 15 minutes |
| Idle (idle > 1 hour) | Every 30 minutes |

Each evaluation cycle performs the following 7-step pipeline:

1. Execute memory engine think cycle (maintenance, triggers, patterns, conflicts)
2. Extract triggers as structured data
3. Fetch active patterns from memory engine
4. Count open memory conflicts
5. Extract emotional valence trend from triggers
6. Update companion cognitive state cache
7. Evaluate all instinct modules against current state; push resulting urges; generate proactive message for urges exceeding urgency threshold (default: 0.7)

#### I.H. Proactive Message Generation

When one or more urges exceed the proactive urgency threshold, the system bundles their reasons and passes them to the LLM for synthesis:

```
GENERATE_PROACTIVE_MESSAGE(companion, high_urgency_urges):
    reasons = [urge.reason FOR EACH urge IN high_urgency_urges]

    system_prompt = "You are {companion_name}, a thoughtful companion.
                     Generate a brief, natural message based on what's
                     on your mind. Be warm but concise (1-2 sentences)."

    user_prompt = "Things on your mind:\n" +
                  JOIN(["- " + reason FOR EACH reason IN reasons])

    response = LLM.chat([system_prompt, user_prompt],
                        max_tokens=150, temperature=0.7, top_p=0.9)

    STORE ProactiveMessage {
        text: response.text,
        urge_ids: [urge.cooldown_key FOR EACH urge],
        generated_at: current_timestamp()
    }
```

This bundling approach prevents disjoint message bombardment by synthesizing multiple concerns into a single coherent proactive message.

#### I.I. Configuration Parameters

| Parameter | Default Value | Description |
|-----------|---------------|-------------|
| EXPIRY_HOURS | 48.0 | Time-to-live for urges in hours |
| MAX_PENDING | 20 | Maximum pending urges in queue |
| BOOST_INCREMENT | 0.1 | Urgency increase per deduplication boost |
| PROACTIVE_URGENCY_THRESHOLD | 0.7 | Minimum urgency for proactive message generation |
| CHECK_IN_HOURS | 8.0 | Idle time threshold for check-in instinct |
| CONFLICT_ALERT_THRESHOLD | 5 | Minimum conflict count for alerting instinct |

---

### II. UNIFIED IN-PROCESS COMPANION RUNTIME

#### II.A. System Architecture

The present invention provides a single compiled binary that integrates the following subsystems within a shared address space:

1. **Language Model Inference Module** — A quantized language model (e.g., in one embodiment, a GGUF-formatted model such as Qwen2.5-0.5B) with token generation, chat completion, and tool calling
2. **Text Embedding Module** — A sentence transformer model (e.g., in one embodiment, MiniLM-L6-v2) producing fixed-dimensional normalized vectors (e.g., 384 dimensions)
3. **Vector Similarity Index** — An in-memory approximate nearest neighbor index (e.g., in one embodiment, an HNSW graph) for vector similarity search
4. **Relational Memory Store** — A persistent relational database (e.g., in one embodiment, SQLite with 11-version schema evolution) storing memories, entities, edges, conflicts, triggers, patterns, and learned weights
5. **Knowledge Graph Module** — An entity-relationship graph with typed edges, N-hop expansion, and proximity scoring
6. **Background Cognitive Maintenance Module** — Periodic evaluation cycles with adaptive intervals, trigger generation, pattern mining, and instinct evaluation
7. **Companion Agent Module** — A 7-step message handling pipeline with memory recall, urge delivery, instinct evaluation, context assembly, language model inference with tool loop, and post-interaction learning

#### II.B. Architecture Comparison

##### II.B.1. Prior Art Architecture (3 Separate Processes)

```
┌─────────────┐  HTTP/JSON  ┌──────────────┐  HTTP/JSON  ┌──────────────┐
│ LLM Server  │ ←─────────→ │  Companion   │ ←─────────→ │  Embedding   │
│ (Process 1) │   ~2-5ms    │  (Process 2) │   ~2-5ms    │  (Process 3) │
│ ~350MB RAM  │  per call   │  ~340MB RAM  │  per call   │  ~350MB RAM  │
└─────────────┘             └──────────────┘             └──────────────┘

Total: ~1040MB RAM + ~4-10ms serialization overhead per inference call
```

##### II.B.2. Present Invention (Single Process)

```
┌──────────────────────────────────────────────────────┐
│                  Single Binary Process                 │
│                                                        │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────┐  │
│  │ LLM      │  │ Embedder │  │   Memory Engine   │  │
│  │ Engine   │  │ (MiniLM) │  │ ┌───────────────┐ │  │
│  │ (GGUF)   │  │          │  │ │ SQLite        │ │  │
│  │          │  │          │  │ │ HNSW Index    │ │  │
│  │ ~350MB   │  │  ~45MB   │  │ │ Knowledge     │ │  │
│  │          │  │          │  │ │ Graph         │ │  │
│  └────┬─────┘  └────┬─────┘  │ │ Trigger Log   │ │  │
│       │              │        │ │ Pattern Store │ │  │
│       │  zero-copy   │        │ └───────────────┘ │  │
│       └──────┬───────┘        └────────┬──────────┘  │
│              │                          │              │
│       ┌──────┴──────────────────────────┘             │
│       │                                                │
│  ┌────┴──────────────────────────────────────────┐   │
│  │            Companion Service                    │   │
│  │  Instincts │ Urge Queue │ Background Cognition │   │
│  └────────────────────────────────────────────────┘   │
│                                                        │
│  Total: ~505MB RAM — zero serialization overhead       │
└──────────────────────────────────────────────────────┘
```

**Memory savings: 51% reduction (1040MB → 505MB)**

#### II.C. Zero-Copy Data Flow

The unified architecture enables zero-copy data handoff at critical performance boundaries:

1. **Embedding → Storage**: Embedding vectors (`Vec<f32>`, 384 dimensions, 1536 bytes) are passed directly from the embedding model's output buffer to the HNSW index insertion and SQLite BLOB storage — no HTTP serialization, no JSON encoding, no memory copy.

2. **HNSW Search → Scoring**: Candidate memory IDs from HNSW approximate nearest neighbor search are passed directly to the composite scoring function with pre-cached metadata — no network roundtrip.

3. **Scoring → Agent Context**: Recalled memory texts are formatted directly into the LLM prompt — no serialization boundary.

4. **LLM Output → Learning**: Generated response text is passed directly to the learning module for memory extraction — no HTTP serialization.

In the prior art 3-process architecture, each of these boundaries requires: (a) serialization to JSON, (b) HTTP request construction, (c) network transmission, (d) HTTP parsing, and (e) JSON deserialization — approximately 6 serialization/deserialization steps per recall operation.

#### II.D. Message Handling Pipeline

The companion service processes each user message through a 7-step pipeline, entirely within the single process:

**Step 1 — Session Management**: Compare current time against last interaction timestamp. If idle time exceeds session timeout (default: 30 minutes), clear conversation history and reset session state.

**Step 2 — Memory Recall**: Embed the user's text using the in-process embedding engine, then search the HNSW index for the top-k (default: 5) most relevant memories using the composite scoring function described in Section III.

**Step 3 — Urge Delivery**: Pop the highest-urgency pending urges (default: 2) from the urge queue, atomically marking them as 'delivered'.

**Step 4 — Reactive Instinct Evaluation**: Evaluate each instinct module's `on_interaction()` method with the current state and user text. Push any resulting urges to the queue.

**Step 5 — Context Assembly**: Construct the LLM prompt incorporating: personality/tone instructions, current datetime, recalled memories (truncated to token budget), delivered urges, active patterns, tool definitions, and conversation history.

**Step 6 — LLM Inference with Tool Loop**: Execute LLM chat completion. If the response contains tool calls, execute each tool (memory_record, memory_recall, entity_relate, memory_think), append results, and re-invoke the LLM (up to 3 rounds).

**Step 7 — Post-Interaction Learning**: Use the LLM to extract factual information, preferences, and entities from the exchange, then record as new memories and entity relationships.

#### II.E. Deterministic Memory Budget

| Component | RAM Allocation |
|-----------|---------------|
| Quantized Language Model (e.g., Qwen2.5-0.5B Q4_K_M) | ~350MB |
| Sentence Transformer (e.g., MiniLM-L6-v2) | ~45MB |
| Language Model KV Cache (2048 context) | ~50MB |
| Memory Engine + Relational Store + Vector Index | ~50MB |
| Companion Logic + Urge Queue | ~10MB |
| **Total** | **~505MB** |

This deterministic budget enables deployment on devices with as little as 1GB free RAM (phones, single-board computers, embedded systems).

#### II.F. Benchmark Results

| Metric | Measured Value |
|--------|---------------|
| Binary size | 16 MB (release build) |
| Startup time (warm cache) | < 1 second |
| Health check latency | < 10 ms |
| Cold chat response | 6.8 seconds |
| Memory recall (10K memories) | 35.1 ms average |
| Memory recall (post-consolidation) | 14.5 ms average |
| Memory storage rate | 99 memories/second |
| Think cycle (10K memories) | 4.6 seconds |
| Consolidation: 10K → active | 167 memories |
| Test suite | 277 tests passing |

---

### III. CONTRADICTION-AWARE MEMORY LIFECYCLE WITH ADAPTIVE RETRIEVAL SCORING

#### III.A. Multi-Signal Retrieval Scoring

##### III.A.1. Composite Score Formula

The retrieval scoring function combines five distinct signals through a novel relevance-gated importance amplification mechanism:

```
base_relevance = W_SIM × similarity + W_DECAY × decay + W_RECENCY × recency
gate = sigmoid(GATE_K × (similarity - GATE_TAU))
score = base_relevance × (1 + gate × ALPHA_IMP × MIN(importance, 1.0))
        × query_valence_boost(valence, query_sentiment)
```

##### III.A.2. Signal Weight Constants

| Parameter | Symbol | Value | Purpose |
|-----------|--------|-------|---------|
| Similarity Weight | W_SIM | 0.50 | Cosine similarity between query and memory embeddings |
| Decay Weight | W_DECAY | 0.20 | Temporal importance decay with configurable half-life |
| Recency Weight | W_RECENCY | 0.30 | Access recency (how recently memory was retrieved) |
| Gate Sharpness | GATE_K | 12.0 | Sigmoid steepness for importance gate |
| Gate Threshold | GATE_TAU | 0.25 | Similarity value where gate = 0.5 |
| Importance Amplification | ALPHA_IMP | 0.80 | Maximum importance boost factor |

##### III.A.3. Sub-Signal Functions

**Temporal Decay** (exponential with configurable half-life):

```
decay_score(importance, half_life, elapsed_seconds) = importance × 2^(-elapsed / half_life)
```

Default half_life = 604,800 seconds (7 days). After one half-life, the decay score equals half the original importance. This models the biological principle that unused memories fade but important ones decay more slowly.

**Access Recency** (exponential):

```
recency_score(age_seconds) = e^(-age / (7 × 86400))
```

Decays to approximately 0.37 after 7 days and 0.14 after 14 days. This prioritizes recently accessed memories regardless of their creation date.

**Valence Boost** (symmetric emotional weighting):

```
valence_boost(valence) = 1.0 + 0.3 × |valence|
```

Range: [1.0, 1.3]. Emotionally charged memories (positive or negative) receive up to a 30% boost over neutral memories, reflecting the psychological principle that emotional experiences are more memorable.

**Relevance-Gated Importance Amplification** (novel contribution):

```
gate = sigmoid(12.0 × (similarity - 0.25))
```

| Similarity | Gate Value | Effect |
|------------|------------|--------|
| 0.05 (irrelevant) | ≈ 0.08 | Importance barely affects score |
| 0.25 (threshold) | ≈ 0.50 | Half importance activation |
| 0.80 (highly relevant) | ≈ 1.00 | Full importance boost |

This gating mechanism is a key innovation. Prior art systems (e.g., Generative Agents) use multiplicative scoring where importance always amplifies results regardless of semantic relevance. The sigmoid gate ensures that high-importance but semantically irrelevant memories do not dominate search results. Only when a memory is already semantically relevant does its importance amplify its score.

**Query-Aware Valence Boost** (novel contribution):

```
query_valence_boost(memory_valence, query_sentiment):
    base = 1.0 + 0.3 × |memory_valence|
    IF query_sentiment = 0.0: RETURN base
    alignment = query_sentiment × SIGN(memory_valence)
    RETURN base × (1.0 + 0.2 × alignment)
```

When the query has detectable emotional sentiment (positive or negative), memories with matching emotional polarity receive additional amplification (up to 1.56× for strong alignment), while mismatched memories receive reduced boost (down to 1.04×). Query sentiment is detected through keyword matching against curated positive/negative word lists with stemming.

##### III.A.4. Graph-Augmented Scoring

When knowledge graph entity expansion is enabled, the scoring formula incorporates a fifth signal — graph proximity — with adjusted weights:

| Parameter | Standard | Graph-Augmented |
|-----------|----------|-----------------|
| W_SIM | 0.50 | 0.35 |
| W_DECAY | 0.20 | 0.15 |
| W_RECENCY | 0.30 | 0.20 |
| W_GRAPH | — | 0.30 |
| ALPHA_IMP | 0.80 | 0.60 |

**Graph Proximity Score:**

```
proximity(memory, expanded_entities) = MAX over all entities linked to memory:
    (cumulative_edge_weight / 2^hops)
```

Entities are expanded from query-extracted seed entities using breadth-first search with edge weight accumulation:

```
EXPAND_ENTITIES(seeds, max_hops=2, max_entities=30):
    frontier = [(seed, hops=0, weight=1.0) FOR seed IN seeds]
    visited = {}

    WHILE frontier NOT EMPTY AND |visited| < max_entities:
        (entity, hops, weight) = frontier.pop_lowest_hops()
        IF entity IN visited: CONTINUE
        visited[entity] = (hops, weight)

        IF hops < max_hops:
            FOR EACH (neighbor, edge_weight) IN get_edges(entity):
                cumulative = weight × edge_weight
                frontier.push((neighbor, hops + 1, cumulative))

    RETURN visited
```

The exponential hop decay (2^hops) prevents distant entities from dominating the proximity signal.

##### III.A.5. Adaptive Weight Learning

The system includes a feedback-driven weight evolution mechanism:

**Feedback Collection:**

```sql
CREATE TABLE recall_feedback (
    id INTEGER PRIMARY KEY,
    query_text TEXT NOT NULL,
    query_embedding BLOB,
    retrieved_rid TEXT NOT NULL,
    feedback TEXT NOT NULL,          -- 'relevant' or 'irrelevant'
    score_at_retrieval REAL,
    created_at REAL NOT NULL,
    namespace TEXT DEFAULT 'default'
);
```

**Learned Weights** (bounded ranges to prevent divergence):

| Weight | Default | Min | Max |
|--------|---------|-----|-----|
| w_sim | 0.50 | 0.30 | 0.70 |
| w_decay | 0.20 | 0.10 | 0.30 |
| w_recency | 0.30 | 0.20 | 0.50 |
| gate_tau | 0.25 | 0.15 | 0.40 |
| alpha_imp | 0.80 | 0.50 | 1.00 |
| keyword_boost | 0.30 | 0.20 | 0.50 |

**Weight Update Algorithm** (signal-proportional gradient-free optimization):

```
LEARN_FROM_FEEDBACK(feedback_samples, current_weights, learning_rate=0.05):
    FOR EACH sample IN feedback_samples:
        signals = [similarity, decay_score, recency_score]  // normalized [0,1]

        IF sample.feedback == "relevant":
            direction = +learning_rate
        ELSE:
            direction = -learning_rate

        // Adjust each weight proportionally to how active that signal was
        FOR i IN 0..num_weights:
            current_weights[i] += direction × signals[i]

        // Re-normalize signal weights to sum to 1.0
        total = SUM(current_weights[0..num_signal_weights])
        FOR i IN 0..num_signal_weights:
            current_weights[i] = current_weights[i] / total

        // Clamp all weights to bounded ranges to prevent divergence
        FOR i IN 0..num_weights:
            current_weights[i] = CLAMP(current_weights[i], min_bound[i], max_bound[i])

    INCREMENT generation_counter
    STORE current_weights WITH generation_counter AND namespace
```

The update direction is proportional to each signal's activation: if a memory was correctly highly relevant with high similarity, the similarity weight increases more than recency weight. The bounded clamping (see table below) prevents any single signal from dominating. A minimum of 10 feedback samples is required before learned weights are substituted into scoring.

Learned weights are stored per-namespace and substituted into the composite score formula when sufficient feedback has been accumulated. A generation counter tracks the number of weight updates.

#### III.B. Contradiction Detection and Conflict Resolution

##### III.B.1. Conflict Types

The system classifies detected contradictions into five types with associated priority levels:

| Conflict Type | Priority | Example |
|---------------|----------|---------|
| IdentityFact | Critical | "I am an engineer" vs. "I am a designer" |
| Preference | High | "I like coffee" vs. "I hate coffee" |
| Temporal | High | Contradictory event sequences |
| Consolidation | Medium | Conflicts arising from memory merge operations |
| Minor | Low | Low-priority contradictions |

##### III.B.2. Conflict Record

Each detected conflict is stored with full provenance:

```sql
CREATE TABLE conflicts (
    conflict_id TEXT PRIMARY KEY,
    conflict_type TEXT NOT NULL,
    priority TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    memory_a TEXT NOT NULL,
    memory_b TEXT NOT NULL,
    entity TEXT,
    rel_type TEXT,
    detected_at REAL NOT NULL,
    detected_by TEXT NOT NULL,
    detection_reason TEXT NOT NULL,
    resolved_at REAL,
    resolved_by TEXT,
    strategy TEXT,
    winner_rid TEXT,
    resolution_note TEXT,
    hlc BLOB NOT NULL,
    origin_actor TEXT NOT NULL
);
```

##### III.B.3. Resolution Strategies

Four formal resolution strategies are supported:

| Strategy | Action on Memory A | Action on Memory B | New Memory |
|----------|--------------------|--------------------|------------|
| keep_a | Preserve as active | Tombstone | None |
| keep_b | Tombstone | Preserve as active | None |
| keep_both | Preserve as active | Preserve as active | None |
| merge | Tombstone | Tombstone | Create merged memory |

For the merge strategy:

```
RESOLVE_CONFLICT_MERGE(conflict_id, new_text, resolution_note):
    1. Tombstone memory_a (set consolidation_status = 'tombstoned')
    2. Tombstone memory_b (set consolidation_status = 'tombstoned')
    3. new_rid = RECORD(new_text, importance = MAX(a.importance, b.importance))
    4. SET new_memory.metadata = {"merged_from": [a.rid, b.rid],
                                   "conflict_id": conflict_id}
    5. UPDATE conflict SET status = 'resolved', strategy = 'merge',
                          winner_rid = new_rid, resolved_at = NOW()
```

#### III.C. Memory Lifecycle State Machine

##### III.C.1. States

| State | Description | In HNSW Index | In Scoring Cache |
|-------|-------------|---------------|-----------------|
| active | Normal, retrievable memory | Yes | Yes |
| consolidated | Merged into semantic summary | No (importance reduced to 30%) | No |
| tombstoned | Logically deleted | No | No |

##### III.C.2. Transitions

```
RECORD() ────→ ACTIVE ←──── RECALL() (increments access_count)
                  │
        ┌─────────┼──────────┐
        ↓         ↓          ↓
   CONSOLIDATE  FORGET   RESOLVE(keep_b/merge)
        ↓         ↓          ↓
  CONSOLIDATED TOMBSTONED ←──┘
  (imp × 0.3)
```

#### III.D. Memory Consolidation

##### III.D.1. Clustering Algorithm

The consolidation system uses greedy agglomerative clustering with dual criteria:

```
FIND_CLUSTERS(memories, sim_threshold=0.6, time_window=7d, min=2, max=10):
    1. Sort memories by creation time (ascending)
    2. Initialize empty cluster list
    3. FOR EACH unclassified memory (earliest first):
       a. Create new cluster with this memory as seed
       b. FOR EACH later unclassified memory:
          IF cosine_similarity(seed.embedding, candidate.embedding) >= sim_threshold
          AND |seed.created_at - candidate.created_at| <= time_window × 86400:
              Add candidate to cluster
          IF cluster.size >= max: BREAK
       c. IF cluster.size >= min: Accept cluster
          ELSE: Discard cluster, mark seed as unclassifiable
    4. RETURN accepted clusters
```

Both semantic similarity AND temporal proximity must be satisfied for cluster membership.

##### III.D.2. Consolidation Merge (Atomic Transaction)

```
CONSOLIDATE_CLUSTER(cluster):
    BEGIN TRANSACTION

    1. summary_text = extractive_summary(cluster)    // Top-importance memories concatenated
    2. mean_embedding = element_wise_mean(cluster.embeddings)
    3. consolidated_importance = MIN(MAX(cluster.importance) × 1.1, 1.0)
    4. mean_valence = MEAN(cluster.valence)
    5. consolidated_half_life = MAX(cluster.half_life) × 1.5

    6. INSERT new semantic memory with consolidated values
    7. INSERT consolidation_members (CRDT membership tracking)
    8. FOR EACH source memory IN cluster:
       UPDATE SET consolidation_status = 'consolidated',
                  consolidated_into = new_rid,
                  importance = importance × 0.3
    9. TRANSFER entity edges from source memories to consolidated memory

    COMMIT
```

The importance factor of 0.3 preserves source memories in the database for historical purposes while reducing their retrieval score. The 1.1× importance boost for the consolidated memory reflects increased confidence in well-evidenced information. The 1.5× half-life extension reflects that consolidated (semantic) memories should persist longer than individual (episodic) memories.

##### III.D.3. Entity Edge Transfer

During consolidation, knowledge graph edges connected to source memories are transferred to the consolidated memory:

```
FOR EACH edge WHERE src IN source_rids OR dst IN source_rids:
    IF edge.src IN source_rids:
        CREATE EDGE (consolidated_rid, edge.dst, edge.rel_type, edge.weight)
    IF edge.dst IN source_rids:
        CREATE EDGE (edge.src, consolidated_rid, edge.rel_type, edge.weight)
```

This preserves knowledge graph connectivity across consolidation operations, ensuring that entity relationships remain queryable.

##### III.D.4. Replication-Safe Membership (Set-Union CRDT)

```sql
CREATE TABLE consolidation_members (
    consolidation_rid TEXT NOT NULL,
    source_rid TEXT NOT NULL,
    hlc BLOB NOT NULL,
    actor_id TEXT NOT NULL,
    PRIMARY KEY (consolidation_rid, source_rid)
);
```

The set-union CRDT semantics enable replication-safe consolidation across multiple devices. When devices independently consolidate overlapping memory sets, the membership table converges through INSERT OR IGNORE with HLC-based tiebreaking.

#### III.E. Trigger System

The cognition loop generates typed triggers that drive instinct evaluation:

| Trigger Type | Cooldown | Expiry | Description |
|--------------|----------|--------|-------------|
| DecayReview | 3 days | 7 days | Important memory decayed below threshold |
| ConsolidationReady | 1 day | 3 days | Cluster of memories eligible for consolidation |
| ConflictEscalation | 2 days | 14 days | Open conflict exceeded age threshold |
| TemporalDrift | 14 days | 7 days | Time sequence inconsistencies detected |
| Redundancy | 1 day | default | Multiple memories expressing same information |
| RelationshipInsight | 7 days | default | New entity relationship pattern discovered |
| ValenceTrend | 7 days | default | Emotional trajectory detected |
| EntityAnomaly | 7 days | default | Unusual entity mention patterns |
| PatternDiscovered | 7 days | default | New recurring behavioral pattern |

#### III.F. Pattern Mining

The system automatically discovers five types of behavioral patterns:

| Pattern Type | Detection Method | Default Threshold |
|-------------|-----------------|-------------------|
| topic_cluster | Semantic similarity clustering | similarity ≥ 0.55, 30-day window |
| temporal_cluster | Sequential event detection | ≥ 3 events |
| co_occurrence | Entity co-appearance frequency | ≥ 3 memories |
| valence_trend | Emotional trajectory analysis | delta ≥ 0.3 |
| entity_hub | Central entity detection | degree ≥ 5 |

Patterns are stored with confidence scores, evidence memory RIDs, occurrence counts, and lifecycle status (active → stale → deprecated).

#### III.G. Encryption at Rest

The system implements envelope encryption for memory protection:

1. **Master Key** (user-provided, 256-bit) wraps a randomly-generated Data Encryption Key (DEK) using AES Key Wrap
2. **DEK** (256-bit, per-database) encrypts sensitive memory fields using AES-256-GCM
3. **Encrypted fields**: text content, embedding vectors, metadata JSON
4. **In-memory indices** (HNSW, scoring cache) operate on decrypted data for performance
5. **Encrypted DEK** stored as base64 in the database metadata table

---

## CLAIMS

### Independent Claims — Family 1: Instinct-Urge Cognitive Scheduler

**Claim 1.** A computer-implemented method for generating proactive communications in an artificial intelligence companion system, the method executed by one or more processors and comprising:
- (a) maintaining, in a persistent data store, an urge queue comprising urge records, each urge record stored as a data structure comprising at least: an urgency score represented as a floating-point value in the range [0.0, 1.0], a composite deduplication key encoded as a string, a status indicator selected from a set of lifecycle states, a time-to-live value, and a boost counter;
- (b) evaluating, by each of a plurality of independent instinct modules, a companion cognitive state data structure comprising at least: a last-interaction timestamp, zero or more trigger objects received from a memory cognition subsystem, and a count of unresolved memory conflicts, wherein each instinct module produces zero or more urge specification objects;
- (c) for each produced urge specification object, performing a composite-key deduplication lookup against the urge queue, wherein if a pending or delivered urge record with a matching composite deduplication key exists, incrementing the existing urge record's urgency score by a configurable boost increment and incrementing the boost counter, rather than inserting a new urge record;
- (d) when no matching urge record exists, inserting a new urge record into the urge queue and enforcing a maximum queue capacity by identifying the lowest-urgency pending urge record and transitioning it to an expired state when the capacity is reached;
- (e) during user interaction processing, selecting the highest-urgency pending urge records up to a configurable limit and atomically transitioning their status to delivered with a delivery timestamp;
- (f) periodically scanning the urge queue and transitioning pending urge records whose time-to-live has been exceeded to an expired state; and
- (g) when one or more pending urge records exceed a configurable proactive urgency threshold, bundling the reason fields from those urge records into a prompt and invoking a language model to generate a coherent natural-language proactive message for delivery to a user.

### Independent Claims — Family 2: Unified In-Process Companion Runtime

**Claim 2.** A system for artificial intelligence companion processing, comprising:
- one or more processors; and
- a memory storing instructions that, when executed by the one or more processors, cause the system to instantiate and operate, within a single operating system process address space, all of the following modules:
- (a) a text embedding module configured to receive input text and produce fixed-dimensional vector representations thereof;
- (b) a vector similarity index maintained in process memory and configured to perform approximate nearest neighbor search over the vector representations;
- (c) a relational memory store configured to persistently store memory records, each memory record comprising at least: text content, a vector embedding, an importance score, an emotional valence value, and a lifecycle status indicator;
- (d) a language model inference module configured to accept token sequences and generate natural language text from quantized model weights stored in a file;
- (e) a knowledge graph module maintaining typed entity-relationship edges with associated weights;
- (f) a background cognitive maintenance module configured to execute periodic evaluation cycles at adaptive intervals determined by detected user activity state, producing trigger objects and discovered pattern records; and
- (g) an interactive companion agent module configured to process user messages through a multi-step pipeline comprising: embedding the user message via the text embedding module, retrieving candidate memory records via the vector similarity index, scoring candidates using a composite scoring function, delivering pending urge records from a priority queue, invoking the language model inference module with assembled context, executing tool calls parsed from the language model output, and performing post-interaction memory extraction;
- wherein the text embedding module, the vector similarity index, the relational memory store, the language model inference module, the knowledge graph module, and the companion agent module communicate via direct in-process function invocations and shared memory references without inter-process communication, serialization into an interchange format, or network transmission.

### Independent Claims — Family 3: Contradiction-Aware Memory Lifecycle with Adaptive Retrieval

**Claim 3.** A computer-implemented method for scoring candidate memory records during retrieval from a memory store, the method executed by one or more processors and comprising:
- (a) receiving a query and computing a query embedding vector via a text embedding function;
- (b) for each candidate memory record, computing a cosine similarity between the query embedding vector and a stored memory embedding vector;
- (c) computing a temporal decay value as a function of the candidate memory record's importance score and elapsed time since creation, using an exponential decay function parameterized by a configurable half-life;
- (d) computing an access recency value as an exponential function of elapsed time since the candidate memory record was last retrieved;
- (e) computing a base relevance score as a weighted sum of the cosine similarity, the temporal decay value, and the access recency value, using signal weights that sum to a normalization constant;
- (f) computing a relevance gate value by applying a sigmoid function with a configurable sharpness parameter to the difference between the cosine similarity and a configurable gate threshold, wherein the relevance gate value approaches zero when the cosine similarity is substantially below the gate threshold and approaches one when the cosine similarity is substantially above the gate threshold;
- (g) computing a gated importance amplification by multiplying a configurable importance amplification factor by the relevance gate value and by the candidate memory record's importance score, such that the importance amplification is substantially suppressed when the cosine similarity falls below the gate threshold; and
- (h) computing a final retrieval score by multiplying the base relevance score by one plus the gated importance amplification, and further multiplying by a valence boost factor derived from the absolute value of the candidate memory record's emotional valence.

**Claim 4.** A computer-implemented method for managing a memory lifecycle in a memory system comprising memory records with associated embedding vectors, the method executed by one or more processors and comprising:
- (a) detecting contradictions between pairs of stored memory records by analyzing semantic content, and creating conflict records, each conflict record comprising: a conflict type classification selected from a predefined set of conflict types having associated priority levels, references to the two contradicting memory records, a detection reason, and a status indicator initialized to an open state;
- (b) resolving a conflict record by applying a resolution strategy selected from: (i) retaining a first memory record and transitioning a second memory record to a tombstoned state, (ii) retaining the second memory record and transitioning the first memory record to a tombstoned state, (iii) retaining both memory records and marking the conflict as non-contradictory, or (iv) transitioning both memory records to a tombstoned state and creating a new merged memory record with combined metadata referencing both source memory records;
- (c) identifying clusters of semantically similar memory records by computing pairwise cosine similarity between embedding vectors and accepting into a cluster only memory records satisfying both: a cosine similarity above a configurable similarity threshold relative to the cluster seed, and a creation timestamp within a configurable temporal window of the cluster seed;
- (d) for each identified cluster meeting a minimum size requirement, executing an atomic consolidation operation comprising: creating a consolidated memory record with a summary text derived from cluster members, a mean embedding vector computed element-wise from the cluster members' embedding vectors, an importance score derived from the maximum importance of cluster members boosted by a configurable factor, and a half-life extended from the maximum half-life of cluster members;
- (e) transferring entity-relationship edges connected to cluster member memory records to the consolidated memory record, preserving knowledge graph connectivity; and
- (f) recording consolidation membership in a set-union conflict-free replicated data type (CRDT) structure keyed by consolidation record identifier and source record identifier, with hybrid logical clock timestamps and actor identifiers, enabling convergent membership resolution across independently operating replicas without coordination.

### Independent Claims — Computer-Readable Media

**Claim 5.** A non-transitory computer-readable medium storing instructions that, when executed by one or more processors, cause the one or more processors to perform the method of Claim 1.

**Claim 6.** A non-transitory computer-readable medium storing instructions that, when executed by one or more processors, cause the one or more processors to perform the method of Claim 3.

### Independent Claims — Combined System

**Claim 7.** A cognitive memory and proactive companion system, comprising:
- one or more processors configured to execute within a single operating system process address space:
- (a) an instinct-urge cognitive scheduler comprising a plurality of independent instinct modules, each configured to evaluate a companion cognitive state data structure and emit urgency-scored urge specification objects into a persistent priority queue implementing composite-key deduplication with urgency boosting, capacity enforcement, and time-to-live expiry, the scheduler further comprising a proactive message generator that bundles high-urgency urge records and generates natural-language messages via a language model inference module;
- (b) a memory retrieval engine implementing composite scoring with relevance-gated importance amplification, wherein a sigmoid gate function modulates the influence of a memory record's importance score based on the memory record's cosine similarity to a query, such that importance amplification is substantially suppressed for semantically dissimilar memory records;
- (c) a contradiction-aware memory lifecycle manager comprising: a contradiction detector that creates typed conflict records, a conflict resolver supporting a plurality of resolution strategies, an agglomerative clustering consolidation module that creates consolidated memory records with mean embeddings and transfers entity-relationship edges from source memory records, and a conflict-free replicated data type membership tracker with hybrid logical clock timestamps for multi-device convergence; and
- (d) a unified runtime integrating a language model inference module, a text embedding module, a vector similarity index, a relational memory store, and a knowledge graph module, all communicating via direct in-process function invocations without serialization or network communication;
- wherein the instinct-urge cognitive scheduler receives trigger objects and pattern records produced by the memory retrieval engine and the contradiction-aware memory lifecycle manager, creating a closed-loop feedback system where memory state changes drive proactive companion behavior.

### Dependent Claims

**Claim 8.** The method of Claim 1, wherein each instinct module implements two evaluation modes: a background periodic mode invoked during scheduled cognitive maintenance cycles, and a per-interaction reactive mode invoked upon receipt of a user message, enabling both anticipatory and responsive urge generation.

**Claim 9.** The method of Claim 1, wherein the periodic evaluation cycles of the background cognitive maintenance execute at adaptive intervals comprising: a first shorter interval when user interaction has occurred within a first time threshold, a second intermediate interval when user interaction has occurred within a second time threshold greater than the first, and a third longer interval when user interaction has not occurred within the second time threshold.

**Claim 10.** The method of Claim 1, wherein the plurality of instinct modules comprises: a check-in instinct that computes urgency as a linear function of idle time exceeding a configurable threshold, an emotional awareness instinct that responds to valence trend trigger objects, a follow-up instinct that responds to decay review trigger objects for memories whose importance has decayed below a threshold, a reminder instinct that surfaces time-sensitive memory records, a pattern surfacing instinct that responds to pattern discovery trigger objects, and a conflict alerting instinct that computes urgency as a function of unresolved memory conflict count.

**Claim 11.** The method of Claim 1, wherein the configurable boost increment causes urgency escalation toward a maximum value upon repeated deduplication matches, expressing recurring concern through monotonically increasing urgency rather than queue growth, and wherein the composite deduplication key encodes scope granularity such that a first instinct type uses a global key deduplicating all urges of that type while a second instinct type uses a per-trigger key deduplicating only urges associated with a specific trigger.

**Claim 12.** The system of Claim 2, wherein the relational memory store further comprises an encryption subsystem implementing envelope encryption, with a user-provided master key wrapping a per-database data encryption key, the data encryption key encrypting at least the text content, embedding vectors, and metadata of memory records, while the vector similarity index operates on decrypted data in process memory.

**Claim 13.** The system of Claim 2, wherein the system operates within a deterministic memory budget of less than one gigabyte of random access memory, comprising: less than 400 megabytes for the language model inference module, less than 100 megabytes for the text embedding module, less than 100 megabytes for language model key-value cache, and less than 100 megabytes for the relational memory store and vector similarity index combined.

**Claim 14.** The method of Claim 3, further comprising augmenting the final retrieval score with a knowledge graph proximity signal computed by: extracting seed entities from the query, performing breadth-first expansion from the seed entities through typed entity-relationship edges up to a configurable hop limit, and for each candidate memory record linked to an expanded entity, computing a proximity score as the maximum cumulative edge weight divided by two raised to the power of the hop count.

**Claim 15.** The method of Claim 3, further comprising an adaptive weight learning system that adjusts the signal weights based on accumulated user relevance feedback, wherein: for each feedback sample indicating a retrieved memory record was relevant, signal weights are incremented proportionally to their corresponding signal activations for that retrieval; for each feedback sample indicating a retrieved memory record was irrelevant, signal weights are decremented proportionally; signal weights are re-normalized after each adjustment; and all weights are bounded within configurable minimum and maximum ranges to prevent divergence.

**Claim 16.** The method of Claim 3, wherein the valence boost factor is further modulated by a detected sentiment of the query, such that candidate memory records having emotional valence polarity aligned with the detected query sentiment receive additional amplification and candidate memory records having emotional valence polarity opposing the detected query sentiment receive reduced amplification.

**Claim 17.** The method of Claim 4, further comprising mining behavioral patterns from stored memory records, the patterns comprising at least: topic clusters identified by semantic similarity, temporal event sequences identified by sequential timestamps, entity co-occurrence patterns identified by shared entity references, emotional valence trends identified by valence trajectory analysis, and entity hub detection identified by entity degree centrality, and generating trigger objects when patterns satisfying confidence thresholds are discovered.

**Claim 18.** The method of Claim 4, wherein each trigger object has a type-specific cooldown period preventing generation of duplicate triggers of the same type within the cooldown period, and a type-specific expiry period after which the trigger object is no longer presented for instinct evaluation.

**Claim 19.** The method of Claim 4, wherein the consolidated memory record's importance score is boosted by a configurable factor above the maximum source memory record importance, reflecting increased confidence in well-evidenced information, and wherein source memory records remain in the memory store with importance reduced by a configurable reduction factor rather than being deleted, preserving historical provenance and enabling auditability of the consolidation process.

---

## ABSTRACT

A cognitive memory and proactive companion system operating within a single process address space, comprising three synergistic subsystems communicating via direct in-process function invocations without serialization overhead: (1) an instinct-urge cognitive scheduler wherein a plurality of independent instinct modules evaluate a companion cognitive state data structure and emit urgency-scored urge specification objects into a persistent priority queue implementing composite-key deduplication with urgency boosting, capacity enforcement, time-to-live expiry, and language-model-driven proactive message generation from bundled high-urgency urge records; (2) a unified in-process companion runtime integrating a language model inference module, a text embedding module, a vector similarity index, a relational memory store, a knowledge graph module, a background cognitive maintenance module with adaptive evaluation intervals, and an interactive companion agent module, all within a shared address space with a deterministic memory budget enabling deployment on memory-constrained devices; and (3) a contradiction-aware memory lifecycle with multi-signal retrieval scoring featuring relevance-gated importance amplification via a sigmoid gate that suppresses importance for semantically dissimilar memories, query-aware valence boosting, and knowledge graph proximity scoring, combined with automatic contradiction detection producing typed conflict records with formal resolution protocols, agglomerative clustering-based consolidation creating mean-embedding summary memories with entity edge transfer, set-union conflict-free replicated data type membership tracking with hybrid logical clock timestamps for multi-device convergence, and adaptive signal weight learning from user relevance feedback bounded within configurable ranges. The instinct-urge scheduler receives trigger objects produced by the memory lifecycle manager, forming a closed-loop feedback system where memory state changes drive proactive companion behavior.

---

## DRAWINGS

[To be prepared by patent attorney — the following figures should be included:]

- **FIG. 1** — System architecture diagram showing the unified single-binary process with all subsystems
- **FIG. 2** — Urge lifecycle state machine diagram
- **FIG. 3** — Instinct evaluation and urge flow diagram
- **FIG. 4** — Background cognition loop with adaptive intervals
- **FIG. 5** — Composite scoring formula with relevance-gated importance amplification
- **FIG. 6** — Memory lifecycle state transitions (active → consolidated → tombstoned)
- **FIG. 7** — Consolidation algorithm with entity edge transfer
- **FIG. 8** — Knowledge graph N-hop expansion for proximity scoring
- **FIG. 9** — 7-step message handling pipeline
- **FIG. 10** — Contradiction detection and resolution flow

---

*This provisional patent application establishes priority for the inventions described herein. A non-provisional application with formal claims, drawings, and additional embodiments will follow within 12 months of the filing date.*
