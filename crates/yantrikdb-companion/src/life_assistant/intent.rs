//! Intent parser — fast keyword-based detection for life assistant tasks.
//!
//! Detects task types (find_restaurant, find_person, find_product, find_job,
//! find_hotel, find_service) from natural language and extracts structured
//! parameters without any LLM call.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Data Structures ──

/// Parsed intent from user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedIntent {
    /// Detected task type (e.g., "find_restaurant", "find_person").
    pub task_type: Option<String>,
    /// Confidence score 0.0-1.0.
    pub confidence: f64,
    /// Extracted parameters.
    pub params: HashMap<String, String>,
    /// Parameters that are still missing (required but not provided).
    pub missing_params: Vec<String>,
    /// Whether this is a life assistant intent at all.
    pub is_life_task: bool,
}

/// Clarifying question to ask the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarifyingQuestion {
    /// The question text.
    pub question: String,
    /// Which parameters this question resolves.
    pub resolves: Vec<String>,
    /// Optional suggested answers.
    pub suggestions: Vec<String>,
}

// ── Keyword Lists ──

const RESTAURANT_KEYWORDS: &[&str] = &[
    "restaurant", "food", "eat", "dining", "dinner", "lunch", "breakfast",
    "brunch", "pizza", "burger", "sushi", "thai", "mexican", "italian",
    "chinese", "indian", "korean", "japanese", "ramen", "taco", "steak",
    "seafood", "vegan", "vegetarian", "cafe", "bistro", "diner",
    "where can i eat", "where to eat", "good place to eat", "hungry",
    "food place", "takeout", "delivery", "cuisine",
];

const PERSON_KEYWORDS: &[&str] = &[
    "who is", "find person", "search for person", "look up person",
    "linkedin", "search linkedin", "facebook profile", "twitter profile",
    "find someone", "people search", "person named", "contact info for",
    "who was", "biography", "background check",
];

const PRODUCT_KEYWORDS: &[&str] = &[
    "best laptop", "compare prices", "where to buy", "product review",
    "buy a", "purchase", "shopping", "price of", "how much does",
    "amazon", "deal on", "deals for", "cheap", "affordable",
    "recommendation for", "suggest a", "best phone", "best headphones",
    "gadget", "which model", "specs for", "compare",
];

const JOB_KEYWORDS: &[&str] = &[
    "jobs in", "job search", "hiring", "career", "work at", "position at",
    "remote job", "freelance", "salary for", "glassdoor", "indeed",
    "job listing", "job opening", "apply for", "resume", "interview at",
    "looking for work", "employment", "vacancy", "internship",
];

const HOTEL_KEYWORDS: &[&str] = &[
    "hotel", "where to stay", "accommodation", "airbnb", "motel",
    "hostel", "booking", "lodge", "resort", "bed and breakfast",
    "room near", "stay in", "stay near", "lodging", "vacation rental",
];

const SERVICE_KEYWORDS: &[&str] = &[
    "plumber", "electrician", "mechanic", "dentist", "doctor", "lawyer",
    "attorney", "accountant", "therapist", "contractor", "handyman",
    "cleaner", "mover", "tutor", "vet", "veterinarian", "barber",
    "hairdresser", "salon", "gym near", "find a service", "near me",
    "recommend a", "good mechanic", "good doctor",
];

// ── Cuisine Detection ──

const CUISINES: &[&str] = &[
    "italian", "thai", "mexican", "chinese", "indian", "japanese",
    "korean", "french", "greek", "mediterranean", "vietnamese",
    "american", "brazilian", "ethiopian", "turkish", "lebanese",
    "peruvian", "spanish", "german", "british", "caribbean",
    "hawaiian", "cajun", "southern", "bbq", "barbecue",
];

const FOOD_ITEMS: &[&str] = &[
    "pizza", "burger", "sushi", "ramen", "taco", "steak", "seafood",
    "noodles", "pasta", "wings", "ribs", "pho", "curry", "dim sum",
    "dumplings", "kebab", "falafel", "shawarma", "sandwich", "salad",
    "soup", "brunch", "dessert", "ice cream", "coffee",
];

// ── Intent Detection ──

/// Fast keyword-based intent detection. No LLM call needed.
///
/// Returns a `ParsedIntent` with `is_life_task = false` if the text does not
/// match any life assistant pattern.
pub fn detect_intent(user_text: &str) -> ParsedIntent {
    let lower = user_text.to_lowercase();
    let mut params = HashMap::new();

    // Score each task type
    let scores = [
        ("find_restaurant", score_keywords(&lower, RESTAURANT_KEYWORDS)),
        ("find_person", score_keywords(&lower, PERSON_KEYWORDS)),
        ("find_product", score_keywords(&lower, PRODUCT_KEYWORDS)),
        ("find_job", score_keywords(&lower, JOB_KEYWORDS)),
        ("find_hotel", score_keywords(&lower, HOTEL_KEYWORDS)),
        ("find_service", score_keywords(&lower, SERVICE_KEYWORDS)),
    ];

    // Pick the highest-scoring task type
    let (best_type, best_score) = scores
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(t, s)| (t.to_string(), *s))
        .unwrap_or_default();

    // Threshold: need at least some signal
    if best_score < 0.2 {
        return ParsedIntent {
            task_type: None,
            confidence: best_score,
            params,
            missing_params: Vec::new(),
            is_life_task: false,
        };
    }

    // Extract parameters based on task type
    extract_location(&lower, &mut params);
    extract_budget(&lower, &mut params);
    extract_date(&lower, &mut params);
    extract_party_size(&lower, &mut params);

    match best_type.as_str() {
        "find_restaurant" => {
            extract_cuisine(&lower, &mut params);
        }
        "find_person" => {
            extract_person_name(user_text, &lower, &mut params);
        }
        "find_product" => {
            extract_product_query(&lower, &mut params);
        }
        "find_job" => {
            extract_job_query(&lower, &mut params);
        }
        "find_hotel" => {
            // location + date + budget already extracted above
        }
        "find_service" => {
            extract_service_type(&lower, &mut params);
        }
        _ => {}
    }

    // Determine required fields and which are missing
    let required = required_fields(&best_type);
    let missing: Vec<String> = required
        .iter()
        .filter(|f| !params.contains_key(f.as_str()))
        .cloned()
        .collect();

    // Confidence: higher if more params extracted, lower if many missing
    let filled_ratio = if required.is_empty() {
        1.0
    } else {
        (required.len() - missing.len()) as f64 / required.len() as f64
    };
    let confidence = (best_score * 0.6 + filled_ratio * 0.4).min(1.0);

    ParsedIntent {
        task_type: Some(best_type),
        confidence,
        params,
        missing_params: missing,
        is_life_task: true,
    }
}

/// Returns the list of required parameter names for a given task type.
fn required_fields(task_type: &str) -> Vec<String> {
    match task_type {
        "find_restaurant" => vec!["location".into()],
        "find_person" => vec!["name".into()],
        "find_product" => vec!["query".into()],
        "find_job" => vec!["query".into()],
        "find_hotel" => vec!["location".into()],
        "find_service" => vec!["service_type".into(), "location".into()],
        _ => Vec::new(),
    }
}

// ── Keyword Scoring ──

/// Score how well the text matches a keyword list.
/// Returns 0.0-1.0 based on number and quality of matches.
fn score_keywords(text: &str, keywords: &[&str]) -> f64 {
    let mut hits = 0;
    let mut phrase_hits = 0;

    for kw in keywords {
        if kw.contains(' ') {
            // Multi-word phrase — worth more
            if text.contains(kw) {
                phrase_hits += 1;
            }
        } else if text.contains(kw) {
            hits += 1;
        }
    }

    // Phrases are worth double
    let weighted = hits as f64 + phrase_hits as f64 * 2.0;
    let max_possible = keywords.len() as f64;

    // Sigmoid-ish: even 2-3 hits should give a decent score
    let raw = weighted / max_possible.max(1.0);
    // Boost: 1 hit = ~0.3, 2 hits = ~0.5, 3+ hits = ~0.7+
    (raw * 3.0).min(1.0)
}

// ── Parameter Extraction ──

/// Extract location from patterns like "near X", "in X", "around X", city names.
fn extract_location(text: &str, params: &mut HashMap<String, String>) {
    // Pattern: "near X", "in X", "around X"
    let location_patterns = ["near ", "in ", "around ", "close to ", "nearby "];

    for pattern in &location_patterns {
        if let Some(idx) = text.find(pattern) {
            let after = &text[idx + pattern.len()..];
            let loc = extract_noun_phrase(after);
            if !loc.is_empty() && !is_stop_word(&loc) {
                params.insert("location".into(), loc);
                return;
            }
        }
    }

    // Pattern: "downtown", "midtown", "uptown"
    for area in &["downtown", "midtown", "uptown", "city center"] {
        if text.contains(area) {
            params.insert("location".into(), area.to_string());
            return;
        }
    }

    // Pattern: well-known cities (quick check)
    let cities = [
        "new york", "los angeles", "chicago", "houston", "phoenix",
        "san francisco", "seattle", "denver", "austin", "boston",
        "miami", "atlanta", "portland", "san diego", "dallas",
        "london", "paris", "tokyo", "berlin", "amsterdam",
        "mumbai", "bangalore", "delhi", "toronto", "vancouver",
        "sydney", "melbourne", "singapore", "hong kong", "dubai",
    ];
    for city in &cities {
        if text.contains(city) {
            params.insert("location".into(), city.to_string());
            return;
        }
    }

    // Pattern: "near me" — special marker
    if text.contains("near me") {
        params.insert("location".into(), "near_me".into());
    }
}

/// Extract budget from patterns like "under $X", "cheap", "expensive", "$$".
fn extract_budget(text: &str, params: &mut HashMap<String, String>) {
    // Dollar amount patterns
    if let Some(idx) = text.find("under $") {
        let after = &text[idx + 7..];
        if let Some(amount) = extract_number(after) {
            params.insert("budget".into(), format!("under_{}", amount));
            return;
        }
    }
    if let Some(idx) = text.find("under ") {
        let after = &text[idx + 6..];
        if let Some(amount) = extract_dollar_number(after) {
            params.insert("budget".into(), format!("under_{}", amount));
            return;
        }
    }
    if let Some(idx) = text.find("below $") {
        let after = &text[idx + 7..];
        if let Some(amount) = extract_number(after) {
            params.insert("budget".into(), format!("under_{}", amount));
            return;
        }
    }
    if let Some(idx) = text.find("less than $") {
        let after = &text[idx + 11..];
        if let Some(amount) = extract_number(after) {
            params.insert("budget".into(), format!("under_{}", amount));
            return;
        }
    }
    if let Some(idx) = text.find('$') {
        let after = &text[idx + 1..];
        if let Some(amount) = extract_number(after) {
            params.insert("budget".into(), format!("around_{}", amount));
            return;
        }
    }

    // Dollar sign tiers
    if text.contains("$$$$") {
        params.insert("budget".into(), "luxury".into());
    } else if text.contains("$$$") {
        params.insert("budget".into(), "expensive".into());
    } else if text.contains("$$") {
        params.insert("budget".into(), "moderate".into());
    }

    // Word-based
    if text.contains("cheap") || text.contains("budget") || text.contains("inexpensive") {
        params.insert("budget".into(), "cheap".into());
    } else if text.contains("expensive") || text.contains("luxury") || text.contains("upscale")
        || text.contains("fine dining")
    {
        params.insert("budget".into(), "expensive".into());
    } else if text.contains("moderate") || text.contains("mid-range") || text.contains("mid range")
    {
        params.insert("budget".into(), "moderate".into());
    }
}

/// Extract date/time from patterns like "tonight", "friday", "this weekend", "tomorrow".
fn extract_date(text: &str, params: &mut HashMap<String, String>) {
    let date_keywords = [
        ("tonight", "tonight"),
        ("this evening", "tonight"),
        ("tomorrow", "tomorrow"),
        ("tomorrow night", "tomorrow_night"),
        ("tomorrow evening", "tomorrow_evening"),
        ("this weekend", "this_weekend"),
        ("next weekend", "next_weekend"),
        ("this friday", "this_friday"),
        ("this saturday", "this_saturday"),
        ("this sunday", "this_sunday"),
        ("next week", "next_week"),
        ("today", "today"),
        ("right now", "now"),
        ("asap", "now"),
    ];

    // Check day names
    let days = [
        "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
    ];

    for (phrase, value) in &date_keywords {
        if text.contains(phrase) {
            params.insert("date".into(), value.to_string());
            return;
        }
    }

    for day in &days {
        if text.contains(day) {
            params.insert("date".into(), day.to_string());
            return;
        }
    }
}

/// Extract party size from patterns like "for N people", "party of N", "N guests".
fn extract_party_size(text: &str, params: &mut HashMap<String, String>) {
    let patterns = [
        ("for ", " people"),
        ("for ", " persons"),
        ("for ", " guests"),
        ("party of ", ""),
        ("group of ", ""),
        ("table for ", ""),
        ("seats for ", ""),
    ];

    for (prefix, suffix) in &patterns {
        if let Some(start) = text.find(prefix) {
            let after = &text[start + prefix.len()..];
            let end = if suffix.is_empty() {
                // Take next word
                after.find(' ').unwrap_or(after.len())
            } else if let Some(s) = after.find(suffix) {
                s
            } else {
                continue;
            };
            let num_str = after[..end].trim();
            if let Ok(n) = num_str.parse::<u32>() {
                if n > 0 && n < 100 {
                    params.insert("party_size".into(), n.to_string());
                    return;
                }
            }
            // Try word-numbers
            if let Some(n) = word_to_number(num_str) {
                params.insert("party_size".into(), n.to_string());
                return;
            }
        }
    }
}

/// Extract cuisine type for restaurant searches.
fn extract_cuisine(text: &str, params: &mut HashMap<String, String>) {
    // Check cuisine names
    for cuisine in CUISINES {
        if text.contains(cuisine) {
            params.insert("cuisine".into(), cuisine.to_string());
            return;
        }
    }

    // Check specific food items (maps to a "cuisine" in a broader sense)
    for food in FOOD_ITEMS {
        if text.contains(food) {
            params.insert("cuisine".into(), food.to_string());
            return;
        }
    }

    // Dietary preferences
    if text.contains("vegan") {
        params.insert("cuisine".into(), "vegan".into());
    } else if text.contains("vegetarian") {
        params.insert("cuisine".into(), "vegetarian".into());
    } else if text.contains("halal") {
        params.insert("cuisine".into(), "halal".into());
    } else if text.contains("kosher") {
        params.insert("cuisine".into(), "kosher".into());
    } else if text.contains("gluten free") || text.contains("gluten-free") {
        params.insert("cuisine".into(), "gluten_free".into());
    }
}

/// Extract a person name from the original (case-preserved) text.
fn extract_person_name(original: &str, lower: &str, params: &mut HashMap<String, String>) {
    // Pattern: "who is X" — take words after "who is"
    let name_triggers = ["who is ", "who was ", "find person ", "search for "];
    for trigger in &name_triggers {
        if let Some(idx) = lower.find(trigger) {
            let start = idx + trigger.len();
            let name = extract_name_from_position(original, start);
            if !name.is_empty() {
                params.insert("name".into(), name);
                return;
            }
        }
    }

    // Pattern: "find X on linkedin" / "search linkedin for X"
    if lower.contains("linkedin") || lower.contains("facebook") || lower.contains("twitter") {
        // Try "for X" pattern
        if let Some(idx) = lower.find(" for ") {
            let start = idx + 5;
            let name = extract_name_from_position(original, start);
            if !name.is_empty() {
                params.insert("name".into(), name);
                return;
            }
        }
    }
}

/// Extract a product query.
fn extract_product_query(text: &str, params: &mut HashMap<String, String>) {
    // "best X under $Y" — X is the query
    if let Some(idx) = text.find("best ") {
        let after = &text[idx + 5..];
        let end = after
            .find(" under")
            .or_else(|| after.find(" below"))
            .or_else(|| after.find(" for "))
            .unwrap_or(after.len());
        let query = after[..end].trim();
        if !query.is_empty() {
            params.insert("query".into(), query.to_string());
            return;
        }
    }

    // "where to buy X"
    if let Some(idx) = text.find("where to buy ") {
        let after = &text[idx + 13..];
        let query = extract_noun_phrase(after);
        if !query.is_empty() {
            params.insert("query".into(), query);
            return;
        }
    }

    // "price of X" / "how much does X cost"
    for prefix in &["price of ", "how much does ", "how much is "] {
        if let Some(idx) = text.find(prefix) {
            let after = &text[idx + prefix.len()..];
            let end = after.find(" cost").unwrap_or(after.len());
            let query = after[..end].trim();
            if !query.is_empty() {
                params.insert("query".into(), query.to_string());
                return;
            }
        }
    }
}

/// Extract a job search query.
fn extract_job_query(text: &str, params: &mut HashMap<String, String>) {
    // "jobs in X" — X could be a field or location; treat as query
    if let Some(idx) = text.find("jobs in ") {
        let after = &text[idx + 8..];
        let query = extract_noun_phrase(after);
        if !query.is_empty() {
            params.insert("query".into(), query);
            return;
        }
    }

    // "hiring X" / "career at X"
    for prefix in &["hiring ", "career at ", "position at ", "work at "] {
        if let Some(idx) = text.find(prefix) {
            let after = &text[idx + prefix.len()..];
            let query = extract_noun_phrase(after);
            if !query.is_empty() {
                params.insert("query".into(), query);
                return;
            }
        }
    }

    // "remote X jobs"
    if text.contains("remote") {
        params.insert("remote".into(), "true".into());
        // Try to extract the role
        if let Some(idx) = text.find("remote ") {
            let after = &text[idx + 7..];
            let end = after.find(" job").unwrap_or(after.len());
            let role = after[..end].trim();
            if !role.is_empty() && role != "job" && role != "jobs" {
                params.insert("query".into(), role.to_string());
                return;
            }
        }
    }
}

/// Extract service type for find_service.
fn extract_service_type(text: &str, params: &mut HashMap<String, String>) {
    let services = [
        ("plumber", "plumber"),
        ("plumbing", "plumber"),
        ("electrician", "electrician"),
        ("electrical", "electrician"),
        ("mechanic", "mechanic"),
        ("auto repair", "mechanic"),
        ("dentist", "dentist"),
        ("dental", "dentist"),
        ("doctor", "doctor"),
        ("physician", "doctor"),
        ("lawyer", "lawyer"),
        ("attorney", "lawyer"),
        ("legal", "lawyer"),
        ("accountant", "accountant"),
        ("cpa", "accountant"),
        ("therapist", "therapist"),
        ("counselor", "therapist"),
        ("contractor", "contractor"),
        ("handyman", "handyman"),
        ("cleaner", "cleaner"),
        ("cleaning", "cleaner"),
        ("mover", "mover"),
        ("moving", "mover"),
        ("tutor", "tutor"),
        ("tutoring", "tutor"),
        ("vet", "veterinarian"),
        ("veterinarian", "veterinarian"),
        ("barber", "barber"),
        ("hairdresser", "hairdresser"),
        ("salon", "salon"),
        ("gym", "gym"),
    ];

    for (keyword, service_type) in &services {
        if text.contains(keyword) {
            params.insert("service_type".into(), service_type.to_string());
            return;
        }
    }
}

// ── Helper Functions ──

/// Extract a "noun phrase" — consecutive words until a stop word or punctuation.
fn extract_noun_phrase(text: &str) -> String {
    let stop_words = [
        "and", "or", "but", "the", "a", "an", "is", "are", "was", "were",
        "that", "which", "who", "where", "when", "how", "what", "with",
        "for", "from", "to", "of", "on", "at", "by", "in", "near",
        "under", "below", "above", "please", "can", "could", "would",
        "should", "i", "me", "my", "you", "your",
    ];

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut result = Vec::new();

    for word in &words {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'');
        if clean.is_empty() {
            break;
        }
        let lower = clean.to_lowercase();
        // Stop at stop words, but allow first word even if it's a stop word
        if !result.is_empty() && stop_words.contains(&lower.as_str()) {
            break;
        }
        result.push(clean);
        // Cap at 5 words
        if result.len() >= 5 {
            break;
        }
    }

    result.join(" ")
}

/// Extract a name from a position in the original (case-preserved) text.
/// Names are sequences of capitalized words.
fn extract_name_from_position(original: &str, start: usize) -> String {
    if start >= original.len() {
        return String::new();
    }
    let remaining = &original[start..];
    let words: Vec<&str> = remaining.split_whitespace().collect();
    let mut name_parts = Vec::new();

    for word in &words {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'');
        if clean.is_empty() {
            break;
        }
        // Accept capitalized words as name parts, or all lowercase if it's the first word
        let first_char = clean.chars().next().unwrap_or(' ');
        if first_char.is_uppercase() || name_parts.is_empty() {
            name_parts.push(clean);
        } else {
            break;
        }
        // Max 4 words for a name
        if name_parts.len() >= 4 {
            break;
        }
    }

    // Filter out common non-name words that might sneak in
    let non_names = ["the", "a", "an", "on", "at", "from", "in"];
    let filtered: Vec<&str> = name_parts
        .into_iter()
        .filter(|w| !non_names.contains(&w.to_lowercase().as_str()))
        .collect();

    filtered.join(" ")
}

/// Extract a number from the start of text.
fn extract_number(text: &str) -> Option<u64> {
    let num_str: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Extract a number that might start with '$'.
fn extract_dollar_number(text: &str) -> Option<u64> {
    let trimmed = text.trim_start_matches('$');
    extract_number(trimmed)
}

/// Convert word-numbers to digits.
fn word_to_number(word: &str) -> Option<u32> {
    match word.to_lowercase().as_str() {
        "one" | "1" => Some(1),
        "two" | "2" => Some(2),
        "three" | "3" => Some(3),
        "four" | "4" => Some(4),
        "five" | "5" => Some(5),
        "six" | "6" => Some(6),
        "seven" | "7" => Some(7),
        "eight" | "8" => Some(8),
        "nine" | "9" => Some(9),
        "ten" | "10" => Some(10),
        "eleven" | "11" => Some(11),
        "twelve" | "12" => Some(12),
        _ => None,
    }
}

/// Check if a word is a common stop word (to skip for location extraction).
fn is_stop_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "the" | "a" | "an" | "and" | "or" | "but" | "is" | "are" | "was"
            | "were" | "it" | "that" | "this" | "for" | "to" | "of"
            | "with" | "me" | "my" | "i" | "we" | "you" | "your"
            | "some" | "any" | "good" | "best" | "great" | "nice"
    )
}

// ── Clarifying Question Generation ──

/// Generate natural clarifying questions for missing parameters.
///
/// Rules:
/// - Max 2 questions per message (don't overwhelm)
/// - Conversational tone, not form-filling
/// - Include suggestions when obvious
/// - If only optional fields missing, don't ask — just proceed with defaults
pub fn generate_clarifying_questions(intent: &ParsedIntent, required_missing: &[String]) -> Vec<ClarifyingQuestion> {
    if required_missing.is_empty() {
        return Vec::new();
    }

    let task_type = intent.task_type.as_deref().unwrap_or("");
    let mut questions = Vec::new();

    // Try to batch related questions together
    let has_missing_location = required_missing.iter().any(|p| p == "location");
    let has_missing_cuisine = required_missing.iter().any(|p| p == "cuisine");
    let has_missing_name = required_missing.iter().any(|p| p == "name");
    let has_missing_query = required_missing.iter().any(|p| p == "query");
    let has_missing_service = required_missing.iter().any(|p| p == "service_type");

    // Batch: cuisine + location for restaurants
    if task_type == "find_restaurant" && has_missing_cuisine && has_missing_location {
        questions.push(ClarifyingQuestion {
            question: "What kind of food are you in the mood for, and where should I look? (e.g., 'Thai food near downtown')".into(),
            resolves: vec!["cuisine".into(), "location".into()],
            suggestions: vec![
                "Italian near me".into(),
                "Thai food downtown".into(),
                "sushi anywhere".into(),
            ],
        });
    } else {
        // Individual questions
        if has_missing_location {
            let q = match task_type {
                "find_restaurant" => ClarifyingQuestion {
                    question: "Where should I search? Any particular neighborhood or city?".into(),
                    resolves: vec!["location".into()],
                    suggestions: vec!["near me".into(), "downtown".into()],
                },
                "find_hotel" => ClarifyingQuestion {
                    question: "Where are you planning to stay? Which city or area?".into(),
                    resolves: vec!["location".into()],
                    suggestions: vec![],
                },
                "find_service" => ClarifyingQuestion {
                    question: "Where should I look for this service? Your area or a specific city?".into(),
                    resolves: vec!["location".into()],
                    suggestions: vec!["near me".into()],
                },
                _ => ClarifyingQuestion {
                    question: "Where should I search? Any particular area?".into(),
                    resolves: vec!["location".into()],
                    suggestions: vec!["near me".into()],
                },
            };
            questions.push(q);
        }

        if has_missing_cuisine {
            questions.push(ClarifyingQuestion {
                question: "What kind of food are you in the mood for?".into(),
                resolves: vec!["cuisine".into()],
                suggestions: vec![
                    "Italian".into(),
                    "Thai".into(),
                    "Mexican".into(),
                    "anything good".into(),
                ],
            });
        }
    }

    if has_missing_name {
        questions.push(ClarifyingQuestion {
            question: "Who are you looking for? Full name helps me find the right person.".into(),
            resolves: vec!["name".into()],
            suggestions: vec![],
        });
    }

    if has_missing_query {
        let q = match task_type {
            "find_product" => ClarifyingQuestion {
                question: "What product are you looking for? Any specific features or brand preference?".into(),
                resolves: vec!["query".into()],
                suggestions: vec![],
            },
            "find_job" => ClarifyingQuestion {
                question: "What kind of role or field are you interested in?".into(),
                resolves: vec!["query".into()],
                suggestions: vec!["remote software engineer".into(), "marketing manager".into()],
            },
            _ => ClarifyingQuestion {
                question: "Can you tell me more about what you're looking for?".into(),
                resolves: vec!["query".into()],
                suggestions: vec![],
            },
        };
        questions.push(q);
    }

    if has_missing_service {
        questions.push(ClarifyingQuestion {
            question: "What kind of service do you need? (e.g., plumber, dentist, mechanic)".into(),
            resolves: vec!["service_type".into()],
            suggestions: vec![
                "plumber".into(),
                "electrician".into(),
                "dentist".into(),
                "mechanic".into(),
            ],
        });
    }

    // Max 2 questions per message
    questions.truncate(2);
    questions
}

/// Format clarifying questions into a response string for the LLM to relay.
pub fn format_clarifying_response(intent: &ParsedIntent, questions: &[ClarifyingQuestion]) -> String {
    let task_label = intent.task_type.as_deref().unwrap_or("search");
    let mut parts = Vec::new();

    parts.push(format!(
        "[NEEDS_CLARIFICATION] task_type={}, confidence={:.2}",
        task_label, intent.confidence
    ));

    if !intent.params.is_empty() {
        let known: Vec<String> = intent
            .params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        parts.push(format!("Known: {}", known.join(", ")));
    }

    parts.push(format!(
        "Missing: {}",
        intent.missing_params.join(", ")
    ));

    for q in questions {
        let mut line = format!("Q: {}", q.question);
        if !q.suggestions.is_empty() {
            line.push_str(&format!(" [suggestions: {}]", q.suggestions.join(", ")));
        }
        parts.push(line);
    }

    parts.join("\n")
}

/// Format a fully-resolved intent for the orchestration layer.
pub fn format_ready_intent(intent: &ParsedIntent) -> String {
    let task_label = intent.task_type.as_deref().unwrap_or("unknown");
    let mut parts = Vec::new();

    parts.push(format!(
        "[READY] task_type={}, confidence={:.2}",
        task_label, intent.confidence
    ));

    for (k, v) in &intent.params {
        parts.push(format!("  {}={}", k, v));
    }

    // Serialize as JSON too for machine consumption
    if let Ok(json) = serde_json::to_string(intent) {
        parts.push(format!("JSON: {}", json));
    }

    parts.join("\n")
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restaurant_basic() {
        let intent = detect_intent("find me a good pizza place near downtown");
        assert!(intent.is_life_task);
        assert_eq!(intent.task_type.as_deref(), Some("find_restaurant"));
        assert_eq!(intent.params.get("cuisine"), Some(&"pizza".to_string()));
        assert_eq!(intent.params.get("location"), Some(&"downtown".to_string()));
    }

    #[test]
    fn test_restaurant_missing_location() {
        let intent = detect_intent("I want sushi tonight");
        assert!(intent.is_life_task);
        assert_eq!(intent.task_type.as_deref(), Some("find_restaurant"));
        assert_eq!(intent.params.get("cuisine"), Some(&"sushi".to_string()));
        assert_eq!(intent.params.get("date"), Some(&"tonight".to_string()));
        assert!(intent.missing_params.contains(&"location".to_string()));
    }

    #[test]
    fn test_person_search() {
        let intent = detect_intent("who is John Smith");
        assert!(intent.is_life_task);
        assert_eq!(intent.task_type.as_deref(), Some("find_person"));
        assert_eq!(intent.params.get("name"), Some(&"John Smith".to_string()));
    }

    #[test]
    fn test_product_search() {
        let intent = detect_intent("best laptop under $500");
        assert!(intent.is_life_task);
        assert_eq!(intent.task_type.as_deref(), Some("find_product"));
        assert!(intent.params.contains_key("query"));
        assert!(intent.params.contains_key("budget"));
    }

    #[test]
    fn test_job_search() {
        let intent = detect_intent("remote jobs in rust programming");
        assert!(intent.is_life_task);
        assert_eq!(intent.task_type.as_deref(), Some("find_job"));
    }

    #[test]
    fn test_hotel_search() {
        let intent = detect_intent("hotel in paris this weekend");
        assert!(intent.is_life_task);
        assert_eq!(intent.task_type.as_deref(), Some("find_hotel"));
        assert_eq!(intent.params.get("location"), Some(&"paris".to_string()));
        assert_eq!(intent.params.get("date"), Some(&"this_weekend".to_string()));
    }

    #[test]
    fn test_service_search() {
        let intent = detect_intent("find a good plumber near me");
        assert!(intent.is_life_task);
        assert_eq!(intent.task_type.as_deref(), Some("find_service"));
        assert_eq!(intent.params.get("service_type"), Some(&"plumber".to_string()));
        assert_eq!(intent.params.get("location"), Some(&"near_me".to_string()));
    }

    #[test]
    fn test_not_life_task() {
        let intent = detect_intent("what time is it");
        assert!(!intent.is_life_task);
        assert!(intent.task_type.is_none());
    }

    #[test]
    fn test_not_life_task_code() {
        let intent = detect_intent("help me write a rust function");
        assert!(!intent.is_life_task);
    }

    #[test]
    fn test_party_size() {
        let intent = detect_intent("restaurant for 4 people near boston");
        assert!(intent.is_life_task);
        assert_eq!(intent.params.get("party_size"), Some(&"4".to_string()));
    }

    #[test]
    fn test_budget_extraction() {
        let intent = detect_intent("cheap italian food in new york");
        assert!(intent.is_life_task);
        assert_eq!(intent.params.get("budget"), Some(&"cheap".to_string()));
        assert_eq!(intent.params.get("cuisine"), Some(&"italian".to_string()));
    }

    #[test]
    fn test_clarifying_questions_restaurant_missing_all() {
        let intent = ParsedIntent {
            task_type: Some("find_restaurant".into()),
            confidence: 0.5,
            params: HashMap::new(),
            missing_params: vec!["cuisine".into(), "location".into()],
            is_life_task: true,
        };
        let questions = generate_clarifying_questions(&intent, &intent.missing_params);
        assert!(!questions.is_empty());
        assert!(questions.len() <= 2);
        // Should batch cuisine + location into one question
        assert!(questions[0].resolves.contains(&"cuisine".into()));
        assert!(questions[0].resolves.contains(&"location".into()));
    }

    #[test]
    fn test_clarifying_questions_person_missing_name() {
        let intent = ParsedIntent {
            task_type: Some("find_person".into()),
            confidence: 0.4,
            params: HashMap::new(),
            missing_params: vec!["name".into()],
            is_life_task: true,
        };
        let questions = generate_clarifying_questions(&intent, &intent.missing_params);
        assert_eq!(questions.len(), 1);
        assert!(questions[0].resolves.contains(&"name".into()));
    }

    #[test]
    fn test_clarifying_questions_none_when_complete() {
        let intent = ParsedIntent {
            task_type: Some("find_restaurant".into()),
            confidence: 0.8,
            params: HashMap::from([
                ("cuisine".into(), "thai".into()),
                ("location".into(), "downtown".into()),
            ]),
            missing_params: vec![],
            is_life_task: true,
        };
        let questions = generate_clarifying_questions(&intent, &intent.missing_params);
        assert!(questions.is_empty());
    }

    #[test]
    fn test_max_two_questions() {
        let intent = ParsedIntent {
            task_type: Some("find_service".into()),
            confidence: 0.4,
            params: HashMap::new(),
            missing_params: vec![
                "service_type".into(),
                "location".into(),
                "budget".into(),
            ],
            is_life_task: true,
        };
        let questions = generate_clarifying_questions(&intent, &intent.missing_params);
        assert!(questions.len() <= 2);
    }

    #[test]
    fn test_format_clarifying_response() {
        let intent = ParsedIntent {
            task_type: Some("find_restaurant".into()),
            confidence: 0.5,
            params: HashMap::from([("cuisine".into(), "thai".into())]),
            missing_params: vec!["location".into()],
            is_life_task: true,
        };
        let questions = generate_clarifying_questions(&intent, &intent.missing_params);
        let response = format_clarifying_response(&intent, &questions);
        assert!(response.contains("[NEEDS_CLARIFICATION]"));
        assert!(response.contains("find_restaurant"));
        assert!(response.contains("Missing: location"));
    }

    #[test]
    fn test_format_ready_intent() {
        let intent = ParsedIntent {
            task_type: Some("find_restaurant".into()),
            confidence: 0.8,
            params: HashMap::from([
                ("cuisine".into(), "thai".into()),
                ("location".into(), "downtown".into()),
            ]),
            missing_params: vec![],
            is_life_task: true,
        };
        let response = format_ready_intent(&intent);
        assert!(response.contains("[READY]"));
        assert!(response.contains("find_restaurant"));
        assert!(response.contains("JSON:"));
    }
}
