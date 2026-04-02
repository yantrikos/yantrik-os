//! Cognitive Router — dynamic tool & recipe routing without an LLM.
//!
//! Routes user queries using a 4-layer pipeline:
//! 0.  Conversation detection (greetings, farewell, thanks)
//! 0.5 Keyword rules (math, time, units, disambiguation)
//! 0.7 Recipe matching (multi-step intents → built-in recipes)
//! 1.  Embedding similarity (MiniLM cosine, 22MB)
//!
//! Tools AND recipes register dynamically — no hardcoded lists. Works with
//! marketplace plugins, user-installed tools, and the built-in 86+ tools
//! plus 50+ recipe templates.
//!
//! Handles ~80% of OS interactions without touching Ollama/Claude.
//! Falls through to the real LLM for creative/complex tasks.

use anyhow::Result;
use std::sync::Mutex;
use std::time::Instant;

use crate::traits::{Embedder, LLMBackend};
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

// ── Keyword rule ─────────────────────────────────────────────────────

/// A keyword-based fast-path rule. Checked before embeddings.
struct KeywordRule {
    tool: &'static str,
    /// Query must contain ALL of these (lowercase).
    all_of: &'static [&'static str],
    /// Query must contain ANY of these (lowercase). Empty = skip check.
    any_of: &'static [&'static str],
    /// Regex pattern (optional). Checked if all_of/any_of pass.
    pattern: Option<&'static str>,
}

/// Built-in keyword rules for categories embeddings struggle with.
static KEYWORD_RULES: &[KeywordRule] = &[
    // ── Math expressions: digits + operator ──
    KeywordRule {
        tool: "calculate",
        all_of: &[],
        any_of: &[],
        pattern: Some(r"\d+\s*[+\-*/^%]\s*\d+"),
    },
    KeywordRule {
        tool: "calculate",
        all_of: &[],
        any_of: &["calculate", "compute", "evaluate", "what is", "how much is"],
        pattern: Some(r"\d"),
    },
    // ── Unit conversion ──
    KeywordRule {
        tool: "unit_convert",
        all_of: &[],
        any_of: &["convert", "conversion"],
        pattern: None,
    },
    KeywordRule {
        tool: "unit_convert",
        all_of: &["to"],
        any_of: &[
            "miles", "km", "kilometers", "celsius", "fahrenheit", "pounds",
            "kg", "kilograms", "inches", "feet", "meters", "gallons", "liters",
            "ounces", "grams", "yards", "mph", "kph",
        ],
        pattern: None,
    },
    // ── Time / date ──
    KeywordRule {
        tool: "date_calc",
        all_of: &[],
        any_of: &[
            "what date", "current date",
            "today's date", "what day", "date today",
        ],
        pattern: None,
    },
    KeywordRule {
        tool: "timer",
        all_of: &[],
        any_of: &["set timer", "start timer", "countdown", "set alarm"],
        pattern: None,
    },
    // ── Clipboard disambiguation ──
    KeywordRule {
        tool: "read_clipboard",
        all_of: &[],
        any_of: &[
            "what's on my clipboard", "whats on my clipboard", "read clipboard",
            "show clipboard", "paste from clipboard", "clipboard contents",
        ],
        pattern: None,
    },
    KeywordRule {
        tool: "write_clipboard",
        all_of: &[],
        any_of: &["copy to clipboard", "write to clipboard", "set clipboard"],
        pattern: None,
    },
    // ── Archive disambiguation ──
    KeywordRule {
        tool: "archive_create",
        all_of: &[],
        any_of: &[
            "compress", "zip ", "zip it", "tar ", "tarball", "create archive",
            "make archive", "archive this", "archive the",
        ],
        pattern: None,
    },
    KeywordRule {
        tool: "archive_extract",
        all_of: &[],
        any_of: &["extract", "unzip", "untar", "decompress", "unpack"],
        pattern: None,
    },
    // ── Network ports ──
    KeywordRule {
        tool: "network_ports",
        all_of: &[],
        any_of: &["port open", "open port", "listening port", "port 80", "port 443", "port 8080", "port 3000", "port 22"],
        pattern: Some(r"port\s+\d+"),
    },
    // ── Git status vs log ──
    KeywordRule {
        tool: "git_status",
        all_of: &[],
        any_of: &["git status", "repo status", "what repos", "uncommitted", "working tree"],
        pattern: None,
    },
    // ── Screenshot ──
    KeywordRule {
        tool: "screenshot",
        all_of: &[],
        any_of: &["screenshot", "screen capture", "capture screen", "take a screenshot"],
        pattern: None,
    },
    // ── Battery ──
    KeywordRule {
        tool: "battery_forecast",
        all_of: &[],
        any_of: &["battery", "charge level", "power level"],
        pattern: None,
    },
    // ── File operations ──
    KeywordRule {
        tool: "read_file",
        all_of: &[],
        any_of: &["read file", "read the file", "open file", "show file", "cat "],
        pattern: None,
    },
    KeywordRule {
        tool: "glob",
        all_of: &[],
        any_of: &["find all", "find files", "find every", "list all files", "glob"],
        pattern: None,
    },
    KeywordRule {
        tool: "grep",
        all_of: &[],
        any_of: &[
            "search for", "search in", "search source", "grep", "find in files",
            "search code", "search files for",
        ],
        pattern: None,
    },
    // ── Web / download ──
    KeywordRule {
        tool: "web_search",
        all_of: &[],
        any_of: &["search the web", "web search", "google for", "search online", "look up online"],
        pattern: None,
    },
    KeywordRule {
        tool: "download_file",
        all_of: &[],
        any_of: &["download", "fetch file", "save from url"],
        pattern: None,
    },
    // ── Service vs Docker ──
    KeywordRule {
        tool: "service_control",
        all_of: &[],
        any_of: &["service", "systemctl", "restart service", "stop service", "start service"],
        pattern: None,
    },
    // ── Firewall ──
    KeywordRule {
        tool: "firewall_list_rules",
        all_of: &[],
        any_of: &["firewall rule", "firewall rules", "list rules", "show rules"],
        pattern: None,
    },
    // ── Packages ──
    KeywordRule {
        tool: "package_list",
        all_of: &[],
        any_of: &["installed packages", "list packages", "show packages"],
        pattern: None,
    },
    KeywordRule {
        tool: "package_install",
        all_of: &[],
        any_of: &["install package", "install app", "apt install", "brew install"],
        pattern: None,
    },
    // ── Antivirus ──
    KeywordRule {
        tool: "antivirus_scan",
        all_of: &[],
        any_of: &["virus", "malware", "antivirus", "scan for virus", "infected"],
        pattern: None,
    },
    // ── Memory ──
    KeywordRule {
        tool: "remember",
        all_of: &[],
        any_of: &["remember that", "remember i", "remember my", "save note", "keep in mind"],
        pattern: None,
    },
    KeywordRule {
        tool: "recall",
        all_of: &[],
        any_of: &["recall", "do you remember", "what did i say", "what do you know about"],
        pattern: None,
    },
    // ── Notifications ──
    KeywordRule {
        tool: "send_notification",
        all_of: &[],
        any_of: &[
            "send a notification", "send notification", "notify me",
            "notification saying", "alert me", "send alert",
        ],
        pattern: None,
    },
    // ── System info (RAM, CPU, uptime) ──
    KeywordRule {
        tool: "system_info",
        all_of: &[],
        any_of: &[
            "how much ram", "ram usage", "memory usage", "cpu usage",
            "system info", "system status", "uptime", "how much memory",
        ],
        pattern: None,
    },
    // ── Current time (distinct from date_calc) ──
    KeywordRule {
        tool: "current_time",
        all_of: &[],
        any_of: &[
            "what time", "current time", "time now", "time is it",
            "what's the time", "tell me the time",
        ],
        pattern: None,
    },
    // ── Docker ──
    KeywordRule {
        tool: "docker_ps",
        all_of: &[],
        any_of: &["docker containers", "list containers", "running containers", "docker ps"],
        pattern: None,
    },
    // ── Process listing ──
    KeywordRule {
        tool: "list_processes",
        all_of: &[],
        any_of: &[
            "running processes", "show processes", "list processes",
            "eating cpu", "using cpu", "top processes",
        ],
        pattern: None,
    },
];

/// Check keyword rules against a lowercased query. Returns tool name on match.
fn check_keyword_rules(q: &str) -> Option<&'static str> {
    for rule in KEYWORD_RULES {
        // Check all_of: every term must be present
        if !rule.all_of.is_empty() && !rule.all_of.iter().all(|kw| q.contains(kw)) {
            continue;
        }
        // Check any_of: at least one term must be present (skip if empty)
        if !rule.any_of.is_empty() && !rule.any_of.iter().any(|kw| q.contains(kw)) {
            continue;
        }
        // Check regex pattern if provided
        if let Some(pat) = rule.pattern {
            // Simple regex check — we use a basic approach to avoid pulling in regex crate
            if !simple_regex_match(pat, q) {
                continue;
            }
        }
        // If all_of and any_of are both empty, pattern must have matched
        if rule.all_of.is_empty() && rule.any_of.is_empty() && rule.pattern.is_none() {
            continue;
        }
        return Some(rule.tool);
    }
    None
}

/// Minimal regex matching for the patterns we use (digit check, digit+operator+digit).
fn simple_regex_match(pattern: &str, text: &str) -> bool {
    match pattern {
        r"\d" => text.chars().any(|c| c.is_ascii_digit()),
        r"\d+\s*[+\-*/^%]\s*\d+" => {
            // Match: one or more digits, optional space, operator, optional space, digits
            let chars: Vec<char> = text.chars().collect();
            let len = chars.len();
            let mut i = 0;
            while i < len {
                // Find first digit sequence
                if chars[i].is_ascii_digit() {
                    // Skip digits
                    while i < len && chars[i].is_ascii_digit() { i += 1; }
                    // Skip optional spaces
                    while i < len && chars[i] == ' ' { i += 1; }
                    // Check for operator
                    if i < len && "+-*/^%".contains(chars[i]) {
                        i += 1;
                        // Skip optional spaces
                        while i < len && chars[i] == ' ' { i += 1; }
                        // Check for digit
                        if i < len && chars[i].is_ascii_digit() {
                            return true;
                        }
                    }
                } else {
                    i += 1;
                }
            }
            false
        }
        r"port\s+\d+" => {
            if let Some(pos) = text.find("port") {
                let after = &text[pos + 4..];
                let trimmed = after.trim_start();
                trimmed.chars().next().map_or(false, |c| c.is_ascii_digit())
            } else {
                false
            }
        }
        _ => false,
    }
}

// ── Plan shape ────────────────────────────────────────────────────────

/// Detected execution shape of a user query.
/// Used to boost recipe candidates over single tools for composite intents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanShape {
    /// Single tool call: "turn on wifi", "check battery"
    Atomic,
    /// Multi-step / conditional / scheduled: "set up a project and init git"
    Composite,
    /// Pure conversation, no tool needed
    Conversational,
}

/// Multi-step intent markers — if ANY match, query is likely Composite.
static COMPOSITE_MARKERS: &[&str] = &[
    // Sequencing
    "and then", "and also", "after that", "followed by",
    // Setup / workflow verbs
    "set up", "prepare", "organize", "clean up", "set me up",
    "bootstrap", "initialize", "configure",
    // Conditional
    "if ", "when ", "whenever", "unless",
    // Scheduled / recurring
    "every morning", "every day", "every week", "daily",
    "on startup", "at ", "before bed", "end of day",
    // Broad multi-action
    "help me with", "walk me through", "get ready for",
    "make sure", "take care of",
];

/// Detect the plan shape of a query. Cheap lexical check only.
fn detect_plan_shape(q: &str) -> PlanShape {
    // Check for composite markers
    if COMPOSITE_MARKERS.iter().any(|m| q.contains(m)) {
        return PlanShape::Composite;
    }
    // Multiple verbs (crude: count common action words)
    let action_words = [
        "open", "start", "run", "create", "delete", "move", "copy",
        "send", "check", "show", "find", "search", "install", "update",
        "stop", "restart", "close", "save", "download", "upload",
        "connect", "disconnect", "enable", "disable", "switch",
    ];
    let verb_count = action_words.iter().filter(|v| q.contains(**v)).count();
    if verb_count >= 2 {
        return PlanShape::Composite;
    }
    PlanShape::Atomic
}

// ── Routing result ────────────────────────────────────────────────────

/// What the router decided to do with a query.
#[derive(Debug, Clone)]
pub enum RouteDecision {
    /// Routed to a specific tool — return the tool name + confidence score.
    Tool { name: String, score: f32, response: String },
    /// Routed to a recipe template — multi-step workflow.
    Recipe { id: String, name: String, score: f32 },
    /// Conversation (greeting, farewell, thanks) — handled directly.
    Conversation { response: String },
    /// Needs a real LLM — creative, translate, summarize, complex reasoning.
    NeedsLLM,
}

// ── Dynamic tool entry ────────────────────────────────────────────────

/// A tool registered with the router.
struct ToolEntry {
    name: String,
    description: String,
    category: String,
    embedding: Vec<f32>,
}

/// A recipe template registered with the router.
struct RecipeEntry {
    id: String,
    name: String,
    description: String,
    category: String,
    embedding: Vec<f32>,
}

// ── Main router ───────────────────────────────────────────────────────

pub struct CognitiveRouter {
    tools: Mutex<Vec<ToolEntry>>,
    recipes: Mutex<Vec<RecipeEntry>>,
    embedder: Box<dyn Embedder>,
    /// Minimum similarity score to consider a tool match (below = NeedsLLM).
    pub similarity_threshold: f32,
    /// Score boost applied to recipe candidates when PlanShape is Composite.
    pub recipe_composite_boost: f32,
}

impl CognitiveRouter {
    /// Create router with an embedder. Tools and recipes are registered dynamically.
    pub fn new(embedder: Box<dyn Embedder>) -> Self {
        Self {
            tools: Mutex::new(Vec::new()),
            recipes: Mutex::new(Vec::new()),
            embedder,
            similarity_threshold: 0.35,
            recipe_composite_boost: 1.5,
        }
    }

    /// Register a tool with the router. Computes embedding from name + description.
    ///
    /// Call this for every tool in the companion registry, plus any marketplace
    /// or plugin tools as they're loaded.
    pub fn register_tool(&self, name: &str, description: &str, category: &str) {
        let embed_text = format!(
            "{} {} {}",
            name.replace('_', " "),
            description,
            category
        );

        let embedding = match self.embedder.embed(&embed_text) {
            Ok(emb) => emb,
            Err(e) => {
                tracing::warn!(tool = name, err = %e, "Failed to embed tool, skipping");
                return;
            }
        };

        let entry = ToolEntry {
            name: name.to_string(),
            description: description.to_string(),
            category: category.to_string(),
            embedding,
        };

        if let Ok(mut tools) = self.tools.lock() {
            // Update if already registered (e.g. plugin reload)
            if let Some(existing) = tools.iter_mut().find(|t| t.name == name) {
                existing.description = entry.description;
                existing.category = entry.category;
                existing.embedding = entry.embedding;
            } else {
                tools.push(entry);
            }
        }
    }

    /// Register multiple tools at once (batch embedding for efficiency).
    pub fn register_tools(&self, tools: &[(&str, &str, &str)]) {
        let texts: Vec<String> = tools
            .iter()
            .map(|(name, desc, cat)| format!("{} {} {}", name.replace('_', " "), desc, cat))
            .collect();

        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = match self.embedder.embed_batch(&text_refs) {
            Ok(embs) => embs,
            Err(e) => {
                tracing::warn!(err = %e, "Batch embed failed, falling back to individual");
                for &(name, desc, cat) in tools {
                    self.register_tool(name, desc, cat);
                }
                return;
            }
        };

        if let Ok(mut tool_list) = self.tools.lock() {
            for ((name, desc, cat), embedding) in tools.iter().zip(embeddings) {
                if let Some(existing) = tool_list.iter_mut().find(|t| t.name == *name) {
                    existing.description = desc.to_string();
                    existing.category = cat.to_string();
                    existing.embedding = embedding;
                } else {
                    tool_list.push(ToolEntry {
                        name: name.to_string(),
                        description: desc.to_string(),
                        category: cat.to_string(),
                        embedding,
                    });
                }
            }
            tracing::info!(count = tool_list.len(), "Registered tools with router");
        }
    }

    /// Unregister a tool (e.g. when a plugin is unloaded).
    pub fn unregister_tool(&self, name: &str) {
        if let Ok(mut tools) = self.tools.lock() {
            tools.retain(|t| t.name != name);
        }
    }

    /// How many tools are registered.
    pub fn tool_count(&self) -> usize {
        self.tools.lock().map(|t| t.len()).unwrap_or(0)
    }

    // ── Recipe registration ──────────────────────────────────────────

    /// Register a recipe template. Embeds from name + description + keywords.
    pub fn register_recipe(
        &self,
        id: &str,
        name: &str,
        description: &str,
        category: &str,
        keywords: &[&str],
    ) {
        let embed_text = format!(
            "{} {} {} {}",
            name,
            description,
            category,
            keywords.join(" "),
        );

        let embedding = match self.embedder.embed(&embed_text) {
            Ok(emb) => emb,
            Err(e) => {
                tracing::warn!(recipe = id, err = %e, "Failed to embed recipe, skipping");
                return;
            }
        };

        if let Ok(mut recipes) = self.recipes.lock() {
            if let Some(existing) = recipes.iter_mut().find(|r| r.id == id) {
                existing.name = name.to_string();
                existing.description = description.to_string();
                existing.category = category.to_string();
                existing.embedding = embedding;
            } else {
                recipes.push(RecipeEntry {
                    id: id.to_string(),
                    name: name.to_string(),
                    description: description.to_string(),
                    category: category.to_string(),
                    embedding,
                });
            }
        }
    }

    /// Batch-register recipe templates.
    pub fn register_recipes(&self, recipes: &[(&str, &str, &str, &str, &[&str])]) {
        let texts: Vec<String> = recipes
            .iter()
            .map(|(_, name, desc, cat, kws)| {
                format!("{} {} {} {}", name, desc, cat, kws.join(" "))
            })
            .collect();

        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = match self.embedder.embed_batch(&text_refs) {
            Ok(embs) => embs,
            Err(e) => {
                tracing::warn!(err = %e, "Batch recipe embed failed, falling back to individual");
                for &(id, name, desc, cat, kws) in recipes {
                    self.register_recipe(id, name, desc, cat, kws);
                }
                return;
            }
        };

        if let Ok(mut recipe_list) = self.recipes.lock() {
            for ((id, name, desc, cat, _), embedding) in recipes.iter().zip(embeddings) {
                if let Some(existing) = recipe_list.iter_mut().find(|r| r.id == *id) {
                    existing.name = name.to_string();
                    existing.description = desc.to_string();
                    existing.category = cat.to_string();
                    existing.embedding = embedding;
                } else {
                    recipe_list.push(RecipeEntry {
                        id: id.to_string(),
                        name: name.to_string(),
                        description: desc.to_string(),
                        category: cat.to_string(),
                        embedding,
                    });
                }
            }
            tracing::info!(count = recipe_list.len(), "Registered recipes with router");
        }
    }

    /// How many recipes are registered.
    pub fn recipe_count(&self) -> usize {
        self.recipes.lock().map(|r| r.len()).unwrap_or(0)
    }

    /// Detect the execution shape of a query.
    pub fn plan_shape(query: &str) -> PlanShape {
        detect_plan_shape(&query.to_lowercase())
    }

    /// Route a user query to a tool or decide it needs an LLM.
    pub fn route(&self, query: &str) -> RouteDecision {
        let t0 = Instant::now();
        let q = query.to_lowercase();

        // ── Layer 0: Conversation detection ──
        if let Some(resp) = Self::check_conversation(&q) {
            tracing::debug!(ms = t0.elapsed().as_millis(), "Router: conversation");
            return RouteDecision::Conversation { response: resp };
        }

        // ── Layer 0.5: Keyword rules (math, time, units, disambiguation) ──
        if let Some(tool_name) = check_keyword_rules(&q) {
            tracing::debug!(
                tool = tool_name, ms = t0.elapsed().as_millis(),
                "Router: keyword match"
            );
            return RouteDecision::Tool {
                name: tool_name.to_string(),
                score: 1.0, // keyword match = maximum confidence
                response: String::new(),
            };
        }

        // ── Layer 0.7 + 1: Embedding similarity (tools AND recipes) ──
        let query_emb = match self.embedder.embed(query) {
            Ok(emb) => emb,
            Err(e) => {
                tracing::warn!(err = %e, "Failed to embed query");
                return RouteDecision::NeedsLLM;
            }
        };

        let shape = detect_plan_shape(&q);

        // Score best tool
        let best_tool = if let Ok(tools) = self.tools.lock() {
            let mut best: Option<(String, f32)> = None;
            for tool in tools.iter() {
                let sim = cosine_similarity(&query_emb, &tool.embedding);
                if best.is_none() || sim > best.as_ref().unwrap().1 {
                    best = Some((tool.name.clone(), sim));
                }
            }
            best
        } else {
            None
        };

        // Score best recipe (with composite boost)
        let best_recipe = if let Ok(recipes) = self.recipes.lock() {
            let mut best: Option<(String, String, f32)> = None;
            for recipe in recipes.iter() {
                let mut sim = cosine_similarity(&query_emb, &recipe.embedding);
                // Boost recipes when query looks like a multi-step intent
                if shape == PlanShape::Composite {
                    sim *= self.recipe_composite_boost;
                }
                if best.is_none() || sim > best.as_ref().unwrap().2 {
                    best = Some((recipe.id.clone(), recipe.name.clone(), sim));
                }
            }
            best
        } else {
            None
        };

        // Pick the best candidate across tools and recipes
        let tool_score = best_tool.as_ref().map(|(_, s)| *s).unwrap_or(0.0);
        let recipe_score = best_recipe.as_ref().map(|(_, _, s)| *s).unwrap_or(0.0);

        // Recipe wins if it scores higher AND above threshold
        if recipe_score > tool_score && recipe_score >= self.similarity_threshold {
            let (id, name, score) = best_recipe.unwrap();
            tracing::debug!(
                recipe_id = %id, recipe_name = %name, score,
                shape = ?shape, ms = t0.elapsed().as_millis(),
                "Router: recipe match"
            );
            return RouteDecision::Recipe { id, name, score };
        }

        // Tool match
        if let Some((name, score)) = best_tool {
            if score >= self.similarity_threshold {
                tracing::debug!(
                    tool = %name, score,
                    shape = ?shape, ms = t0.elapsed().as_millis(),
                    "Router: embedding match"
                );
                return RouteDecision::Tool {
                    name,
                    score,
                    response: String::new(),
                };
            }
        }

        // ── Layer 2: Below threshold → needs LLM ──
        tracing::debug!(
            shape = ?shape, ms = t0.elapsed().as_millis(),
            "Router: needs LLM"
        );
        RouteDecision::NeedsLLM
    }

    /// Route and return top-N candidates with scores (for debugging / UI display).
    pub fn route_top_n(&self, query: &str, n: usize) -> Vec<(String, f32)> {
        let query_emb = match self.embedder.embed(query) {
            Ok(emb) => emb,
            Err(_) => return Vec::new(),
        };

        let mut scores = Vec::new();
        if let Ok(tools) = self.tools.lock() {
            for tool in tools.iter() {
                let sim = cosine_similarity(&query_emb, &tool.embedding);
                scores.push((tool.name.clone(), sim));
            }
        }
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores.truncate(n);
        scores
    }

    /// Format a tool result into a natural, contextual response.
    ///
    /// Bond level (0-10) controls personality:
    /// - 0-2: Professional, concise
    /// - 3-5: Warm, slightly conversational
    /// - 6-10: Casual, anticipatory, suggests next steps
    pub fn format_response(tool_name: &str, result: &str) -> String {
        Self::format_response_with_bond(tool_name, result, 3)
    }

    /// Format with explicit bond level for personality modulation.
    pub fn format_response_with_bond(tool_name: &str, result: &str, bond: u8) -> String {
        if result.is_empty() {
            return match bond {
                0..=2 => format!("Done ({tool_name})."),
                3..=5 => "Done.".into(),
                _ => "All set.".into(),
            };
        }

        // Confirmation prefix varies by bond
        let confirm = match bond {
            0..=2 => "",
            3..=5 => "",
            _ => "",
        };

        let response = match tool_name {
            // ── Math & conversion ──
            "calculate" => format!("The answer is {result}."),
            "unit_convert" => format!("{result}."),
            "date_calc" => result.to_string(),

            // ── Git ──
            "git_status" => format_git_response("Here's the repo status", result, bond),
            "git_commit" => match bond {
                0..=2 => format!("Committed: {result}"),
                3..=5 => format!("Changes committed. {result}"),
                _ => format!("Committed. {result}"),
            },
            "git_clone" => format!("Repository cloned: {result}"),
            "git_log" => format_section("Recent commits", result, bond),
            "git_diff" => format_section("Changes", result, bond),

            // ── Files ──
            "read_file" => result.to_string(),
            "write_file" | "edit_file" => match bond {
                0..=2 => "File updated successfully.".into(),
                _ => "File updated.".into(),
            },
            "glob" | "list_files" => format_section("Found files", result, bond),
            "grep" => format_section("Search results", result, bond),
            "diff_files" => format_section("Differences", result, bond),
            "hash_file" => format!("Hash: {result}"),
            "dir_size" => format!("{result}"),

            // ── System ──
            "list_processes" => format_section("Running processes", result, bond),
            "kill_process" => match bond {
                0..=5 => "Process terminated.".into(),
                _ => "Done, process killed.".into(),
            },
            "system_info" => result.to_string(),
            "disk_usage" => result.to_string(),
            "battery_forecast" => result.to_string(),

            // ── Archive ──
            "archive_create" => match bond {
                0..=2 => format!("Archive created: {result}"),
                _ => format!("Compressed. {result}"),
            },
            "archive_extract" => format!("Extracted to: {result}"),

            // ── Packages ──
            "package_install" => format!("Installed: {result}"),
            "package_remove" => format!("Removed: {result}"),
            "package_list" => format_section("Installed packages", result, bond),
            "package_search" => format_section("Search results", result, bond),

            // ── Docker ──
            "docker_ps" => format_section("Containers", result, bond),
            "docker_start" => format!("Container started: {result}"),
            "docker_stop" => format!("Container stopped: {result}"),

            // ── Network ──
            "network_ping" => result.to_string(),
            "network_interfaces" => format_section("Network interfaces", result, bond),
            "network_ports" => format_section("Open ports", result, bond),
            "network_dns_set" => format!("DNS updated: {result}"),
            "wifi_scan" => format_section("Wi-Fi networks", result, bond),
            "wifi_connect" => match bond {
                0..=2 => format!("Connected to Wi-Fi: {result}"),
                3..=5 => format!("Connected to {result}."),
                _ => format!("You're on {result} now."),
            },
            "wifi_disconnect" => "Disconnected from Wi-Fi.".into(),

            // ── Bluetooth ──
            "bluetooth_connect" => match bond {
                0..=2 => format!("Bluetooth connected: {result}"),
                3..=5 => format!("Connected to {result}."),
                _ => format!("Paired and connected to {result}. Want me to route audio there?"),
            },
            "bluetooth_disconnect" => "Bluetooth disconnected.".into(),
            "bluetooth_pair" => format!("Paired with {result}."),
            "bluetooth_scan" => format_section("Bluetooth devices", result, bond),

            // ── Services ──
            "service_control" => match bond {
                0..=5 => format!("Service updated: {result}"),
                _ => format!("Done. {result}"),
            },

            // ── Security ──
            "antivirus_scan" => match bond {
                0..=2 => format!("Scan complete: {result}"),
                3..=5 => format!("Scan done. {result}"),
                _ => {
                    if result.contains("clean") || result.contains("no threat") {
                        "All clear, no threats found.".into()
                    } else {
                        format!("Scan done. {result}")
                    }
                },
            },
            "antivirus_update" => "Virus definitions updated.".into(),
            "firewall_list_rules" | "firewall_status" => format_section("Firewall", result, bond),

            // ── Vault ──
            "vault_store" => match bond {
                0..=5 => "Credential stored securely.".into(),
                _ => "Stored safely in the vault.".into(),
            },
            "vault_retrieve" => format!("{result}"),
            "vault_delete" => "Credential deleted.".into(),
            "vault_generate_password" => format!("Generated: {result}"),

            // ── Desktop / UI ──
            "set_wallpaper" => match bond {
                0..=5 => "Wallpaper updated.".into(),
                _ => "New wallpaper set. Looks good.".into(),
            },
            "set_resolution" => format!("Resolution changed to {result}."),
            "screenshot" => match bond {
                0..=5 => "Screenshot captured.".into(),
                _ => "Screenshot taken.".into(),
            },
            "focus_window" | "close_window" => "Done.".into(),

            // ── Browser ──
            "browse" | "launch_browser" | "open_url" => match bond {
                0..=5 => "Opened.".into(),
                _ => "Opened in the browser.".into(),
            },
            "web_search" => format_section("Search results", result, bond),
            "browser_cleanup" => "Browser tabs closed.".into(),

            // ── Audio / Media ──
            "audio_control" => match bond {
                0..=5 => "Volume updated.".into(),
                _ => "Adjusted.".into(),
            },

            // ── Weather ──
            "get_weather" => result.to_string(),

            // ── Communication ──
            "send_notification" => "Notification sent.".into(),
            "download_file" => format!("Downloaded: {result}"),

            // ── Encoding ──
            "base64_encode" | "base64_decode" => format!("{result}"),

            // ── Memory ──
            "remember" | "update_memory" => match bond {
                0..=2 => "Noted.".into(),
                3..=5 => "Got it, I'll remember that.".into(),
                _ => "Remembered.".into(),
            },
            "recall" => result.to_string(),
            "forget_memory" => "Memory deleted.".into(),

            // ── SSH ──
            "ssh_run" => format_section("Remote output", result, bond),

            // ── Misc ──
            "timer" => format!("Timer set: {result}"),
            "save_workspace" => match bond {
                0..=5 => "Workspace saved.".into(),
                _ => "Workspace snapshot saved. You can restore it anytime.".into(),
            },

            // Default: tools return their own formatted output
            _ => result.to_string(),
        };

        if confirm.is_empty() {
            response
        } else {
            format!("{confirm} {response}")
        }
    }

    /// Check if no LLM is configured and return a helpful Core Mode message.
    pub fn core_mode_response(query: &str) -> String {
        format!(
            "I can handle system commands, file operations, and automation in Core Mode. \
             For open-ended questions like \"{}\", enable Enhanced Reasoning by configuring an LLM provider in settings.",
            if query.len() > 60 { &query[..60] } else { query }
        )
    }

    // ── Private: conversation detection ───────────────────────────────

    fn check_conversation(q: &str) -> Option<String> {
        let greetings = ["hello", "hi ", "hi!", "hey", "good morning", "good afternoon", "good evening", "howdy", "sup"];
        let farewells = ["goodbye", "bye", "see you", "good night", "later", "gotta go"];
        let thanks = ["thanks", "thank you", "thx", "appreciate", "cheers"];

        if greetings.iter().any(|w| q.starts_with(w) || q == "hi") {
            // Time-aware greeting using system time
            let hour = {
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                // UTC seconds → local hour (approximate; offset applied in companion)
                ((secs / 3600) % 24) as u32
            };
            let time_greeting = match hour {
                5..=11 => "Good morning",
                12..=16 => "Good afternoon",
                17..=21 => "Good evening",
                _ => "Hey",
            };
            return Some(format!("{time_greeting}! How can I help?"));
        }
        if farewells.iter().any(|w| q.contains(w)) {
            return Some("See you later! I'll be here if you need anything.".into());
        }
        if thanks.iter().any(|w| q.contains(w)) {
            return Some("Anytime!".into());
        }
        if q.starts_with("how are you") {
            return Some("Running smooth — all systems green. What can I do for you?".into());
        }
        None
    }

    /// Get current time as a formatted string (used when current_time tool isn't registered).
    pub fn current_time_response() -> String {
        // Use std::process::Command to get formatted time (portable)
        let output = std::process::Command::new("date")
            .arg("+%I:%M %p, %A %B %e, %Y")
            .output();
        match output {
            Ok(o) if o.status.success() => {
                let time_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                format!("It's {time_str}.")
            }
            _ => {
                // Fallback: raw epoch-based
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let hours = (secs / 3600) % 24;
                let minutes = (secs / 60) % 60;
                format!("It's {:02}:{:02} UTC.", hours, minutes)
            }
        }
    }
}

// ── LLMBackend implementation ──────────────────────────────────────────

impl LLMBackend for CognitiveRouter {
    fn chat(
        &self,
        messages: &[ChatMessage],
        _config: &GenerationConfig,
        _tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        let query = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let decision = self.route(query);
        let text = match &decision {
            RouteDecision::Conversation { response } => response.clone(),
            RouteDecision::Tool { name, response, .. } => {
                if response.is_empty() {
                    format!("[ROUTE:{}]", name)
                } else {
                    response.clone()
                }
            }
            RouteDecision::Recipe { id, name, .. } => {
                format!("[RECIPE:{}:{}]", id, name)
            }
            RouteDecision::NeedsLLM => "[NEEDS_LLM]".into(),
        };

        Ok(LLMResponse {
            text,
            prompt_tokens: 0,
            completion_tokens: 0,
            tool_calls: Vec::new(),
            api_tool_calls: Vec::new(),
            stop_reason: match &decision {
                RouteDecision::NeedsLLM => "needs_llm".into(),
                _ => "stop".into(),
            },
        })
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        let resp = self.chat(messages, config, tools)?;
        on_token(&resp.text);
        Ok(resp)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        "cognitive-router"
    }

    fn model_id(&self) -> &str {
        "yantrik-cognitive-router-v1"
    }
}

// ── Response formatting helpers ────────────────────────────────────────

/// Format a section with a header. Bond controls verbosity.
fn format_section(header: &str, content: &str, bond: u8) -> String {
    match bond {
        0..=2 => format!("{header}:\n{content}"),
        _ => {
            // Count lines to decide if we need a header
            let line_count = content.lines().count();
            if line_count <= 2 {
                content.to_string()
            } else {
                format!("{header}:\n{content}")
            }
        }
    }
}

/// Format git-specific responses with context awareness.
fn format_git_response(header: &str, content: &str, bond: u8) -> String {
    let line_count = content.lines().count();
    if line_count == 0 {
        match bond {
            0..=5 => "Working tree is clean.".into(),
            _ => "All clean, nothing to commit.".into(),
        }
    } else {
        format_section(header, content, bond)
    }
}

// ── Utility ────────────────────────────────────────────────────────────

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
}
