# MDG — Research Report R1

**Branch:** `research/mdg-r1` · **Spec:** `docs/research/mdg/MDG_spec.md` (v0.1) · **Code:** `docs/research/mdg/experiment/` · **Date:** 2026-10-05

## Executive summary (for Pranab)

**Bottom line:** the central hypothesis (§25/§32) is **not yet supported in a form that is about MDG**. The first experiment found a large position saving, but nearly all of it comes from **packing one fact into one sequence position**. Plain compact English packed the same way does nearly as well in the pre-registered runs. The **typed MDG schema itself added +2.9 accuracy points at an identical position count, which is within seed noise** (3 seeds, Welch t ≈ 1.8).

At the serialisation level, MDG written one code per slot is 34% shorter than word-level compact English. But a fair text baseline uses a 200-merge domain-trained tokenizer, and against that MDG-with-the-same-tokenizer saves only **~6%** of positions (72.6 vs 76.9). That is far below the pre-registered 30% bar.

With packing (one position per fact), MDG used **61% fewer positions than the best text baseline** (30 vs 77) and scored far higher: **66.9 ± 1.3% vs 30.0 ± 0.6%**. So the pre-registered K1 nominally **passes**. But it passes only because every token-level model, text and MDG alike, failed to train at this tiny CPU budget (all scored 25–32%, close to chance). Packed compact English reached **64.0 ± 2.5% with the same 30 positions**.

The honest reading has three parts:
- *"one position per proposition"* is a real and large efficiency lever in this regime;
- that lever is not new (Meta's Large Concept Models; gist/ICAE context compression);
- it is not specific to MDG.

What would be genuinely new is evidence that the typed temporal, epistemic and provenance graph beats an equally packed, equally learned compression of text. That has not been shown by the pre-registered runs. One **exploratory** follow-up, run post hoc at 3× the training budget, gave a lead. MDG-packed reached **98.0 ± 0.2%** on all 3 seeds. Packed compact English reached **88.1 ± 6.5%**: it learned the "when did X last go to Y" skill in only 1 of 3 seeds. That points to typed codes making facts *more reliably learnable*, possibly just because they disambiguate roles (e.g. event time vs cause reference). It is n=3, post hoc and not significant (Welch t ≈ 2.6), so it is a lead, not a finding. §6 below sets out exactly how to test it on your 3090 Tis.

On the prior art: the components of MDG are all well covered:
- graph meaning: AMR/UMR;
- temporal, provenance and statements-about-statements facts: RDF-star, PROV-O, temporal KGs;
- concept-level sequence models: LCM;
- learned discrete codes: VQ-VAE, Token Assorted;
- structured agent memory: Zep/Graphiti.

The *combination* is the novel bet. MDG's most defensible near-term value is as a **memory/world-state substrate for YantrikDB**, not yet as a more efficient language for the model.

---

## 1. Prior art

**How the citations were checked.** This sandbox's egress proxy blocks arxiv.org, aclanthology.org, w3.org, openreview.net, dl.acm.org and Semantic Scholar (direct `curl` and WebFetch both returned HTTP 403 on CONNECT; I re-checked this myself). So no page below was fetched directly. Every entry was checked by web search: the canonical page (arXiv abs, ACL Anthology, W3C TR, PMLR or ACM DOI) came back with matching title, authors, venue and abstract text, and the summary comes from that abstract. I re-ran the searches for the two most load-bearing entries (Jin et al. 2024 and Token Assorted) and they matched. This is weaker than reading the PDFs. Before relying on a page number, venue or specific claim, open the link. Anything I could not confirm is listed as unverified at the end of this section and is not used in the argument.

### 1.1 Abstract Meaning Representation (AMR), text→AMR and AMR→text
- Banarescu et al. (2013). *Abstract Meaning Representation for Sembanking*. LAW 2013. https://aclanthology.org/W13-2322/ — Introduces AMR, which writes a sentence's meaning as a single rooted, directed, labelled graph.
- Konstas et al. (2017). *Neural AMR: Sequence-to-Sequence Models for Parsing and Generation*. ACL 2017. https://aclanthology.org/P17-1014/ — Seq2seq AMR parsing and AMR→text over a linearised graph. Reports robustness to the order in which the graph is linearised.
- Bevilacqua, Blloshmi & Navigli (2021). *One SPRING to Rule Them Both*. AAAI 2021. https://ojs.aaai.org/index.php/AAAI/article/view/17489 — Runs text→AMR and AMR→text as symmetric seq2seq on BART, using a carefully designed linearisation.
- Bai, Chen & Zhang (2022). *Graph Pre-training for AMR Parsing and Generation* (AMRBART). ACL 2022. https://aclanthology.org/2022.acl-long.415/
- Jin et al. (2024). *Analyzing the Role of Semantic Representations in the Era of Large Language Models*. NAACL 2024. https://aclanthology.org/2024.naacl-long.209/ — Adding AMR to LLM input (AMRCoT) generally does **not** help across five tasks, and often hurts.
- Zhang et al. (2025). *SR-LLM: Rethinking the Structured Representation in Large Language Model*. ACL 2025. https://aclanthology.org/2025.acl-long.172/ — Feeding raw structure (AMR/RDF/code) to LLMs can impede reasoning. Gains come only when the structure is rendered as natural-language descriptions or the model is fine-tuned.

**Overlap.** AMR is the closest well-established ancestor of MDG's "meaning as a typed graph": entities, predicates, roles and reentrancy, with an encoder/decoder pair (parser/generator) that matches MDG Phase 3.
**Difference.** AMR is sentence-level. It has no first-class time validity, no epistemic mode, confidence or provenance, and no conflict representation. It is serialised as human-readable PENMAN, not learned codes. MDG's world-state, temporal and epistemic layers go beyond AMR.
**Lesson for MDG.** The best existing evidence on "give an LLM a semantic graph instead of, or alongside, text" is negative or mixed (Jin 2024; SR-LLM 2025). MDG must beat this without an LLM pretrained on text, or explain why its setting differs.

### 1.2 RDF, statements about statements, Wikidata, temporal KGs, provenance
- Cyganiak, Wood & Lanthaler, eds. (2014). *RDF 1.1 Concepts and Abstract Syntax*. W3C Rec. https://www.w3.org/TR/rdf11-concepts/ — Defines graphs as sets of subject–predicate–object triples.
- Hartig et al., eds. (2021). *RDF-star and SPARQL-star*. W3C CG Final Report. https://www.w3.org/2021/12/rdf-star.html — Lets a triple be annotated by another triple (statements about statements), which is how confidence, provenance and validity attach to a fact. Its successor is RDF 1.2 (https://www.w3.org/TR/rdf12-concepts/, "triple terms" and reifiers). I could not confirm RDF 1.2's current maturity level.
- Lebo, Sahoo & McGuinness, eds. (2013). *PROV-O: The PROV Ontology*. W3C Rec. https://www.w3.org/TR/prov-o/ — A standard vocabulary for provenance (entities, activities, agents).
- Vrandečić & Krötzsch (2014). *Wikidata: A Free Collaborative Knowledgebase*. CACM 57(10). https://doi.org/10.1145/2629489 — Cited for the Wikidata statement model. Only the abstract was confirmed; the details of qualifiers and ranks were not.
- Leblay & Chekol (2018). *Deriving Validity Time in Knowledge Graph* (TTransE). WWW'18 Companion. https://doi.org/10.1145/3184558.3191639 — KG edges with validity intervals.
- Dasgupta, Ray & Talukdar (2018). *HyTE*. EMNLP 2018. https://aclanthology.org/D18-1225/ — Time-aware KG embeddings (one hyperplane per timestamp).
- Trivedi et al. (2017). *Know-Evolve*. ICML 2017. https://proceedings.mlr.press/v70/trivedi17a.html — Temporal KG facts modelled as a point process.
- Cai et al. (2023). *Temporal Knowledge Graph Completion: A Survey*. IJCAI 2023 survey track.

**Overlap.** MDG §5–§11 (entities, relations, states with `valid_from`/`valid_until`, provenance, confidence, conflict) is, almost item for item, expressible in RDF-star/RDF 1.2 + PROV-O + temporal-KG conventions, or in Wikidata's qualifier model. The spec itself names "RDF with different names" as a failure mode (§28).
**Difference.** RDF is an exchange and storage model, not an input format optimised for a transformer, and it has no learned codes. The new part of MDG is not the graph schema. It is the claim that a model *consuming a learned compression of such a graph* is more compute-efficient than one consuming text.

### 1.3 Meta's Large Concept Models (LCM)
- LCM team, Barrault et al. (2024). *Large Concept Models: Language Modeling in a Sentence Representation Space*. arXiv 2412.08821. https://arxiv.org/abs/2412.08821 — Autoregressive modelling over sentence embeddings (SONAR) instead of tokens, including a quantised-SONAR variant.
- Duquenne, Schwenk & Sagot (2023). *SONAR*. arXiv 2308.11466. https://arxiv.org/abs/2308.11466 — Fixed-size multilingual and multimodal sentence embeddings.

**Overlap.** This is the strongest precedent for "one position per unit of meaning rather than per word". It is exactly the packing that makes `mdg_packed` short in my experiment, and it is also language-agnostic and multimodal (MDG §21).
**Difference.** LCM's units are *sentences* embedded in an untyped continuous space. MDG's units are *typed facts* with explicit slots (time, mode, confidence, provenance) and discrete codes. LCM therefore already tests the "fewer positions" half of the MDG idea. The open question is whether typed structure adds anything beyond chunking. The K2 control in §5 tests exactly that.

### 1.4 Continuous latent reasoning
- Hao et al. (2024). *Training Large Language Models to Reason in a Continuous Latent Space* (Coconut). arXiv 2412.06769. https://arxiv.org/abs/2412.06769 — Feeds hidden states back as inputs, which gives fewer reasoning tokens on planning-heavy tasks.
- Goyal et al. (2024). *Think before you speak: Training Language Models With Pause Tokens*. ICLR 2024. https://arxiv.org/abs/2310.02226
- Deng et al. (2023). *Implicit Chain of Thought Reasoning via Knowledge Distillation*. arXiv 2311.01460; Deng, Choi & Shieber (2024). *From Explicit CoT to Implicit CoT*. arXiv 2405.14838.
- Cheng & Van Durme (2024). *Compressed Chain of Thought*. arXiv 2412.13171.

**Overlap.** These works share MDG's premise that the model need not "think in human tokens".
**Difference.** They compress the *reasoning trace* (the output side) into continuous states. They do not give the *input/world state* an explicit structure. MDG is about the representation of context and memory. The two are complementary, and the pause-token result is a warning: fewer positions can mean less computation per answer, so position savings can cost accuracy on multi-hop questions.

### 1.5 VQ-VAE and learned discrete codes for language; context compression
- van den Oord, Vinyals & Kavukcuoglu (2017). *Neural Discrete Representation Learning* (VQ-VAE). NeurIPS 2017. https://arxiv.org/abs/1711.00937
- Su et al. (2025). *Token Assorted: Mixing Latent and Text Tokens for Improved Language Model Reasoning*. arXiv 2502.03275. https://arxiv.org/abs/2502.03275 — VQ-VAE codes replace early chain-of-thought (CoT) text, giving ~17% shorter traces with no loss of accuracy (per the abstract and the authors' summary).
- Pagnoni et al. (2024). *Byte Latent Transformer: Patches Scale Better Than Tokens*. arXiv 2412.09871. https://arxiv.org/abs/2412.09871 — Learned, entropy-based variable-length patches, i.e. "compute proportional to information" (MDG §18) on bytes.
- Mu, Li & Goodman (2023). *Learning to Compress Prompts with Gist Tokens*. NeurIPS 2023. https://arxiv.org/abs/2304.08467 — Up to 26× prompt compression.
- Ge et al. (2024). *In-context Autoencoder* (ICAE). ICLR 2024. https://arxiv.org/abs/2307.06945 — About 4× context compression into memory slots.
- Chevalier et al. (2023). *Adapting Language Models to Compress Contexts* (AutoCompressors). EMNLP 2023. https://aclanthology.org/2023.emnlp-main.232/
- Jiang et al. (2023). *LLMLingua*. EMNLP 2023. https://aclanthology.org/2023.emnlp-main.825/ — Up to 20× prompt compression by dropping tokens.

**Overlap.** MDG §16–§18 (learned, contextual, variable-length discrete codes) is VQ-VAE plus BLT-style variable-length patching applied to semantic units. Gist, ICAE and AutoCompressor already deliver large position reductions on *text* context with a learned encoder.
**Difference.** These systems compress text with *no explicit schema*. MDG inserts a typed graph between the text and the codes. That gives a hard bar: **MDG has to beat gist/ICAE-style learned compression of the same text at the same position budget.** Beating raw text is not enough, because raw text is not the strongest baseline.

### 1.6 Structured state and memory for LLM agents
- Packer et al. (2023). *MemGPT*. arXiv 2310.08560. https://arxiv.org/abs/2310.08560
- Park et al. (2023). *Generative Agents*. UIST 2023. https://arxiv.org/abs/2304.03442 — The memory stream is kept as natural language.
- Rasmussen et al. (2025). *Zep: A Temporal Knowledge Graph Architecture for Agent Memory*. arXiv 2501.13956. https://arxiv.org/abs/2501.13956 — Bi-temporal KG memory (Graphiti) for agents. This is very close to MDG §23 / YantrikDB.
- Gutiérrez et al. (2024). *HippoRAG*. NeurIPS 2024. https://arxiv.org/abs/2405.14831
- Edge et al. (2024). *GraphRAG: From Local to Global*. arXiv 2404.16130. https://arxiv.org/abs/2404.16130

**Overlap.** "Agent memory as a temporal KG with validity and provenance" already exists and works (Zep/Graphiti). MDG §23 restates it.
**Difference.** These systems *store* structure but *render it to text* before the LLM reads it. MDG proposes that the model reads the structure natively. That native reading is the untested part.

### 1.7 Graph-to-sequence and graph encoding for LMs
- Beck, Haffari & Cohn (2018). *Graph-to-Sequence Learning using Gated GNNs*. ACL 2018. https://aclanthology.org/P18-1026/
- Song et al. (2018). *A Graph-to-Sequence Model for AMR-to-Text Generation*. ACL 2018. https://aclanthology.org/P18-1150/
- Wang, Wan & Jin (2020). *AMR-to-Text Generation with Graph Transformer*. TACL 8. https://aclanthology.org/2020.tacl-1.2/
- Ribeiro et al. (2021). *Investigating Pretrained Language Models for Graph-to-Text Generation*. NLP4ConvAI 2021. https://aclanthology.org/2021.nlp4convai-1.20/ — Plain linearised graphs fed to BART/T5 work very well.
- Fatemi, Halcrow & Perozzi (2024). *Talk like a Graph*. ICLR 2024. https://arxiv.org/abs/2310.04560 — LLM accuracy on graph tasks depends strongly (4.8–61.8%) on which text encoding of the graph is used.
- Perozzi et al. (2024). *Let Your Graph Do the Talking* (GraphToken). arXiv 2402.05862. https://arxiv.org/abs/2402.05862 — A learned graph encoder produces a few soft tokens for a frozen LLM.

**Overlap.** MDG's "serialise the graph for the transformer" is exactly this literature. GraphToken is effectively "graph → small number of learned positions → LLM".
**Difference.** These works target graph tasks or graph-to-text, not general world-state reasoning with time and epistemics. They show that the serialisation choice moves accuracy a lot. MDG cannot treat ordering as "an implementation detail" (spec §4) until it has been measured.

### 1.8 Semantic parsing
- Zettlemoyer & Collins (2005). *Learning to Map Sentences to Logical Form*. UAI 2005. https://arxiv.org/abs/1207.1420
- Lake & Baroni (2018). *Generalization without Systematicity* (SCAN). ICML 2018. https://proceedings.mlr.press/v80/lake18a.html
- Kim & Linzen (2020). *COGS*. EMNLP 2020. https://aclanthology.org/2020.emnlp-main.731/ — 96–99% in-distribution vs 16–35% on the generalisation split.
- Andreas et al. (2020). *Task-Oriented Dialogue as Dataflow Synthesis* (SMCalFlow). TACL 8. https://aclanthology.org/2020.tacl-1.36/ — Dialogue state as a dataflow graph, with explicit reference and revision operators.

**Relevance.** MDG's NL→MDG encoder *is* a semantic parser into a broad-coverage target. Decades of results say such parsers are brittle out of distribution (SCAN/COGS). The encoder, not the downstream model, is the likely bottleneck. SMCalFlow's revision operators are a worked example of MDG §11 "conflict and change".

### 1.9 Synthetic world-state benchmarks and tokenisation (closest to the proposed experiment)
- Weston et al. (2015). *bAbI*. arXiv 1502.05698. https://arxiv.org/abs/1502.05698 — Synthetic entity, location and time QA. My generator is a bAbI-like task family.
- Sinha et al. (2019). *CLUTRR*. EMNLP 2019. https://aclanthology.org/D19-1458/
- Tafjord, Dalvi & Clark (2021). *ProofWriter*. Findings ACL 2021. https://aclanthology.org/2021.findings-acl.317/
- Kim & Schuster (2023). *Entity Tracking in Language Models*. ACL 2023. https://aclanthology.org/2023.acl-long.213/ — Final-state tracking after a sequence of operations. This is the "what's true now" question.
- Allen-Zhu & Li (2024). *Physics of Language Models 3.1*. ICML 2024. https://arxiv.org/abs/2309.14316 — The controlled-synthetic-data methodology.
- Chen, Wang & Wang (2021). *TimeQA*. NeurIPS 2021 D&B. https://arxiv.org/abs/2108.06314; Fatemi et al. (2024). *Test of Time*. arXiv 2406.09170. https://arxiv.org/abs/2406.09170
- Rust et al. (2021). *How Good is Your Tokenizer?* ACL 2021. https://aclanthology.org/2021.acl-long.243/ — A dedicated tokenizer improves downstream results.
- Petrov et al. (2023). *Language Model Tokenizers Introduce Unfairness Between Languages*. NeurIPS 2023. https://arxiv.org/abs/2305.15425 — The same content costs up to 15× more tokens depending on the language. Position count is a property of the *tokenizer*, not only of the representation.

### 1.10 Closer to MDG: interlinguas, knowledge-representation languages, machine-to-machine languages
- Van Gysel et al. (2021). *Designing a Uniform Meaning Representation for NLP* (UMR). KI 35. https://doi.org/10.1007/s13218-021-00722-w — AMR extended across languages, with document-level coreference and **temporal and modal (epistemic) dependencies**. This is the closest *linguistic* precursor to MDG's time and epistemic-mode dimensions.
- Lenat (1995). *CYC: A Large-Scale Investment in Knowledge Infrastructure*. CACM 38(11). — A hand-built universal ontology with microtheories (contexts). This is the cautionary tale behind MDG §28's "not a giant fixed ontology".
- Finin et al. (1994). *KQML as an Agent Communication Language*. CIKM'94. https://doi.org/10.1145/191246.191322; FIPA (2002). *ACL Message Structure Specification* SC00061G. https://www.fipa.org/specs/fipa00061/SC00061G.html — Machine-to-machine semantic message formats with performatives (which roughly match MDG's REQUEST and epistemic mode).
- Lazaridou & Baroni (2020). *Emergent Multi-Agent Communication in the Deep Learning Era*. arXiv 2006.02419.
- Pham et al. (2024). *Let Models Speak Ciphers* (CIPHER). ICLR 2024. https://arxiv.org/abs/2310.06272; Zheng et al. (2025). *Thought Communication in Multiagent Collaboration*. arXiv 2510.20733; Du et al. (2025). *Interlat: Enabling Agents to Communicate Entirely in Latent Space*. arXiv 2511.09149; LatentMAS (2025). *Latent Collaboration in Multi-Agent Systems*. arXiv 2511.20639 (first author not confirmed) — LLM agents exchanging embeddings or hidden states instead of text. These report fewer tokens and faster inference. They form the *learned, schema-free* end of the "machine-native language" idea.

**Unverified / not used.** UNL (Universal Networking Language, Uchida & Zhu, UNU) is a relevant 1990s interlingua, but I found no canonical page to confirm it, so it carries no weight here. I did not confirm the venues of Test of Time, Token Assorted and MemGPT, so they are cited as arXiv. I did not confirm the RDF 1.2 maturity level. I found no paper that does exactly "typed temporal/epistemic graph → learned discrete codes → transformer, with an efficiency comparison against text". That absence is noted, not claimed as proof.

---

## 2. Novelty, stated honestly

**Not new**, each with a direct precedent in §1:
- meaning as a typed graph (AMR, UMR, RDF);
- states with temporal validity (temporal KGs, Wikidata qualifiers, Zep's bi-temporal edges);
- provenance and confidence on facts (PROV-O, RDF-star);
- epistemic and modal marking (UMR modal dependencies; FIPA/KQML performatives);
- one position per unit of meaning (LCM);
- learned discrete codes (VQ-VAE, Token Assorted, quantised SONAR in LCM);
- variable-length codes and compute proportional to information (BLT patches);
- structured world state as agent memory (Zep, MemGPT, HippoRAG, GraphRAG);
- machine-to-machine semantic messages (KQML/FIPA-ACL; latent inter-agent channels such as CIPHER, Interlat, LatentMAS).

**Possibly new: the specific combination and the specific claim.**
> A single, typed, *world-level* semantic graph that carries time validity, epistemic mode, confidence, provenance and conflict as first-class slots, compressed into learned discrete codes, and used as the **native input/working representation of the transformer**, not stored and then re-rendered to text. The claim is that this is more compute-efficient per unit of task accuracy than text, including text that has been compressed by learned methods.

I found no paper that tests this combination end-to-end against a strong compressed-text baseline. The nearest work tests only pieces of it:
- GraphToken: graph → soft tokens, but for graph tasks.
- Token Assorted: VQ codes, but for reasoning traces.
- LCM: concept positions, but untyped.
- Zep: a temporal KG, but rendered back to text.

The absence of such a paper is not proof of novelty.

**What MDG must show to be more than a recombination:** that the *types* matter. At a matched position budget and matched learned compression, the typed slots (time, mode, provenance) should buy accuracy, generalisation, or editability that an untyped compression of the same facts does not. R1's packed comparison is the first, small test of that. It came out **+2.9 pts, not detectable** at n=3.

---

## 3. The main confound and how to design around it

**The confound.** In the full MDG pipeline (NL → encoder → MDG → model), the encoder can do the hard work: resolving coreference, ordering events, closing state intervals (`valid_until`), even following causal chains. Then the model downstream of MDG looks efficient only because the reasoning moved upstream, and the encoder's own compute is not counted. A second form of the same confound is a baseline that gets *less help*, for example raw prose with pronouns against MDG with canonical IDs.

**Design rules used in R1, and recommended for every later experiment.**
1. **No parser in the loop for the core test.** Worlds are generated as ground-truth event graphs (`worlds.py`), and every rendering is produced *from* that graph. MDG accuracy can therefore not be inflated by parser cleverness or deflated by parser errors.
2. **No precomputed answers in any rendering.** The MDG carries only events (with time and cause links), never derived state:
   - no `valid_until` on locations;
   - no "current holder";
   - no root-cause actor copied onto the effect.

   Every question type (where now, where at t, who holds, when, root cause via a 2-hop causal chain, device state now) requires the model to integrate the events itself. Adding a resolved-state field would make `where_now` a lookup, and it is deliberately excluded.
3. **Baselines get the same help.**
   - `compact`: English with resolved references, canonical entity names and explicit time ids, one fact per clause. `check_same_information()` asserts that compact and MDG carry the same facts, with the same entities and times.
   - Domain tokenizer: word-level tokens, so every name, place and time is one token, plus a 200-merge **domain-trained BPE**. The same BPE budget is given to MDG.
   - Same packing: `compact_packed` uses the identical one-position-per-fact mechanism as `mdg_packed`.
4. **Same model, same data.** Identical architecture, parameter count (311k), optimiser, schedule and number of training examples (256k fresh worlds). The test set is identical: 2000 worlds, 11,991 questions.
5. **Count everything.** Positions, forward FLOPs, KV-cache size and wall-clock are reported. In real-text experiments, encoder FLOPs must be charged to the MDG condition, and an equally strong *learned text compressor* (gist/ICAE-style) is the baseline to beat, not raw text.
6. **Pre-register.** Kill criteria were committed (`PREREGISTRATION.md`, commit `08a2cb4`) before the main runs. The one change after a disclosed pilot (training budget) was also committed before the main runs (Amendment 1).

---

## 4. A falsifiable first experiment (pre-registered, then run)

**Task family.** These are procedurally generated "day in a building" worlds:
- 6 people (sampled from 20 names), 5 places (from 12), 3 objects (from 10) and 2 devices (from 6);
- an initial state at t0, then 16 timed events: moves, hand-overs, device on/off, and *causal chains* where a person triggers a mechanism (breaker, flood, fire, outage) that later knocks out a device;
- six question types with exact answers from the generator: `where_now`, `where_at` (time t), `holder_now`, `when_moved`, `root_cause` (2-hop: device-off → cause event → actor) and `dev_now`.

Answers are classified over a shared 59-way answer vocabulary.

**Conditions** (an example of each rendering is in `experiment/README.md` / `data.py`):

| id | rendering | mean positions |
|---|---|---|
| plain | verbose English: pronouns, paraphrases, clock times, filler; word tokens | 290.2 |
| compact | compact English, resolved refs, canonical names, `t5` time ids; word tokens | 169.5 |
| compact_bpe | compact + 200-merge domain BPE | 76.9 |
| mdg_seq | MDG typed tuples, one atomic code per slot (`V_MOVE E_lena E_kitchen T1`) | 112.7 |
| mdg_seq_bpe | MDG + same-budget BPE | 72.6 |
| mdg_packed | one position per MDG fact: Σ(code emb + slot emb) | 30.0 |
| compact_packed | one position per compact sentence, same mechanism | 30.0 |

**Model.** A pre-LN Transformer encoder with d=64, 3 layers, 4 heads, FFN 256 and learned positional embeddings: 311k parameters, identical for all conditions. It was trained with AdamW (lr 1e-3, warmup + cosine) at batch 32 for 8000 steps, using a fresh world each example. The model reads out at a final `<read>` position. Training seeds were 0, 1 and 2; `plain` used seed 0 only because of the CPU budget (pre-declared). The test generator seed was 10^6.

**Metrics.** Test accuracy (mean ± sample sd over seeds), mean positions per example, forward FLOPs per example (≈ layers·(24·L·d² + 4·L²·d)) and KV floats per example (2·layers·L·d).

**Kill criteria (fixed before the runs).**
- **K1:** let M\* be the best MDG condition and T\* the best text condition among {compact, compact_bpe}, each chosen as the fewest positions within 1 pt of its family's best accuracy. K1 passes only if acc(M\*) ≥ acc(T\*) − 1 pt **and** pos(M\*) ≤ 0.70 · pos(T\*).
- **K2:** a pass via packing counts as MDG-specific only if `compact_packed` does *not* come within 1 pt of `mdg_packed`. Differences under 2× the seed sd are reported as "no detectable difference".
- **K3:** report MDG vs BPE'd text at the serialisation level as-is.

---

## 5. Results (actually run, CPU sandbox)

All 19 pre-registered runs completed on 4 CPU cores (≈2.5 h wall-clock). Raw per-run JSON is in `experiment/results/`. The table below is `experiment/results_main.md`, regenerated with `python3 analyze.py`.

| condition | seeds | accuracy % (mean ± sd) | per-seed | positions | fwd MFLOPs/ex | KV floats/ex |
|---|---|---|---|---|---|---|
| plain | 1 | 25.1 | 25.1 | 290.2 | 150.3 | 111,442 |
| compact | 3 | 29.5 ± 1.2 | 28.5, 29.2, 30.8 | 169.5 | 72.1 | 65,090 |
| compact_bpe | 3 | 30.0 ± 0.6 | 30.6, 29.6, 29.7 | 76.9 | 27.2 | 29,512 |
| mdg_seq | 3 | 30.9 ± 2.0 | 29.6, 29.9, 33.2 | 112.7 | 43.0 | 43,260 |
| mdg_seq_bpe | 3 | 31.8 ± 0.5 | 31.2, 32.2, 32.1 | 72.6 | 25.4 | 27,861 |
| **mdg_packed** | 3 | **66.9 ± 1.3** | 65.6, 68.3, 66.6 | **30.0** | 9.5 | 11,520 |
| **compact_packed** | 3 | **64.0 ± 2.5** | 61.1, 65.8, 65.0 | **30.0** | 9.5 | 11,520 |

Per question type (accuracy %, mean over seeds):

| condition | where_now | where_at | holder_now | when_moved | root_cause | dev_now |
|---|---|---|---|---|---|---|
| plain | 21.9 | 20.2 | 17.2 | 5.3 | 18.5 | 67.4 |
| compact | 29.5 | 27.3 | 21.0 | 5.6 | 23.5 | 70.0 |
| compact_bpe | 30.6 | 30.9 | 21.0 | 5.1 | 24.7 | 67.4 |
| mdg_seq | 30.9 | 30.0 | 22.3 | 5.2 | 26.5 | 70.5 |
| mdg_seq_bpe | 31.3 | 28.1 | 32.9 | 5.7 | 25.6 | 67.4 |
| mdg_packed | 95.8 | 85.4 | 54.3 | 4.9 | 65.7 | 94.8 |
| compact_packed | 93.8 | 81.6 | 48.1 | 5.2 | 62.7 | 92.4 |

Approximate chance levels: `where_*` 20% (5 places); `holder_now` / `root_cause` ≈17% (6 people); `dev_now` is ~65–70% for always guessing the majority class; `when_moved` ≈ 1/16 (event times).

### Kill-criteria verdicts
- **K1: nominal PASS, substantively uninformative.** T\* = compact_bpe (30.0%, 76.9 positions) and M\* = mdg_packed (66.9%, 30.0 positions), giving +36.9 pts with a position ratio of 0.39. But T\* is at chance-level on 4 of 6 question types; the baseline did not train. K1 was written assuming both sides learn the task. A pass against an untrained baseline measures *trainability at a tiny compute budget*, not representational efficiency. I do not count it as support for the hypothesis.
- **K2: NOT established.** The gap is mdg_packed − compact_packed = +2.9 pts. The pre-registered noise rule (2·sd = 5.0) classes it as **no detectable difference** (Welch t = 1.77, n = 3 vs 3). The literal 1-pt rule would credit MDG, so the two pre-registered rules disagree here. I report the conservative one. **Packing explains ~35 of the ~37 points; the MDG schema explains at most ~3, and not detectably.** The direction is consistent across all six question types, which is weak evidence worth re-testing with more seeds.
- **K3: against the hypothesis at the serialisation level.** Comparing serialisations:
  - mdg_seq (112.7 positions) is *longer* than compact_bpe (76.9).
  - With the same BPE budget, MDG is 72.6 vs 76.9, only **5.6% fewer** positions. That is far short of 30%.
  - mdg_seq_bpe scored +1.9 pts over compact_bpe (2·sd = 1.1, so detectable), but both are near chance, so the difference means little.

**What this says.** Writing the same facts as typed codes instead of compact English barely shortens the sequence once the text gets a domain tokenizer. Most of what the spec's "learned codes" would achieve at the token level, a 200-merge BPE already achieves. The big win is changing the *unit of position* from token to proposition. That lever works for compact text as well as for MDG, provided something segments the text into propositions; here the segmentation is an oracle, which is itself a form of help.

### Exploratory, post-hoc (NOT pre-registered): is the token-level failure just under-training?
These runs were added *after* seeing the main results, so they are hypothesis-generating, not confirmatory. Settings are the same except for **24,000 steps (3× the budget)**. The packed conditions have 3 seeds; the BPE token conditions have seed 0 only. Raw data: `experiment/results_explore_24k/`, table: `experiment/results_explore_24k.md`.

| condition | seeds | accuracy % | per-seed | positions |
|---|---|---|---|---|
| compact_bpe | 1 | 32.3 | 32.3 | 76.9 |
| mdg_seq_bpe | 1 | 37.6 | 37.6 | 72.6 |
| mdg_packed | 3 | **98.0 ± 0.2** | 97.8, 98.1, 98.2 | 30.0 |
| compact_packed | 3 | **88.1 ± 6.5** | 86.0, 82.8, 95.4 | 30.0 |

Per seed, the packed conditions differ on two question types:

| run | when_moved | root_cause | holder_now | others |
|---|---|---|---|---|
| mdg_packed s0 / s1 / s2 | 99.8 / 100.0 / 100.0 | 99.6 / 100.0 / 99.7 | 89.1 / 89.5 / 90.8 | ≥98.3 |
| compact_packed s0 / s1 / s2 | 34.5 / **5.5** / 99.7 | 99.7 / 99.8 / **85.2** | 86.8 / 93.5 / 90.5 | ≥95.9 |

What this shows, cautiously:
1. **The token-level failure is not fixed by 3× more training at this model size.** Both BPE'd sequences stay near chance. The packing advantage at this scale is therefore not just "trained a bit longer". It probably reflects a tiny 3-layer model being unable to compose token-level bindings (name → verb → place → time across positions) without many more layers or steps. A bigger model should close much of this gap, and that is the first thing R2 must check.
2. **A lead for the typed schema.** At equal positions, MDG facts were learned *reliably*: all 3 seeds solved every skill except the shared `holder_now` ceiling of ~90%. The same facts written as packed words solved `when_moved` in only 1 of 3 seeds, and one seed regressed on `root_cause`. The mean gap is +9.9 pts (Welch t ≈ 2.6, n=3 vs 3), which is not significant at conventional levels, and the variance difference (sd 0.2 vs 6.5) is the more striking feature. One plausible mechanism is **typed codes that disambiguate roles**. In compact text the same word `t5` serves both as an event's own time and as a cause reference ("… because of t5"). MDG uses distinct `T5` and `C_T5` codes, and a single relation code rather than "goes to" / "is in". This can be tested directly: give the text baseline distinct cause-reference words and see whether the gap closes. If it closes, the "typed advantage" is just disambiguated vocabulary, which text can also have.
3. This does **not** change the pre-registered verdicts above (K1 uninformative, K2 not established, K3 against). It changes what R2 should prioritise.

### Limitations (read before quoting any number)
- **Tiny scale.**
  - 311k parameters on CPU, and the model is under-trained even in the best condition.
  - `when_moved` was never learned by any condition (~5%), and `holder_now` is weak.
  - At scale, token-level models usually *can* do bAbI-style tracking, so the 35-point packing gap is very likely a small-model / low-compute effect. The FLOP and KV savings (≈3×, 2.6× vs compact_bpe) are structural and would survive.
- **Oracle segmentation.**
  - The packed conditions are given exact fact boundaries. Real text needs a segmenter or encoder, whose cost and errors are not counted here.
  - The packing (a sum of slot-tagged embeddings) is one design among many; gist/ICAE-style learned compression is the stronger text competitor and was not run.
- **Seeds.** n = 3 per condition (n = 1 for `plain`), and the sds are sample sds over 3 seeds. Test-set sampling error is small by comparison (~12k questions → ±0.9 pt 95% CI at 65%).
- **Tasks.** Only time, state, ownership and causal-chain questions were tested. The epistemic, confidence, provenance and conflict dimensions, which are MDG's most distinctive features, were **not** exercised.
- **Pilot disclosure.** Three pilots on `mdg_packed` only were run to set the training budget (36.0% at 2k steps, 66.8% at 8k steps, 49.2% for a larger model at 4k steps). They were recorded in Amendment 1 before the main runs.
- **Matching.** Training was matched on examples, not FLOPs; matching FLOPs would favour the short conditions *more*. I could not check GPT-style tokenizer counts for `plain` because the tiktoken download is blocked by the proxy.

---

## 6. What would change the conclusion, and the next 3 experiments

**Would move me toward "MDG works":**
- The exploratory reliability gap (98.0 ± 0.2 vs 88.1 ± 6.5 at 24k steps) replicates with ≥5 pre-registered seeds, *and* survives giving the text baseline role-disambiguated words (e.g. `cause-t5` vs `t5`). If disambiguated text closes the gap, the advantage belongs to "unambiguous symbols", which any well-designed text format can have, not to MDG as such.
- With ≥5 seeds, `mdg_packed` beats `compact_packed` by more than the noise, *and* the gap grows on epistemic/conflict/provenance questions where typed slots should matter.
- MDG keeps an accuracy-per-FLOP advantage over a **learned** text compressor (gist/ICAE-style, one or a few vectors per sentence) after both are trained to convergence.
- MDG generalises better out of distribution (longer worlds, unseen entity names, new event compositions), which is the compositionality claim of §14/§27.

**Would move me toward "MDG is a good memory schema but not a better model language":**
- With adequate compute, token-level text reaches the same accuracy, so packing buys only FLOPs, not capability.
- Learned text chunking matches packed MDG.
- The NL→MDG encoder's errors on real text cost more accuracy than the downstream savings buy back.

**Next 3 experiments (in order):**
1. **R2: scale up and converge, on one 3090 Ti.**
   - Same generator; d=256, 6 layers, 30k steps, 5 seeds (command in `experiment/README.md`).
   - Plot accuracy against training FLOPs for all 7 conditions, and add two learned-compression text baselines:
     - (a) a per-sentence encoder that pools each compact sentence to one vector (learned segmentation-free variant: fixed-size windows);
     - (b) gist tokens: k learned summary positions per window.
   - Decision rule: MDG must Pareto-dominate (b) at ≥30% fewer positions, else K1 fails.
   - Pre-register the packed-reliability comparison from the exploratory runs: `mdg_packed` vs `compact_packed` vs `compact_packed_disambiguated`, 5 seeds each. CPU is enough for this sub-test (~30 min per run at 24k steps).
2. **R3: test the dimensions only MDG has.**
   - Extend `worlds.py` with sources of differing reliability, contradictory reports, beliefs ("Bob thinks the key is in the office"), retractions and confidence.
   - Ask "what is true", "what does X believe", "which source is wrong" and "how confident".
   - Compare `mdg_packed` (typed mode/provenance slots) against `compact_packed` (the same facts in words) and an untyped-code ablation, with the same pre-registration discipline.
   - This is where a typed schema has the best a-priori chance to matter.
3. **R4: the real pipeline, with the encoder's cost charged.**
   - Take a real-text task (bAbI-style or the Kim & Schuster 2023 entity-tracking data) and build an NL→MDG encoder (a prompted LLM or a fine-tuned parser).
   - Count encoder FLOPs and errors, and compare end-to-end against (i) the same LLM reading text and (ii) ICAE/gist compression of the text at the same total compute.
   - Separately, replace symbolic MDG codes with learned VQ codes (§16) and measure how far the codebook can shrink before accuracy drops. That is the spec's §32 follow-up question.

**Recommendation.** Treat MDG's schema (§5–§11) as the design for YantrikDB's world-state and memory layer now; it is well-founded and has strong precedent (Zep/Graphiti, PROV-O). Treat "MDG as the model's native language" as an open research bet that has to win R2 and R3 before any architecture work.
