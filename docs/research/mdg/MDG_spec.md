# Multidimensional Grammar (MDG)
## A Machine-Native Universal Semantic Language

**Status:** Conceptual research specification  
**Version:** 0.1  
**Purpose:** Explore a representation and grammar designed specifically for AI systems rather than human languages.

---

## 1. Vision

MDG is a proposed machine-native language in which information is not primarily represented as a sequence of human-readable words.

The central idea is:

> **Represent meaning as a multidimensional, compositional structure and only serialize it into tokens when required by a computational architecture.**

MDG is therefore not intended to replace English, Bengali, programming languages, JSON, or other human-facing interfaces.

Those languages become **front ends** to MDG.

A possible architecture is:

```text
Human language / code / image / audio / tools
                    ↓
             Semantic Encoder
                    ↓
        Multidimensional Grammar
                    ↓
          Semantic Structure
                    ↓
       Learned Discrete Encoding
                    ↓
                  LLM
                    ↓
       Learned Discrete Encoding
                    ↓
        Multidimensional Grammar
                    ↓
             Semantic Decoder
                    ↓
Human language / code / image / audio / actions
```

---

# 2. Core Design Principle

Traditional language is optimized for human communication.

MDG is optimized for:

- semantic density
- compositionality
- precision
- machine reasoning
- temporal reasoning
- causal reasoning
- uncertainty
- provenance
- multimodality
- compression
- efficient transformer processing

The objective is not:

> fewer words.

The objective is:

> **more recoverable information and useful meaning per computational unit.**

---

# 3. Why Not Simply Create a Better Tokenizer?

A tokenizer operates primarily on a sequence.

For example:

```text
"The database remembers the user's preference."
```

might become:

```text
The | database | remembers | the | user | 's | preference
```

A better tokenizer can reduce repeated surface patterns, but it still fundamentally represents language as a sequence.

MDG instead asks:

> What if the underlying representation did not need to resemble language at all?

The representation could instead encode:

```text
ENTITY(user)
RELATION(prefers)
OBJECT(database)
STATE(memory)
```

and eventually compress this structure into learned machine codes.

---

# 4. MDG Is Not Primarily a Sequence

The fundamental MDG object is a **semantic structure**.

Conceptually:

```text
             TIME
              │
              │
ENTITY ─── RELATION ─── ENTITY
   │           │
 STATE       CAUSE
   │           │
 PROPERTY    EVENT
               │
             ACTION
               │
             GOAL
```

The structure may contain:

- entities
- relations
- states
- events
- actions
- properties
- goals
- constraints
- time
- space
- quantities
- causality
- uncertainty
- provenance
- modality
- conflict

The ordering used to serialize this structure for a transformer is an implementation detail, not the semantic definition.

---

# 5. Core Semantic Dimensions

MDG begins with a small set of universal dimensions.

## 5.1 Entity

Something that can be referred to.

Examples:

```text
PERSON
OBJECT
ANIMAL
SOFTWARE
ORGANIZATION
LOCATION
CONCEPT
MODEL
DEVICE
DOCUMENT
```

Example:

```text
ENTITY(P1, PERSON)
ENTITY(RUST, LANGUAGE)
ENTITY(YDB, SOFTWARE)
```

---

## 5.2 Relation

A connection between semantic objects.

Examples:

```text
OWNS
USES
PREFERS
DEPENDS_ON
LOCATED_AT
CREATED_BY
PART_OF
CAUSES
SIMILAR_TO
CONTRADICTS
```

Example:

```text
P1 ──PREFERS──> RUST
YDB ──USES──> RUST
```

---

## 5.3 State

A condition that may change over time.

```text
STATE(YDB, ACTIVE)
STATE(SERVER, OFFLINE)
STATE(PROJECT, DEVELOPMENT)
```

States should have temporal validity.

```text
STATE(
    YDB,
    ACTIVE,
    valid_from=T1,
    valid_until=T2
)
```

---

## 5.4 Event

Something that happens.

```text
EVENT(
    SWITCH_MODEL,
    actor=P1,
    from=QWEN,
    to=DEEPSEEK,
    time=T
)
```

Events may contain other events.

---

## 5.5 Action

An intentional or executable operation.

```text
ACTION(
    actor=AGENT,
    operation=RUN,
    object=MODEL
)
```

---

## 5.6 Goal

A desired future state.

```text
GOAL(
    actor=P1,
    objective=MAXIMIZE(TPS)
)
```

Goals can contain constraints.

```text
GOAL(
    objective=MAXIMIZE(TPS),
    constraint=VRAM < 24GB
)
```

---

## 5.7 Constraint

A condition that limits possible actions or states.

```text
CONSTRAINT(
    VRAM < 24GB
)
```

Constraints can be:

- hard
- soft
- probabilistic
- temporal
- resource-based
- safety-related

---

## 5.8 Time

Time is a first-class semantic dimension.

MDG should support:

```text
INSTANT
INTERVAL
DURATION
BEFORE
AFTER
DURING
SINCE
UNTIL
RECURRING
```

Example:

```text
PREFERS(P1, QWEN)
valid_until=T2

PREFERS(P1, DEEPSEEK)
valid_from=T2
```

This avoids forcing temporal reasoning into prose.

---

## 5.9 Space

Spatial relationships are also first-class.

```text
LOCATED_AT(server, datacenter)
NEAR(device, router)
INSIDE(object, room)
ABOVE(A, B)
DISTANCE(A, B, 12m)
```

Spatial information can be continuous rather than purely symbolic.

---

# 6. Epistemic Dimension

MDG must distinguish between different kinds of knowledge.

A claim may be:

```text
OBSERVED
REMEMBERED
INFERRED
PREDICTED
HYPOTHESIZED
SIMULATED
DESIRED
```

Example:

```text
CLAIM(
    P1 PREFERS RUST,
    mode=INFERRED,
    confidence=0.87
)
```

This is critical for intelligent agents because:

> A fact and a hypothesis are not the same type of information.

---

# 7. Confidence

Confidence is a native dimension.

```text
confidence = 0.97
```

A claim could therefore be represented as:

```text
PREFERS(P1, RUST)
confidence=0.97
```

Confidence may itself have provenance.

```text
confidence(
    value=0.97,
    source=OBSERVATION_182
)
```

---

# 8. Provenance

Every important piece of knowledge should be able to identify where it came from.

Possible sources:

```text
USER
DOCUMENT
TOOL
SENSOR
MODEL
INFERENCE
MEMORY
DATABASE
EXTERNAL_API
```

Example:

```text
PREFERS(P1, RUST)
source=USER
observed=T1
confidence=1.0
```

This enables source-aware reasoning.

---

# 9. Causality

Causality should be explicit.

```text
CAUSE(A, B)
```

Example:

```text
CAUSE(
    DEEPSEEK_HIGHER_SPEED,
    SWITCH_TO_DEEPSEEK
)
```

Causal chains can be recursive:

```text
A → B → C → D
```

The model does not need to infer every causal relationship from prose.

---

# 10. Intent

Intent is different from action.

```text
INTENT(
    actor=P1,
    desired=RUN_QWEN
)
```

A goal may result in multiple possible actions.

```text
GOAL
  ↓
PLAN
  ↓
ACTION
  ↓
EVENT
  ↓
STATE_CHANGE
```

This creates a natural representation for agentic systems.

---

# 11. Conflict

Contradiction should be represented explicitly.

Instead of:

```text
P1 PREFERS QWEN
P1 PREFERS DEEPSEEK
```

being two unrelated facts:

```text
CONFLICT(
    PREFERS(P1,QWEN),
    PREFERS(P1,DEEPSEEK)
)
```

The system can then determine whether the conflict is:

- temporal
- contextual
- factual
- unresolved
- caused by changing preferences

For example:

```text
PREFERS(P1,QWEN)
valid_until=T1

PREFERS(P1,DEEPSEEK)
valid_from=T1
```

This is not a contradiction.

---

# 12. Multidimensional Rather Than Linear

A conventional representation:

```text
A B C D E
```

implicitly requires sequence.

MDG represents relationships independently of serialization order.

Conceptually:

```text
                TIME
                 │
                 ▼
              EVENT
             /  │  \
            /   │   \
        ACTOR ACTION OBJECT
          │      │      │
          P1   SWITCH   MODEL
                       /    \
                     QWEN  DEEPSEEK
```

The graph is the semantic object.

A transformer-specific serializer may convert it to a sequence, but that sequence is not the language itself.

---

# 13. Sparse Semantic Tensors

One possible physical representation is a sparse tensor.

Conceptually:

```text
[
  entity=P1,
  relation=PREFERS,
  object=RUST,
  time=NOW,
  confidence=0.97,
  source=USER,
  modality=OBSERVED
]
```

Only relevant dimensions are activated.

This allows the representation to become very compact.

---

# 14. Composition

MDG should be recursively compositional.

For example:

```text
GOAL(
    actor=P1,
    action=RUN,
    object=QWEN27B,
    constraints=[
        VRAM < 24GB,
        TPS > 100
    ]
)
```

The entire structure can become a single semantic object.

Composition should work at arbitrary depth.

```text
CAUSE(
    EVENT(
        ACTION(
            ...
        )
    )
)
```

---

# 15. Semantic Factoring

MDG should support factoring repeated information.

Suppose:

```text
A USES Rust
A USES Linux
A USES PostgreSQL

B USES Rust
B USES Linux
B USES PostgreSQL
```

Instead of repeating:

```text
COMMON {
    USES Rust
    USES Linux
    USES PostgreSQL
}

A + COMMON
B + COMMON
```

This is **semantic compression through grammar**.

The representation itself can discover and exploit shared structure.

---

# 16. Learned Semantic Codes

Once the grammar is defined, semantic structures can be mapped into learned discrete codes.

For example:

```text
PREFERS(P1,RUST)
```

could eventually become:

```text
ζ17 ζ3 ζ812
```

where:

```text
ζ17  = entity representation
ζ3   = relation representation
ζ812 = concept representation
```

These codes do not need to correspond to words.

They may be learned directly by the model.

---

# 17. Codes Should Be Contextual

A code should not necessarily have one fixed meaning.

Its interpretation can depend on:

- semantic dimension
- structural position
- neighboring codes
- temporal context
- active world state
- task

Therefore:

```text
ζ17
```

does not necessarily mean one human-readable concept.

It may represent a position or transformation in a learned semantic space.

---

# 18. Hierarchical Encoding

MDG should support hierarchical codes.

Common concepts can use very short representations.

Rare concepts can use longer representations.

Conceptually:

```text
COMMON:
ζ4

LESS_COMMON:
ζ4 ζ91

RARE:
ζ4 ζ91 ζ773 ζ12
```

This creates a variable-length semantic encoding.

The objective is approximately:

> **Use computational capacity proportional to information content.**

---

# 19. Exact and Semantic Modes

MDG should have at least two modes.

## Lossless mode

Required for:

- source code
- numbers
- legal text
- configuration
- database records
- exact instructions
- identifiers

The original information must be recoverable.

## Semantic mode

Used for:

- memory
- conversation
- retrieval
- world models
- summaries
- reasoning context

The objective is preservation of task-relevant meaning rather than exact surface form.

---

# 20. Numbers as Native Objects

Numbers should not be treated like ordinary words.

Examples:

```text
INT(18793952)
FLOAT(0.973)
MONEY(1250.37, USD)
DURATION(3.2, seconds)
DATE(2026-10-04)
```

The representation should support exact arithmetic and comparison without relying on linguistic tokenization.

---

# 21. Multimodal Representation

Text, images, audio and video should map into the same semantic system.

For example, an image could produce:

```text
SCENE
 ├── OBJECT(car)
 │    ├── COLOR(red)
 │    ├── TYPE(sedan)
 │    └── POSITION(x,y)
 │
 └── OBJECT(person)
      └── POSITION(x,y)
```

Audio could produce:

```text
SPEECH_EVENT
    speaker=P1
    language=BENGALI
    time=T
    content=...
    emotion=...
```

Video becomes a sequence of semantic state changes.

This makes MDG a candidate **universal representation layer**.

---

# 22. Tool Interface

Tools should ideally communicate through MDG rather than forcing the model through verbose natural-language or JSON representations.

Example:

```text
REQUEST(
    operation=GET_WEATHER,
    location=BENTONVILLE
)
```

Response:

```text
WEATHER(
    location=BENTONVILLE,
    temperature=72F,
    humidity=61%,
    condition=CLEAR,
    observed=T
)
```

The exact wire encoding could be highly compressed.

---

# 23. Memory Interface

A memory system could store:

```text
ENTITY
STATE
EVENT
RELATION
GOAL
PREFERENCE
CONSTRAINT
SOURCE
CONFIDENCE
TEMPORAL_VALIDITY
```

Instead of repeatedly converting memory into prose.

This is especially relevant to persistent agent architectures.

A memory system such as YantrikDB could act as a native MDG knowledge substrate.

---

# 24. LLM Interface

The ultimate architecture could be:

```text
              MDG
               │
       ┌───────┴───────┐
       │               │
   World State       Context
       │               │
       └───────┬───────┘
               ↓
        Semantic Encoder
               ↓
        Discrete MDG Codes
               ↓
             LLM
               ↓
        Discrete MDG Codes
               ↓
        Semantic Decoder
               ↓
         World / Action
```

The LLM therefore becomes a processor of semantic structures rather than primarily a predictor of human-language fragments.

---

# 25. Key Research Hypothesis

The central hypothesis is:

> **A transformer trained to operate on a machine-native multidimensional semantic representation may perform useful reasoning with substantially fewer sequence positions than a transformer operating directly on natural-language tokens.**

This should be tested rather than assumed.

---

# 26. Metrics

The project should measure:

### Semantic density

```text
useful semantic information / token
```

### Information density

```text
recoverable information / bit
```

### Context efficiency

```text
task-relevant world state / context token
```

### Reasoning efficiency

```text
task accuracy / FLOP
```

### Compression ratio

```text
natural-language representation /
MDG representation
```

### Fidelity

```text
original meaning preserved /
required meaning
```

### Generalization

Can the system represent concepts that were not explicitly present during grammar design?

---

# 27. Design Constraints

MDG should satisfy:

1. **Compositionality**
2. **Recursion**
3. **Variable-length encoding**
4. **Sparse representation**
5. **Temporal reasoning**
6. **Causal reasoning**
7. **Uncertainty**
8. **Provenance**
9. **Conflict representation**
10. **Multimodal compatibility**
11. **Exact numeric representation**
12. **Tool interoperability**
13. **Efficient serialization**
14. **Learnable discrete codes**
15. **No dependence on human-readable syntax**

---

# 28. What MDG Should NOT Become

MDG should not simply become:

- compressed English
- another programming language
- JSON with shorter syntax
- RDF with different names
- a giant fixed ontology
- a larger BPE vocabulary
- a collection of arbitrary symbols

The objective is fundamentally different.

MDG should define **how an intelligent machine represents relationships between things, events, states, goals and evidence.**

---

# 29. Proposed Development Path

## Phase 1 — Semantic Grammar

Define the minimal universal primitives:

```text
ENTITY
RELATION
STATE
EVENT
ACTION
PROPERTY
TIME
SPACE
QUANTITY
CAUSE
GOAL
CONSTRAINT
BELIEF
OBSERVATION
INFERENCE
CONFLICT
```

Do not optimize compression yet.

---

## Phase 2 — Canonical Representation

Define an unambiguous canonical structure.

The same meaning should produce the same or equivalent MDG structure.

---

## Phase 3 — Encoder / Decoder

Build:

```text
Natural language
      ↓
MDG
      ↓
Natural language
```

and measure semantic fidelity.

---

## Phase 4 — Compression

Experiment with:

- dictionary coding
- graph factoring
- hierarchical codes
- entropy coding
- vector quantization
- learned discrete representations
- product quantization
- residual semantic codes

---

## Phase 5 — Model Training

Train a small model directly on MDG.

Do not initially attempt to modify a large model.

A small proof-of-concept model can answer:

> Is the representation itself better?

---

## Phase 6 — Transformer Integration

Compare:

```text
Natural language → Transformer
```

against:

```text
Natural language → MDG → Transformer
```

Measure:

- accuracy
- context length
- training efficiency
- inference speed
- KV-cache size
- memory bandwidth
- FLOPs

---

# 30. The Long-Term Vision

The ultimate goal is not merely a compressed tokenizer.

It is:

```text
              HUMAN WORLD
                   │
      ┌────────────┼────────────┐
      ↓            ↓            ↓
     TEXT        IMAGE        AUDIO
      │            │            │
      └────────────┼────────────┘
                   ↓
                 MDG
                   │
       ┌───────────┼───────────┐
       ↓           ↓           ↓
    MEMORY       TOOLS      WORLD MODEL
       │           │           │
       └───────────┼───────────┘
                   ↓
                  LLM
                   │
                   ↓
                ACTION
                   │
                   ↓
              WORLD STATE
```

Human languages become interfaces.

MDG becomes the machine's common semantic representation.

---

# 31. Core Principle

The most important principle of MDG is:

> **Do not force machine intelligence to think in a representation designed for human communication.**

Human language is extraordinarily powerful, but it carries historical baggage:

- ambiguity
- redundancy
- sequential constraints
- linguistic morphology
- cultural assumptions
- irregular syntax
- inefficient numeric representation

A machine-native language can start from a different question:

> **What is the most efficient structure in which an intelligent system can represent, manipulate, communicate and transform information?**

That is the problem MDG is intended to explore.

---

## 32. Initial Research Question

The first experimental question should be deliberately small:

> **Can a model trained on a multidimensional semantic representation solve the same tasks using substantially fewer computational sequence positions than natural-language tokens?**

If the answer is yes, the next question becomes:

> **How far can the representation be compressed before reasoning quality degrades?**

That boundary is potentially more important than the language itself.

---

## 33. Possible Future Name

Working name:

**MDG — Multidimensional Grammar**

Possible future terminology:

- Machine Semantic Language (MSL)
- Universal Machine Language (UML)
- Universal Semantic Representation (USR)
- Cognitive Intermediate Representation (CIR)
- Multidimensional Semantic Language (MSL)

The name should remain secondary until the representation is experimentally validated.

---

# 34. Final Thesis

MDG proposes that the next generation of AI communication should move through three conceptual layers:

```text
              HUMAN LANGUAGE
                    ↓
             MEANING / WORLD
                    ↓
       MULTIDIMENSIONAL GRAMMAR
                    ↓
          LEARNED MACHINE CODES
                    ↓
                  MODEL
```

The breakthrough, if it exists, will not come from inventing shorter words.

It will come from discovering whether **meaning itself can be represented in a fundamentally more computationally efficient geometry.**
