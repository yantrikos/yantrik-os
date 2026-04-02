//! Lightweight tool registry with keyword + TF-IDF matching.
//!
//! Simulates YantrikDB embedding search for tool selection.
//! The Perceiver column extracts intent, then this registry finds the best tool.

use std::collections::HashMap;

/// Tool descriptor for the registry.
struct Tool {
    name: String,
    description: String,
    category: String,
    execute_fn: Option<fn(&str) -> String>,
}

/// Lightweight TF-IDF tool matcher.
pub struct ToolRegistry {
    tools: Vec<Tool>,
    vocab: HashMap<String, usize>,
    idf: HashMap<String, f32>,
    tool_vecs: Vec<Vec<f32>>,
}

impl ToolRegistry {
    /// Create a new registry with built-in tools.
    pub fn new() -> Self {
        let mut reg = Self {
            tools: Vec::new(),
            vocab: HashMap::new(),
            idf: HashMap::new(),
            tool_vecs: Vec::new(),
        };
        reg.register_builtins();
        reg.build_index();
        reg
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Find the best matching tool for a query.
    pub fn best_match(&self, query: &str, _intent: &str) -> (String, f32) {
        let q = query.to_lowercase();

        // ── Keyword hard overrides ──
        // Math
        if has_math_keywords(&q) {
            return ("calculate".into(), 0.95);
        }
        // Unit conversion
        if has_unit_keywords(&q) {
            return ("unit_convert".into(), 0.95);
        }
        // Time
        if ["what time", "current time", "today's date", "what day", "what date"]
            .iter()
            .any(|w| q.contains(w))
        {
            return ("current_time".into(), 0.95);
        }
        // Weather
        if q.contains("weather") {
            return ("get_weather".into(), 0.95);
        }
        // Git
        if q.starts_with("git ") || q.contains("git status") || q.contains("git diff") {
            return ("git_status".into(), 0.90);
        }
        // Factual recall
        if ["who ", "what ", "when ", "where ", "tell me about", "capital of"]
            .iter()
            .any(|w| q.contains(w))
        {
            return ("recall".into(), 0.80);
        }

        // ── TF-IDF fallback ──
        self.tfidf_match(&q)
    }

    /// Execute a tool by name.
    pub fn execute(&self, tool_name: &str, query: &str) -> String {
        for tool in &self.tools {
            if tool.name == tool_name {
                if let Some(f) = tool.execute_fn {
                    return f(query);
                }
                return format!("[Executed {tool_name}]");
            }
        }
        format!("[Unknown tool: {tool_name}]")
    }

    // ── Private ──

    fn register(&mut self, name: &str, desc: &str, category: &str, execute_fn: Option<fn(&str) -> String>) {
        self.tools.push(Tool {
            name: name.into(),
            description: desc.into(),
            category: category.into(),
            execute_fn,
        });
    }

    fn register_builtins(&mut self) {
        // Computation
        self.register("calculate", "Evaluate mathematical expression arithmetic add subtract multiply divide percentage", "math", Some(calc_fn));
        self.register("unit_convert", "Convert between units miles kilometers pounds kilograms fahrenheit celsius", "math", Some(unit_fn));

        // Time
        self.register("current_time", "Get current date time day month year clock now today", "time", Some(time_fn));
        self.register("set_reminder", "Set a reminder alarm for future time event", "time", None);

        // Search & Knowledge
        self.register("web_search", "Search internet web for current information news", "search", None);
        self.register("recall", "Search retrieve find relevant memories from past conversations", "memory", None);
        self.register("remember", "Store save note keep a new memory fact", "memory", None);

        // Code
        self.register("code_execute", "Write run execute code program script function", "code", None);

        // Files
        self.register("read_file", "Read open show contents of a file document", "files", None);
        self.register("write_file", "Write create save a file document", "files", None);
        self.register("search_files", "Search find locate files by name content pattern", "files", None);

        // System
        self.register("run_command", "Run execute terminal shell command", "system", None);
        self.register("system_info", "Get system status CPU memory RAM disk usage", "system", None);

        // Communication
        self.register("send_email", "Compose send write an email message", "communication", None);
        self.register("send_notification", "Send desktop notification alert", "communication", None);

        // Git
        self.register("git_status", "Check git status repository changes staged", "git", None);
        self.register("git_commit", "Create git commit save changes", "git", None);

        // Creative
        self.register("write_text", "Write compose creative text story poem article", "creative", None);
        self.register("translate", "Translate text between languages", "creative", None);
        self.register("summarize", "Summarize condense long text document", "creative", None);

        // Conversation (fallback)
        self.register("respond", "Generate conversational response greeting thanks goodbye", "conversation", None);

        // Weather
        self.register("get_weather", "Get current weather forecast temperature", "weather", None);
    }

    fn build_index(&mut self) {
        let mut all_words: Vec<String> = Vec::new();
        let mut doc_words: Vec<Vec<String>> = Vec::new();

        for tool in &self.tools {
            let text = format!("{} {} {}", tool.name, tool.description, tool.category).to_lowercase();
            let words: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();
            for w in &words {
                if !all_words.contains(w) {
                    all_words.push(w.clone());
                }
            }
            doc_words.push(words);
        }

        all_words.sort();
        self.vocab = all_words.iter().enumerate().map(|(i, w)| (w.clone(), i)).collect();

        let doc_count = self.tools.len() as f32;
        let mut word_doc_freq: HashMap<String, usize> = HashMap::new();
        for words in &doc_words {
            let unique: std::collections::HashSet<_> = words.iter().collect();
            for w in unique {
                *word_doc_freq.entry(w.clone()).or_insert(0) += 1;
            }
        }
        self.idf = self.vocab.keys().map(|w| {
            let df = *word_doc_freq.get(w).unwrap_or(&0) as f32;
            (w.clone(), (doc_count / (1.0 + df)).ln())
        }).collect();

        self.tool_vecs = doc_words.iter().map(|words| {
            let mut vec = vec![0.0f32; self.vocab.len()];
            let mut counts: HashMap<&String, usize> = HashMap::new();
            for w in words {
                *counts.entry(w).or_insert(0) += 1;
            }
            for (w, count) in &counts {
                if let Some(&idx) = self.vocab.get(w.as_str()) {
                    let tf = *count as f32 / words.len() as f32;
                    vec[idx] = tf * self.idf.get(w.as_str()).unwrap_or(&0.0);
                }
            }
            let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            }
            vec
        }).collect();
    }

    fn tfidf_match(&self, query: &str) -> (String, f32) {
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut vec = vec![0.0f32; self.vocab.len()];
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for w in &words {
            *counts.entry(w).or_insert(0) += 1;
        }
        for (w, count) in &counts {
            if let Some(&idx) = self.vocab.get(*w) {
                let tf = count.clone() as f32 / words.len() as f32;
                vec[idx] = tf * self.idf.get(*w).unwrap_or(&0.0);
            }
        }
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }

        let mut best = ("respond".to_string(), 0.0f32);
        for (i, tool_vec) in self.tool_vecs.iter().enumerate() {
            let sim: f32 = vec.iter().zip(tool_vec.iter()).map(|(a, b)| a * b).sum();
            if sim > best.1 {
                best = (self.tools[i].name.clone(), sim);
            }
        }
        best
    }
}

// ── Tool implementations ──────────────────────────────────────────────

fn has_math_keywords(q: &str) -> bool {
    ["times", "plus", "minus", "divided", "multiply", "percent", "sqrt", "square root"]
        .iter()
        .any(|w| q.contains(w))
        || q.chars().any(|c| matches!(c, '+' | '*' | '/' | '^'))
            && q.chars().any(|c| c.is_ascii_digit())
}

fn has_unit_keywords(q: &str) -> bool {
    let units = ["miles", "km", "kilometers", "pounds", "kg", "feet", "meters",
                 "inches", "cm", "fahrenheit", "celsius", "gallons", "liters"];
    let has_unit = units.iter().any(|u| q.contains(u));
    let has_to = q.contains(" to ") || q.contains(" in ");
    has_unit && has_to && q.chars().any(|c| c.is_ascii_digit())
}

fn calc_fn(query: &str) -> String {
    let mut expr = query.to_lowercase();
    for p in ["what is ", "calculate ", "compute ", "what's ", "how much is "] {
        expr = expr.replace(p, "");
    }
    expr = expr
        .replace("times", "*")
        .replace("plus", "+")
        .replace("minus", "-")
        .replace("divided by", "/");
    expr = expr.trim().trim_end_matches('?').trim().to_string();

    // Handle "X% of Y" → (X/100)*Y
    if let Some(caps) = regex_lite::Regex::new(r"(\d+)\s*%\s*of\s*(\d+)")
        .ok()
        .and_then(|re| re.captures(&expr))
    {
        let pct: f64 = caps[1].parse().unwrap_or(0.0);
        let base: f64 = caps[2].parse().unwrap_or(0.0);
        let result = (pct / 100.0) * base;
        return if result == result.floor() {
            format!("{}", result as i64)
        } else {
            format!("{:.4}", result)
        };
    }

    // Simple expression evaluation (digits and operators only)
    // For safety, only allow: digits, +, -, *, /, ., (, ), spaces
    let safe: String = expr.chars().filter(|c| "0123456789+-*/.() ".contains(*c)).collect();
    if safe.is_empty() {
        return format!("Error: {expr}");
    }

    // Use a simple recursive descent or just evaluate with basic ops
    match eval_simple(&safe) {
        Some(r) => {
            if r == r.floor() && r.abs() < 1e15 {
                format!("{}", r as i64)
            } else {
                format!("{:.4}", r)
            }
        }
        None => format!("Error: {safe}"),
    }
}

/// Very simple expression evaluator (no dependencies).
fn eval_simple(expr: &str) -> Option<f64> {
    let expr = expr.trim();
    // Try to parse as a simple "a op b" expression
    let ops = [("*", '*'), ("+", '+'), ("-", '-'), ("/", '/')];
    for (op_str, _op_char) in &ops {
        // Split on last occurrence of operator (to handle negative numbers)
        if let Some(pos) = expr.rfind(op_str) {
            if pos > 0 && pos < expr.len() - 1 {
                let left = &expr[..pos].trim();
                let right = &expr[pos + 1..].trim();
                if let (Some(l), Some(r)) = (eval_simple(left), eval_simple(right)) {
                    return match *op_str {
                        "+" => Some(l + r),
                        "-" => Some(l - r),
                        "*" => Some(l * r),
                        "/" => {
                            if r != 0.0 {
                                Some(l / r)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                }
            }
        }
    }
    // Base case: parse as number
    expr.parse::<f64>().ok()
}

fn unit_fn(query: &str) -> String {
    let q = query.to_lowercase();
    // Parse "N unit to unit"
    let re = regex_lite::Regex::new(
        r"(\d+\.?\d*)\s*(miles?|kilometers?|km|pounds?|kg|feet|foot|meters?|inches?|cm|fahrenheit|celsius|gallons?|liters?)\s*(?:to|in)\s*(miles?|kilometers?|km|pounds?|kg|feet|foot|meters?|inches?|cm|fahrenheit|celsius|gallons?|liters?)"
    ).ok();
    let caps = re.as_ref().and_then(|re| re.captures(&q));
    let caps = match caps {
        Some(c) => c,
        None => return "Could not parse conversion.".into(),
    };

    let val: f64 = caps[1].parse().unwrap_or(0.0);
    let from = &caps[2];
    let to = &caps[3];

    // Temperature
    if from.starts_with('f') && to.starts_with('c') {
        return format!("{val} Fahrenheit = {:.1} Celsius", (val - 32.0) * 5.0 / 9.0);
    }
    if from.starts_with('c') && to.starts_with('f') {
        return format!("{val} Celsius = {:.1} Fahrenheit", val * 9.0 / 5.0 + 32.0);
    }

    let conversions: &[(&str, &str, f64)] = &[
        ("mile", "kilo", 1.60934),
        ("kilo", "mile", 0.621371),
        ("km", "mile", 0.621371),
        ("pound", "kilo", 0.453592),
        ("kg", "pound", 2.20462),
        ("feet", "meter", 0.3048),
        ("foot", "meter", 0.3048),
        ("meter", "feet", 3.28084),
        ("inch", "cm", 2.54),
    ];

    let fn_norm = from.trim_end_matches('s');
    let tn_norm = to.trim_end_matches('s');

    for &(a, b, factor) in conversions {
        if fn_norm.starts_with(&a[..a.len().min(3)]) && tn_norm.starts_with(&b[..b.len().min(3)]) {
            return format!("{val} {from} = {:.2} {to}", val * factor);
        }
    }

    format!("Unknown: {from} -> {to}")
}

fn time_fn(_query: &str) -> String {
    let now = chrono::Local::now();
    now.format("It's %A, %B %d, %Y at %I:%M %p.").to_string()
}
