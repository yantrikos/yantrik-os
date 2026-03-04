//! Bulk-load 10K realistic personal memories into YantrikDB and benchmark.
//!
//! Uses real CandleEmbedder for genuine semantic embeddings.
//! Generates diverse memories spanning months of a simulated personal history.
//!
//! Run: cargo run -p yantrikdb-ml --example bulk_load --release

use std::time::Instant;
use yantrikdb_ml::CandleEmbedder;
use yantrikdb_core::types::ThinkConfig;

const EMBEDDER_REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";

fn main() {
    let target_count: usize = std::env::var("MEMORY_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    println!("=== YantrikDB Bulk Load Benchmark ===");
    println!("Target memories: {target_count}");
    println!();

    // 1. Load embedder
    println!("Loading CandleEmbedder...");
    let t = Instant::now();
    let embedder = CandleEmbedder::from_hub(EMBEDDER_REPO, None)
        .expect("failed to load embedder");
    println!("  Embedder loaded in {:.2}s", t.elapsed().as_secs_f64());

    // 2. Create YantrikDB
    let db_path = "bench_10k.db";
    // Remove old DB if exists
    let _ = std::fs::remove_file(db_path);
    let mut db = yantrikdb_core::YantrikDB::new(db_path, 384).expect("failed to create YantrikDB");
    db.set_embedder(Box::new(embedder));
    println!("  YantrikDB created at {db_path}");
    println!();

    // 3. Generate and store memories
    let memories = generate_memories(target_count);
    println!("Generated {} memory templates", memories.len());

    println!("Storing memories with real embeddings...");
    let t = Instant::now();
    let mut stored = 0;
    let mut entities_created = 0;
    let batch_report = target_count / 10;

    for (i, mem) in memories.iter().enumerate() {
        let rid = db
            .record_text(
                &mem.text,
                &mem.memory_type,
                mem.importance,
                mem.valence,
                mem.half_life,
                &mem.metadata,
                "default",
                mem.certainty,
                &mem.domain,
                "bulk_load",
                mem.emotional_state.as_deref(),
            )
            .expect("failed to record memory");
        stored += 1;

        // Add entity relationships for some memories
        for (entity, rel) in &mem.entities {
            let _ = db.relate(entity, &rid, rel, 1.0);
            entities_created += 1;
        }

        if (i + 1) % batch_report == 0 {
            let elapsed = t.elapsed().as_secs_f64();
            let rate = (i + 1) as f64 / elapsed;
            println!(
                "  [{}/{}] {:.0} memories/sec, {:.1}s elapsed",
                i + 1,
                target_count,
                rate,
                elapsed
            );
        }
    }

    let store_time = t.elapsed().as_secs_f64();
    let rate = stored as f64 / store_time;
    println!();
    println!("=== Storage Results ===");
    println!("  Stored: {stored} memories");
    println!("  Entities linked: {entities_created}");
    println!("  Time: {store_time:.2}s");
    println!("  Rate: {rate:.1} memories/sec");
    println!();

    // 4. Stats
    let stats = db.stats(None).expect("failed to get stats");
    println!("=== YantrikDB Stats ===");
    println!("  Active memories: {}", stats.active_memories);
    println!("  Entities: {}", stats.entities);
    println!("  Edges: {}", stats.edges);
    println!();

    // 5. Recall benchmarks
    println!("=== Recall Benchmarks ===");
    let queries = vec![
        ("What are Pranab's hobbies?", "personal"),
        ("meetings with the engineering team", "work"),
        ("feeling stressed or anxious", "emotional"),
        ("family dinner or gathering", "family"),
        ("machine learning projects", "technical"),
        ("weekend plans or activities", "leisure"),
        ("health or exercise routine", "health"),
        ("travel to Japan", "travel"),
        ("debugging a production issue", "work"),
        ("birthday celebration", "events"),
        ("favorite restaurants nearby", "local"),
        ("learning Rust programming", "technical"),
        ("conversation with Sarah about the project", "social"),
        ("financial planning or budget", "finance"),
        ("meditation or mindfulness practice", "wellness"),
    ];

    let mut total_recall_ms = 0.0;
    for (query, category) in &queries {
        let t = Instant::now();
        let results = db
            .recall_text(query, 10)
            .expect("recall failed");
        let elapsed_ms = t.elapsed().as_secs_f64() * 1000.0;
        total_recall_ms += elapsed_ms;

        let top_score = results.first().map(|r| r.score).unwrap_or(0.0);
        let top_text = results
            .first()
            .map(|r| {
                if r.text.len() > 60 {
                    format!("{}...", &r.text[..60])
                } else {
                    r.text.clone()
                }
            })
            .unwrap_or_default();
        println!(
            "  [{category:10}] {elapsed_ms:6.1}ms | top={top_score:.3} | {top_text}"
        );
    }
    println!();
    println!(
        "  Avg recall: {:.1}ms per query ({} queries)",
        total_recall_ms / queries.len() as f64,
        queries.len()
    );
    println!();

    // 6. Think cycle benchmark
    println!("=== Think Cycle Benchmark ===");
    let config = ThinkConfig::default();
    let t = Instant::now();
    let result = db.think(&config).expect("think failed");
    let think_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("  Duration: {think_ms:.1}ms");
    println!("  Triggers: {}", result.triggers.len());
    println!("  Consolidations: {}", result.consolidation_count);
    println!("  Conflicts found: {}", result.conflicts_found);
    println!("  New patterns: {}", result.patterns_new);
    println!();

    // 7. Second think cycle (should find consolidation candidates)
    println!("=== Second Think Cycle ===");
    let t = Instant::now();
    let result2 = db.think(&config).expect("think failed");
    let think2_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("  Duration: {think2_ms:.1}ms");
    println!("  Triggers: {}", result2.triggers.len());
    println!("  Consolidations: {}", result2.consolidation_count);
    println!("  Conflicts found: {}", result2.conflicts_found);
    println!("  New patterns: {}", result2.patterns_new);
    println!();

    // 8. Final stats
    let final_stats = db.stats(None).expect("failed to get stats");
    println!("=== Final Stats ===");
    println!("  Active memories: {}", final_stats.active_memories);
    println!("  Consolidated: {}", final_stats.consolidated_memories);
    println!("  Entities: {}", final_stats.entities);
    println!("  Edges: {}", final_stats.edges);

    // 9. Database file size
    let file_size = std::fs::metadata(db_path)
        .map(|m| m.len())
        .unwrap_or(0);
    println!("  DB file size: {:.1} MB", file_size as f64 / 1_048_576.0);
    println!();

    // 10. Recall with graph expansion
    println!("=== Recall with Graph Expansion ===");
    let t = Instant::now();
    let results = db
        .recall_text("What do I know about Pranab?", 10)
        .expect("recall failed");
    let elapsed_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("  Graph-expanded recall: {elapsed_ms:.1}ms");
    println!("  Results: {}", results.len());
    for (i, r) in results.iter().take(5).enumerate() {
        let text = if r.text.len() > 80 {
            format!("{}...", &r.text[..80])
        } else {
            r.text.clone()
        };
        println!("  [{}] score={:.3} | {}", i + 1, r.score, text);
    }
    println!();

    println!("=== Benchmark Complete ===");
    println!("DB file: {db_path}");
}

// ── Memory Generation ───────────────────────────────────

struct MemoryTemplate {
    text: String,
    memory_type: String,
    importance: f64,
    valence: f64,
    half_life: f64,
    certainty: f64,
    domain: String,
    metadata: serde_json::Value,
    emotional_state: Option<String>,
    entities: Vec<(String, String)>,
}

fn generate_memories(count: usize) -> Vec<MemoryTemplate> {
    let mut memories = Vec::with_capacity(count);

    // Personal facts
    let facts = vec![
        ("My name is Pranab and I'm a software engineer.", "Pranab", 0.9),
        ("I live in the Bay Area, California.", "Bay Area", 0.7),
        ("I work at a startup building AI systems.", "startup", 0.8),
        ("My favorite programming language is Rust.", "Rust", 0.6),
        ("I have a golden retriever named Max.", "Max", 0.7),
        ("My partner's name is Maya.", "Maya", 0.9),
        ("I studied computer science at UC Berkeley.", "UC Berkeley", 0.7),
        ("I'm originally from Kolkata, India.", "Kolkata", 0.7),
        ("I prefer dark roast coffee, black.", "coffee", 0.3),
        ("I'm allergic to shellfish.", "health", 0.8),
        ("My birthday is March 15th.", "birthday", 0.6),
        ("I drive a Tesla Model 3.", "Tesla", 0.4),
        ("My favorite author is Asimov.", "Asimov", 0.5),
        ("I speak Bengali, Hindi, and English fluently.", "languages", 0.6),
        ("I'm training for a half marathon this spring.", "marathon", 0.7),
    ];

    // People and relationships
    let people = vec![
        ("Sarah", "colleague", "engineering team lead"),
        ("Ravi", "friend", "college roommate, works at Google"),
        ("Priya", "sister", "lives in London, data scientist"),
        ("Amit", "father", "retired professor of physics"),
        ("Sunita", "mother", "runs a bookshop in Kolkata"),
        ("David", "mentor", "VP of Engineering, advisor"),
        ("Lisa", "colleague", "product manager on our team"),
        ("Chen", "colleague", "ML engineer, joined last month"),
        ("Nadia", "friend", "yoga instructor, neighbor"),
        ("Jake", "friend", "hiking buddy, photographer"),
    ];

    // Activities and hobbies
    let hobbies = vec![
        "chess", "hiking", "cooking", "reading sci-fi", "running",
        "photography", "meditation", "playing guitar", "gardening",
        "woodworking", "board games", "watching cricket",
    ];

    // Work topics
    let work_topics = vec![
        "vector database optimization", "HNSW index performance", "memory consolidation algorithm",
        "Python bindings with PyO3", "candle ML framework", "GGUF model loading",
        "SQLite WAL mode tuning", "embedding model selection", "LLM prompt engineering",
        "knowledge graph traversal", "Rust async runtime", "axum HTTP server",
        "production deployment", "CI/CD pipeline", "code review process",
        "sprint planning", "customer feedback analysis", "API design",
        "benchmarking methodology", "memory leak debugging",
    ];

    // Locations
    let locations = vec![
        "San Francisco", "Palo Alto", "Mountain View", "Berkeley",
        "Half Moon Bay", "Muir Woods", "Yosemite", "Big Sur",
        "Portland", "Seattle", "Tokyo", "Kyoto", "Kolkata", "London",
    ];

    // Foods and restaurants
    let foods = vec![
        "sushi at Nobu", "tacos from the food truck on Valencia",
        "pizza at Tony's", "pho at Turtle Tower", "biryani from Dum",
        "ramen at Mensho", "dosa at Udupi Palace", "dim sum at Yank Sing",
        "burgers at Super Duper", "Thai food from Kin Khao",
    ];

    // Emotions
    let emotions = vec![
        ("happy", 0.8), ("excited", 0.7), ("grateful", 0.6),
        ("stressed", -0.5), ("anxious", -0.6), ("tired", -0.3),
        ("proud", 0.7), ("frustrated", -0.4), ("calm", 0.3),
        ("nostalgic", 0.2), ("motivated", 0.6), ("overwhelmed", -0.5),
    ];

    let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let months = ["January", "February", "March", "April", "May", "June",
                   "July", "August", "September", "October", "November", "December"];
    let times = ["morning", "afternoon", "evening", "night"];

    let mut idx = 0u64;
    let mut rng = SimpleRng::new(42);

    // Helper: pick from slice
    macro_rules! pick {
        ($arr:expr) => { &$arr[rng.next() as usize % $arr.len()] };
    }

    while memories.len() < count {
        idx += 1;
        let category = idx % 20;

        let mem = match category {
            // Personal facts (repeated with variations)
            0 => {
                let (text, entity, imp) = pick!(facts);
                MemoryTemplate {
                    text: text.to_string(),
                    memory_type: "semantic".into(),
                    importance: *imp,
                    valence: 0.3,
                    half_life: 365.0 * 86400.0,
                    certainty: 0.95,
                    domain: "personal".into(),
                    metadata: serde_json::json!({"category": "fact"}),
                    emotional_state: None,
                    entities: vec![(entity.to_string(), "mentioned_in".into())],
                }
            }

            // Work events
            1 | 2 | 3 => {
                let topic = pick!(work_topics);
                let person = pick!(people);
                let day = pick!(days);
                let time = pick!(times);
                let texts = [
                    format!("Had a productive {time} meeting on {day} about {topic} with {}", person.0),
                    format!("{} and I discussed {topic} — made good progress on the implementation", person.0),
                    format!("Spent the {time} debugging {topic}. Finally found the root cause."),
                    format!("Sprint planning {day}: prioritized {topic} for the next two weeks."),
                    format!("Code review with {} on the {topic} PR. Good suggestions for improvement.", person.0),
                    format!("Demo'd the {topic} feature to the team on {day}. Went well."),
                    format!("Ran into a tricky issue with {topic} but {} helped me figure it out.", person.0),
                ];
                let text = pick!(texts);
                let valence = if text.contains("productive") || text.contains("progress") || text.contains("well") {
                    0.5
                } else if text.contains("tricky") || text.contains("debugging") {
                    -0.2
                } else {
                    0.1
                };
                MemoryTemplate {
                    text: text.clone(),
                    memory_type: "episodic".into(),
                    importance: 0.5 + rng.next_f64() * 0.3,
                    valence,
                    half_life: 30.0 * 86400.0,
                    certainty: 0.85,
                    domain: "work".into(),
                    metadata: serde_json::json!({"category": "work", "topic": topic}),
                    emotional_state: Some(if valence > 0.0 { "engaged" } else { "focused" }.into()),
                    entities: vec![
                        (person.0.to_string(), "worked_with".into()),
                        (topic.to_string(), "related_to".into()),
                    ],
                }
            }

            // Social interactions
            4 | 5 => {
                let person = pick!(people);
                let activity = pick!(hobbies);
                let location = pick!(locations);
                let texts = [
                    format!("Caught up with {} over coffee in {}. We talked about {} and life.", person.0, location, activity),
                    format!("{} called from {}. {} is doing well — planning to visit next month.", person.0, location, person.0),
                    format!("Had dinner with {} at a new place in {}. Great evening.", person.0, location),
                    format!("Video call with {} — showed them the new {} progress.", person.0, activity),
                    format!("{} texted about going {} this weekend. Sounds fun.", person.0, activity),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "episodic".into(),
                    importance: 0.6,
                    valence: 0.5,
                    half_life: 60.0 * 86400.0,
                    certainty: 0.9,
                    domain: "social".into(),
                    metadata: serde_json::json!({"category": "social", "person": person.0, "relationship": person.1}),
                    emotional_state: Some("happy".into()),
                    entities: vec![
                        (person.0.to_string(), "interacted_with".into()),
                        (location.to_string(), "located_at".into()),
                    ],
                }
            }

            // Emotional reflections
            6 => {
                let (emotion, valence) = pick!(emotions);
                let topic = pick!(work_topics);
                let texts = [
                    format!("Feeling {emotion} today. The {topic} project is taking more energy than expected."),
                    format!("Woke up feeling {emotion}. Need to take a step back and recharge."),
                    format!("Really {emotion} about the progress on {topic}. This is coming together."),
                    format!("End of day reflection: mostly {emotion}. Good conversations, manageable workload."),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "episodic".into(),
                    importance: 0.6,
                    valence: *valence,
                    half_life: 14.0 * 86400.0,
                    certainty: 0.8,
                    domain: "emotional".into(),
                    metadata: serde_json::json!({"category": "emotion", "emotion": emotion}),
                    emotional_state: Some(emotion.to_string()),
                    entities: vec![],
                }
            }

            // Hobby activities
            7 | 8 => {
                let hobby = pick!(hobbies);
                let day = pick!(days);
                let location = pick!(locations);
                let texts = [
                    format!("Went {hobby} on {day} near {location}. Beautiful weather."),
                    format!("Spent the evening {hobby}. Really relaxing after a long week."),
                    format!("Tried a new {hobby} spot near {location}. Will definitely go back."),
                    format!("{day} {hobby} session was great. Improving steadily."),
                    format!("Skipped {hobby} today — too tired. Will make it up this weekend."),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "episodic".into(),
                    importance: 0.4,
                    valence: 0.4,
                    half_life: 21.0 * 86400.0,
                    certainty: 0.9,
                    domain: "leisure".into(),
                    metadata: serde_json::json!({"category": "hobby", "hobby": hobby}),
                    emotional_state: Some("relaxed".into()),
                    entities: vec![(hobby.to_string(), "enjoys".into())],
                }
            }

            // Food and dining
            9 => {
                let food = pick!(foods);
                let person = pick!(people);
                let texts = [
                    format!("Had amazing {food} with {}.", person.0),
                    format!("Tried {food} for the first time. Absolutely delicious."),
                    format!("Quick lunch — {food}. Solid choice as always."),
                    format!("Cooking experiment tonight inspired by {food}. Turned out well!"),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "episodic".into(),
                    importance: 0.3,
                    valence: 0.5,
                    half_life: 14.0 * 86400.0,
                    certainty: 0.9,
                    domain: "food".into(),
                    metadata: serde_json::json!({"category": "food"}),
                    emotional_state: Some("happy".into()),
                    entities: vec![],
                }
            }

            // Health and wellness
            10 => {
                let texts = [
                    "Morning run: 5K in 28 minutes. Felt strong.",
                    "Yoga class with Nadia was exactly what I needed.",
                    "Doctor's appointment went well. All numbers looking good.",
                    "Slept poorly last night. Need to cut caffeine after 2pm.",
                    "New personal best on the bench press today!",
                    "Meditation session: 20 minutes. Mind was unusually calm.",
                    "Feeling a cold coming on. Loading up on vitamin C.",
                    "Physical therapy for the knee is helping. Running feels better.",
                    "Started tracking sleep with the new app. Averaging 6.5 hours.",
                    "Half marathon training: did 15K today. On track for the race.",
                ];
                let text = pick!(texts);
                let valence = if text.contains("strong") || text.contains("best") || text.contains("well") {
                    0.6
                } else if text.contains("poorly") || text.contains("cold") {
                    -0.3
                } else {
                    0.2
                };
                MemoryTemplate {
                    text: text.to_string(),
                    memory_type: "episodic".into(),
                    importance: 0.5,
                    valence,
                    half_life: 30.0 * 86400.0,
                    certainty: 0.9,
                    domain: "health".into(),
                    metadata: serde_json::json!({"category": "health"}),
                    emotional_state: None,
                    entities: vec![],
                }
            }

            // Travel memories
            11 => {
                let location = pick!(locations);
                let month = pick!(months);
                let texts = [
                    format!("Trip to {location} in {month} was incredible. The culture, the food, everything."),
                    format!("Planning a trip to {location} for {month}. Looking at flights."),
                    format!("Back from {location}. Already missing the energy of that city."),
                    format!("Found an amazing viewpoint in {location}. Took photos for an hour."),
                    format!("{location} in {month} — the cherry blossoms were peak."),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "episodic".into(),
                    importance: 0.6,
                    valence: 0.7,
                    half_life: 180.0 * 86400.0,
                    certainty: 0.9,
                    domain: "travel".into(),
                    metadata: serde_json::json!({"category": "travel", "location": location}),
                    emotional_state: Some("excited".into()),
                    entities: vec![(location.to_string(), "visited".into())],
                }
            }

            // Family
            12 => {
                let family = [("Maya", "partner"), ("Priya", "sister"), ("Amit", "father"), ("Sunita", "mother")];
                let (name, rel) = pick!(family);
                let texts = [
                    format!("{name} and I had a long talk about the future. Feeling aligned."),
                    format!("Called {name} ({rel}). Always good to hear their voice."),
                    format!("{name} sent photos from the garden. The roses are blooming."),
                    format!("Planning a surprise for {name}'s birthday next month."),
                    format!("Family video call — {name} shared exciting news about their work."),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "episodic".into(),
                    importance: 0.7,
                    valence: 0.6,
                    half_life: 90.0 * 86400.0,
                    certainty: 0.95,
                    domain: "family".into(),
                    metadata: serde_json::json!({"category": "family", "person": name, "relationship": rel}),
                    emotional_state: Some("warm".into()),
                    entities: vec![(name.to_string(), "family".into())],
                }
            }

            // Learning and growth
            13 | 14 => {
                let topic = pick!(work_topics);
                let texts = [
                    format!("Read a great paper on {topic}. Key insight: the combination of approaches matters more than any single technique."),
                    format!("Finished the online course on {topic}. Ready to apply this."),
                    format!("TIL: {topic} can be optimized by batching operations. 3x speedup!"),
                    format!("Book club: discussed 'Designing Data-Intensive Applications'. Relevant to {topic}."),
                    format!("Conference talk on {topic} was excellent. New ideas for our approach."),
                    format!("Deep dive into {topic} — wrote up notes for the team wiki."),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "semantic".into(),
                    importance: 0.6,
                    valence: 0.4,
                    half_life: 60.0 * 86400.0,
                    certainty: 0.8,
                    domain: "learning".into(),
                    metadata: serde_json::json!({"category": "learning", "topic": topic}),
                    emotional_state: Some("curious".into()),
                    entities: vec![(topic.to_string(), "studied".into())],
                }
            }

            // Financial
            15 => {
                let texts = [
                    "Reviewed the monthly budget. Spending on dining out is up 20%.",
                    "Stock portfolio up 3% this month. The tech rally continues.",
                    "Paid off the credit card balance. Feels great to be at zero.",
                    "Need to start saving more for the house down payment.",
                    "Subscription audit: cancelled three services I wasn't using.",
                    "Bonus came through! Putting 80% into savings.",
                ];
                MemoryTemplate {
                    text: pick!(texts).to_string(),
                    memory_type: "episodic".into(),
                    importance: 0.5,
                    valence: 0.2,
                    half_life: 30.0 * 86400.0,
                    certainty: 0.85,
                    domain: "finance".into(),
                    metadata: serde_json::json!({"category": "finance"}),
                    emotional_state: None,
                    entities: vec![],
                }
            }

            // Goals and plans
            16 => {
                let month = pick!(months);
                let texts = [
                    format!("Goal for {month}: ship the v1 of the cognitive memory engine."),
                    "Long-term goal: build the best personal AI companion in the world.".to_string(),
                    format!("Quarterly review: exceeded 2 of 3 OKRs. Need to push harder on {month} targets."),
                    "Career goal: transition from IC to tech lead by end of year.".to_string(),
                    "Personal goal: read 24 books this year. Currently at 8.".to_string(),
                    format!("Planning to run the half marathon in {month}. Training plan is on track."),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "semantic".into(),
                    importance: 0.7,
                    valence: 0.4,
                    half_life: 90.0 * 86400.0,
                    certainty: 0.7,
                    domain: "goals".into(),
                    metadata: serde_json::json!({"category": "goals"}),
                    emotional_state: Some("motivated".into()),
                    entities: vec![],
                }
            }

            // Daily routine
            17 | 18 => {
                let day = pick!(days);
                let time = pick!(times);
                let texts = [
                    format!("{day} {time}: usual routine — coffee, code review, standup, deep work."),
                    format!("Productive {day}. Got through the entire backlog of PRs."),
                    format!("Slow {day} {time}. Couldn't focus. Took a walk to reset."),
                    format!("{day}: worked from the café today. Change of scenery helped."),
                    format!("Late {time} on {day} — stayed up finishing the feature. Worth it."),
                    format!("{day} standup: blocked on the API dependency. Escalated to David."),
                ];
                MemoryTemplate {
                    text: pick!(texts).clone(),
                    memory_type: "episodic".into(),
                    importance: 0.3,
                    valence: 0.1,
                    half_life: 7.0 * 86400.0,
                    certainty: 0.9,
                    domain: "routine".into(),
                    metadata: serde_json::json!({"category": "routine", "day": day}),
                    emotional_state: None,
                    entities: vec![],
                }
            }

            // Contradictory / evolving memories (for conflict detection testing)
            19 => {
                let contradictions = [
                    ("I prefer Python over Rust for most projects.", "My favorite programming language is Rust."),
                    ("The team meeting is on Tuesdays at 10am.", "We moved the team meeting to Thursdays at 2pm."),
                    ("I don't enjoy running. It's too boring.", "I'm training for a half marathon this spring."),
                    ("I'm thinking of leaving the startup.", "I love working at the startup. Best job ever."),
                    ("Sarah is leaving the team next month.", "Sarah just got promoted to director."),
                ];
                let (text, _contradiction) = pick!(contradictions);
                MemoryTemplate {
                    text: text.to_string(),
                    memory_type: "semantic".into(),
                    importance: 0.6,
                    valence: 0.0,
                    half_life: 30.0 * 86400.0,
                    certainty: 0.6,
                    domain: "evolving".into(),
                    metadata: serde_json::json!({"category": "evolving", "may_contradict": true}),
                    emotional_state: None,
                    entities: vec![],
                }
            }

            _ => unreachable!(),
        };

        memories.push(mem);
    }

    memories
}

// Simple deterministic RNG (xorshift64)
struct SimpleRng(u64);

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn next_f64(&mut self) -> f64 {
        (self.next() % 1000) as f64 / 1000.0
    }
}
