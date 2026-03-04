//! Life Assistant — Task Template System + Intent Parser + Multi-Source Orchestration.
//!
//! Provides a structured way to define real-world research tasks (find restaurants,
//! people, products, etc.) with typed data sources, extraction schemas, and ranking
//! configuration. The companion agent uses these templates to plan multi-source
//! web research and present normalized, ranked results.
//!
//! # Architecture
//!
//! - [`TaskTemplate`] — defines *what* to search, *where* to look, and *how* to rank
//! - [`TaskTemplateRegistry`] — holds all templates and matches user queries to them
//! - [`templates`] — built-in templates for common life tasks
//! - [`intent`] — keyword-based intent detection and parameter extraction
//! - [`orchestrator`] — multi-source CDP browser search, extraction, deduplication
//!
//! Templates are data-only (no execution logic). The agent loop reads a matched
//! template and orchestrates browser/fetch tools accordingly.

pub mod templates;
pub mod intent;
pub mod orchestrator;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::tools::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use intent::{
    detect_intent, generate_clarifying_questions, format_clarifying_response, format_ready_intent,
};

// ── Core Types ──────────────────────────────────────────────────────────────

/// A task template defines the structure for a class of real-world research tasks.
///
/// Each template describes data sources to query, fields to extract from results,
/// which inputs are required from the user, and how to rank the final output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTemplate {
    /// Unique task type identifier (e.g., "find_restaurant", "find_person").
    pub task_type: String,
    /// Human-readable name shown to the user.
    pub name: String,
    /// Description for LLM context — explains the task's purpose.
    pub description: String,
    /// Data sources to search, ordered by priority.
    pub sources: Vec<DataSource>,
    /// Fields to extract from each source's results.
    pub extraction_schema: Vec<FieldSpec>,
    /// Fields that must be provided by the user (triggers clarifying questions if missing).
    pub required_fields: Vec<String>,
    /// Optional fields the user might specify to narrow results.
    pub optional_fields: Vec<String>,
    /// How to rank and sort the final results.
    pub ranking: RankingConfig,
    /// Maximum number of results to return.
    pub max_results: usize,
}

/// A data source that can be queried for a particular task type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSource {
    /// Source name (e.g., "google_maps", "yelp", "linkedin").
    pub name: String,
    /// Base URL pattern with `{query}` placeholder for the search term.
    pub search_url: String,
    /// CSS selector or description for locating result containers on the page.
    pub result_selector: Option<String>,
    /// Priority for failover ordering (1 = first, higher = later).
    pub priority: u8,
    /// Whether this source is enabled by default.
    pub enabled: bool,
}

/// Specification for a single field to extract from search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    /// Field name (e.g., "name", "rating", "price").
    pub name: String,
    /// Field type — used for normalization and validation.
    pub field_type: FieldType,
    /// Whether this field is required in each result entry.
    pub required: bool,
    /// Human-readable description for LLM extraction guidance.
    pub description: String,
}

/// Typed field categories for normalization and validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Free-form text.
    Text,
    /// Numeric value (integer or float).
    Number,
    /// Rating normalized to 0.0–5.0 scale.
    Rating,
    /// Currency-aware price value.
    Price,
    /// URL / hyperlink.
    Url,
    /// Phone number.
    Phone,
    /// Physical address.
    Address,
    /// Date and/or time value.
    DateTime,
    /// True/false flag.
    Boolean,
}

/// Configuration for ranking and sorting search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingConfig {
    /// Weighted factors used to compute a composite ranking score.
    pub factors: Vec<RankingFactor>,
    /// Default sort order (e.g., "rating_desc", "price_asc", "relevance").
    pub default_sort: String,
}

/// A single factor in the ranking formula.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingFactor {
    /// Name of the field to use for ranking.
    pub field: String,
    /// Weight of this factor in the composite score (0.0–1.0).
    pub weight: f64,
    /// Sort direction for this factor.
    pub order: SortOrder,
}

/// Sort direction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Ascending,
    Descending,
}

// ── Template Registry ───────────────────────────────────────────────────────

/// Registry holding all task templates with lookup by type and query matching.
///
/// Initialized with built-in templates and supports runtime additions.
pub struct TaskTemplateRegistry {
    templates: HashMap<String, TaskTemplate>,
}

impl TaskTemplateRegistry {
    /// Create a new registry pre-loaded with all built-in templates.
    pub fn new() -> Self {
        let mut registry = Self {
            templates: HashMap::new(),
        };
        for template in templates::builtin_templates() {
            registry
                .templates
                .insert(template.task_type.clone(), template);
        }
        registry
    }

    /// Look up a template by its exact task type identifier.
    pub fn get(&self, task_type: &str) -> Option<&TaskTemplate> {
        self.templates.get(task_type)
    }

    /// Register a custom template (or overwrite a built-in one).
    pub fn register(&mut self, template: TaskTemplate) {
        self.templates
            .insert(template.task_type.clone(), template);
    }

    /// Find the template that best matches a natural-language user query.
    ///
    /// Uses keyword matching against a curated set of trigger words for each
    /// task type. Returns `None` if no keywords match.
    pub fn match_query(&self, query: &str) -> Option<&TaskTemplate> {
        let lower = query.to_lowercase();

        let keyword_map: &[(&str, &[&str])] = &[
            (
                "find_hotel",
                &[
                    "hotel", "motel", "stay", "accommodation", "airbnb",
                    "hostel", "lodge", "booking", "check in", "check out",
                    "room", "resort",
                ],
            ),
            (
                "find_job",
                &[
                    "job", "career", "hiring", "work at", "position",
                    "vacancy", "employment", "internship", "recruit", "openings",
                ],
            ),
            (
                "find_restaurant",
                &[
                    "restaurant", "food", "eat", "dinner", "lunch",
                    "breakfast", "brunch", "cafe", "cuisine", "takeout",
                    "delivery", "dine", "dining", "bistro", "eatery",
                ],
            ),
            (
                "find_person",
                &[
                    "person", "who is", "find someone", "linkedin",
                    "contact info", "people search", "look up someone", "profile",
                ],
            ),
            (
                "find_product",
                &[
                    "buy", "product", "price", "shop", "deal", "purchase",
                    "compare prices", "review", "shopping", "coupon",
                    "discount", "amazon",
                ],
            ),
            (
                "find_service",
                &[
                    "plumber", "doctor", "dentist", "service", "repair",
                    "contractor", "electrician", "mechanic", "lawyer",
                    "therapist", "tutor", "cleaner", "handyman",
                ],
            ),
        ];

        let mut best: Option<(&str, usize)> = None;

        for (task_type, keywords) in keyword_map {
            let score: usize = keywords
                .iter()
                .filter(|kw| lower.contains(**kw))
                .count();

            if score > 0 {
                if best.map_or(true, |(_, s)| score > s) {
                    best = Some((task_type, score));
                }
            }
        }

        best.and_then(|(task_type, _)| self.templates.get(task_type))
    }

    /// List all registered task type identifiers.
    pub fn list(&self) -> Vec<&str> {
        let mut types: Vec<&str> = self.templates.keys().map(|s| s.as_str()).collect();
        types.sort();
        types
    }

    /// Number of registered templates.
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }
}

impl Default for TaskTemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Life Search Tool ────────────────────────────────────────────────────────

/// Parses intent from a natural-language search request, extracts parameters,
/// and either asks clarifying questions or produces a structured intent.
pub struct LifeSearchTool;

impl Tool for LifeSearchTool {
    fn name(&self) -> &'static str {
        "life_search"
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Standard
    }

    fn category(&self) -> &'static str {
        "life_assistant"
    }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "life_search",
                "description": "Search for real-world things: restaurants, people, products, jobs, hotels, services. Parses intent, asks clarifying questions if needed, then returns structured search parameters. Use this when the user asks about finding places, people, products, or services in the real world.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The user's search request in natural language"
                        },
                        "task_type": {
                            "type": "string",
                            "description": "Optional: force a specific task type (find_restaurant, find_person, find_product, find_job, find_hotel, find_service)"
                        }
                    },
                    "required": ["query"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let forced_type = args.get("task_type").and_then(|v| v.as_str());

        if query.trim().is_empty() {
            return "Please provide a search query.".to_string();
        }

        let mut intent = detect_intent(query);

        // Allow the LLM to override the detected task type
        if let Some(ft) = forced_type {
            intent.task_type = Some(ft.to_string());
            intent.is_life_task = true;
            let required = match ft {
                "find_restaurant" => vec!["location"],
                "find_person" => vec!["name"],
                "find_product" => vec!["query"],
                "find_job" => vec!["query"],
                "find_hotel" => vec!["location"],
                "find_service" => vec!["service_type", "location"],
                _ => vec![],
            };
            intent.missing_params = required
                .into_iter()
                .filter(|f| !intent.params.contains_key(*f))
                .map(|f| f.to_string())
                .collect();
        }

        if !intent.is_life_task {
            return "This doesn't seem like a search task. I can help find restaurants, \
                    people, products, jobs, hotels, or services. Try something like \
                    'find Thai food near downtown' or 'who is John Smith'."
                .to_string();
        }

        // Check for missing required params — ask clarifying questions
        if !intent.missing_params.is_empty() {
            let questions =
                generate_clarifying_questions(&intent, &intent.missing_params.clone());
            return format_clarifying_response(&intent, &questions);
        }

        // All required params present — return the structured intent
        format_ready_intent(&intent)
    }
}

// ── Registration ────────────────────────────────────────────────────────────

/// Register all life assistant tools with the tool registry.
pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(LifeSearchTool));
    orchestrator::register(reg);
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_loads_builtins() {
        let reg = TaskTemplateRegistry::new();
        assert!(reg.len() >= 6, "expected at least 6 built-in templates");
        assert!(reg.get("find_restaurant").is_some());
        assert!(reg.get("find_person").is_some());
        assert!(reg.get("find_product").is_some());
        assert!(reg.get("find_job").is_some());
        assert!(reg.get("find_hotel").is_some());
        assert!(reg.get("find_service").is_some());
    }

    #[test]
    fn match_query_restaurant() {
        let reg = TaskTemplateRegistry::new();
        let t = reg.match_query("I want to eat Italian food tonight").unwrap();
        assert_eq!(t.task_type, "find_restaurant");
    }

    #[test]
    fn match_query_person() {
        let reg = TaskTemplateRegistry::new();
        let t = reg.match_query("who is John Smith on linkedin").unwrap();
        assert_eq!(t.task_type, "find_person");
    }

    #[test]
    fn match_query_product() {
        let reg = TaskTemplateRegistry::new();
        let t = reg.match_query("I want to buy a new laptop, compare prices").unwrap();
        assert_eq!(t.task_type, "find_product");
    }

    #[test]
    fn match_query_job() {
        let reg = TaskTemplateRegistry::new();
        let t = reg.match_query("find me a software engineering job").unwrap();
        assert_eq!(t.task_type, "find_job");
    }

    #[test]
    fn match_query_hotel() {
        let reg = TaskTemplateRegistry::new();
        let t = reg.match_query("book a hotel in Paris for my stay").unwrap();
        assert_eq!(t.task_type, "find_hotel");
    }

    #[test]
    fn match_query_service() {
        let reg = TaskTemplateRegistry::new();
        let t = reg.match_query("find a plumber near me").unwrap();
        assert_eq!(t.task_type, "find_service");
    }

    #[test]
    fn match_query_no_match() {
        let reg = TaskTemplateRegistry::new();
        assert!(reg.match_query("what is the weather today").is_none());
    }

    #[test]
    fn list_returns_sorted() {
        let reg = TaskTemplateRegistry::new();
        let list = reg.list();
        let mut sorted = list.clone();
        sorted.sort();
        assert_eq!(list, sorted);
    }

    #[test]
    fn templates_have_valid_structure() {
        let reg = TaskTemplateRegistry::new();
        for task_type in reg.list() {
            let t = reg.get(task_type).unwrap();
            assert!(!t.name.is_empty(), "{} has empty name", task_type);
            assert!(!t.description.is_empty(), "{} has empty description", task_type);
            assert!(!t.sources.is_empty(), "{} has no sources", task_type);
            assert!(!t.extraction_schema.is_empty(), "{} has no extraction schema", task_type);
            assert!(!t.required_fields.is_empty(), "{} has no required fields", task_type);
            assert!(t.max_results > 0, "{} has max_results=0", task_type);
            assert!(
                !t.ranking.factors.is_empty(),
                "{} has no ranking factors",
                task_type
            );
        }
    }

    #[test]
    fn serde_roundtrip() {
        let reg = TaskTemplateRegistry::new();
        let template = reg.get("find_restaurant").unwrap();
        let json = serde_json::to_string(template).expect("serialize");
        let deserialized: TaskTemplate =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.task_type, template.task_type);
        assert_eq!(deserialized.sources.len(), template.sources.len());
        assert_eq!(
            deserialized.extraction_schema.len(),
            template.extraction_schema.len()
        );
    }
}
