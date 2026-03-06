//! Focus detection — infer what the user is working on.
//!
//! Uses heuristics on window title, process name, and system state
//! to determine the user's current activity. Maps to cortex entities
//! via relationship lookups (e.g., file → ticket).
//!
//! Runs on the 10-second system poll — no LLM calls, <1ms.

use rusqlite::Connection;

use super::schema;

// ── Core Types ───────────────────────────────────────────────────────

/// What kind of activity the user is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum ActivityType {
    Coding,
    Reviewing,
    Browsing,
    Emailing,
    Meeting,
    Terminal,
    Idle,
    Unknown,
}

impl ActivityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Reviewing => "reviewing",
            Self::Browsing => "browsing",
            Self::Emailing => "emailing",
            Self::Meeting => "meeting",
            Self::Terminal => "terminal",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
        }
    }
}

/// Structured context about what the user is currently doing.
#[derive(Debug, Clone)]
pub struct FocusContext {
    pub activity: ActivityType,
    pub active_file: Option<String>,
    pub active_project: Option<String>,
    pub linked_ticket: Option<String>,
    pub window_title: String,
    pub process_name: String,
    pub duration_seconds: u64,
    pub confidence: f32,
}

// ── Focus Detector ───────────────────────────────────────────────────

/// Detects the user's current focus from system signals.
///
/// Maintains state across updates to track focus duration.
pub struct FocusDetector {
    current: Option<FocusContext>,
    last_activity: ActivityType,
    activity_start_ts: f64,
}

impl FocusDetector {
    pub fn new() -> Self {
        Self {
            current: None,
            last_activity: ActivityType::Unknown,
            activity_start_ts: now_ts(),
        }
    }

    /// Update focus from system poll data.
    ///
    /// Called every 10 seconds from system_poll.rs.
    pub fn update(
        &mut self,
        conn: &Connection,
        window_title: &str,
        process_name: &str,
        idle_seconds: u64,
    ) {
        let now = now_ts();

        // Detect activity type from signals
        let (activity, confidence) = classify_activity(window_title, process_name, idle_seconds);

        // Track duration
        if activity != self.last_activity {
            self.last_activity = activity.clone();
            self.activity_start_ts = now;
        }
        let duration = (now - self.activity_start_ts) as u64;

        // Extract file path from window title
        let active_file = extract_file_from_title(window_title);

        // Look up linked ticket via cortex relationships
        let linked_ticket = if let Some(ref file) = active_file {
            find_linked_ticket(conn, file)
        } else {
            None
        };

        // Extract project name from file path or window title
        let active_project = active_file
            .as_ref()
            .and_then(|f| extract_project_name(f))
            .or_else(|| extract_project_from_title(window_title));

        // Boost relevance for actively-focused entities
        if let Some(ref ticket) = linked_ticket {
            schema::boost_relevance(conn, &format!("ticket:{}", ticket.to_lowercase()), 0.02);
        }
        if let Some(ref file) = active_file {
            schema::boost_relevance(conn, &format!("file:{}", file.to_lowercase()), 0.01);
        }

        self.current = Some(FocusContext {
            activity,
            active_file,
            active_project,
            linked_ticket,
            window_title: window_title.to_string(),
            process_name: process_name.to_string(),
            duration_seconds: duration,
            confidence,
        });
    }

    /// Get the current focus context.
    pub fn current_focus(&self) -> Option<FocusContext> {
        self.current.clone()
    }
}

// ── Classification Heuristics ────────────────────────────────────────

/// Classify user activity from system signals. Returns (activity, confidence).
fn classify_activity(
    window_title: &str,
    process_name: &str,
    idle_seconds: u64,
) -> (ActivityType, f32) {
    let title_lower = window_title.to_lowercase();
    let process_lower = process_name.to_lowercase();

    // Idle detection (highest priority)
    if idle_seconds > 300 {
        return (ActivityType::Idle, 0.95);
    }

    // Meeting detection
    if process_lower.contains("zoom")
        || process_lower.contains("teams")
        || process_lower.contains("meet")
        || process_lower.contains("webex")
        || title_lower.contains("zoom meeting")
        || title_lower.contains("microsoft teams")
    {
        return (ActivityType::Meeting, 0.9);
    }

    // Coding detection
    if process_lower.contains("code")
        || process_lower.contains("vim")
        || process_lower.contains("nvim")
        || process_lower.contains("emacs")
        || process_lower.contains("idea")
        || process_lower.contains("pycharm")
        || process_lower.contains("webstorm")
        || process_lower.contains("sublime")
    {
        // Extra confidence if file extension visible
        if has_code_extension(&title_lower) {
            return (ActivityType::Coding, 0.95);
        }
        return (ActivityType::Coding, 0.85);
    }

    // Terminal detection
    if process_lower.contains("terminal")
        || process_lower.contains("alacritty")
        || process_lower.contains("kitty")
        || process_lower.contains("wezterm")
        || process_lower.contains("bash")
        || process_lower.contains("zsh")
        || process_lower.contains("powershell")
        || process_lower.contains("cmd")
    {
        return (ActivityType::Terminal, 0.8);
    }

    // Email detection
    if title_lower.contains("gmail")
        || title_lower.contains("outlook")
        || title_lower.contains("mail")
        || title_lower.contains("thunderbird")
        || process_lower.contains("thunderbird")
    {
        return (ActivityType::Emailing, 0.85);
    }

    // Jira/Review detection
    if title_lower.contains("jira")
        || title_lower.contains("atlassian")
        || title_lower.contains("github.com/")
        || title_lower.contains("gitlab.com/")
        || title_lower.contains("pull request")
        || title_lower.contains("code review")
    {
        return (ActivityType::Reviewing, 0.85);
    }

    // General browsing
    if process_lower.contains("chrome")
        || process_lower.contains("firefox")
        || process_lower.contains("brave")
        || process_lower.contains("edge")
        || process_lower.contains("safari")
    {
        return (ActivityType::Browsing, 0.7);
    }

    (ActivityType::Unknown, 0.3)
}

/// Check if a title contains a code file extension.
fn has_code_extension(title: &str) -> bool {
    let extensions = [
        ".rs", ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".java",
        ".cpp", ".c", ".h", ".rb", ".php", ".swift", ".kt", ".scala",
        ".html", ".css", ".scss", ".vue", ".svelte", ".slint",
        ".toml", ".yaml", ".yml", ".json", ".xml", ".sql",
    ];
    extensions.iter().any(|ext| title.contains(ext))
}

/// Extract a file path from an editor window title.
///
/// Common patterns:
/// - "filename.rs — Project — Visual Studio Code"
/// - "filename.rs - Sublime Text"
/// - "VIM - filename.rs"
fn extract_file_from_title(title: &str) -> Option<String> {
    // VSCode pattern: "filename — folder — Visual Studio Code"
    if title.contains("Visual Studio Code") || title.contains("VS Code") || title.contains("Code - OSS") {
        let parts: Vec<&str> = title.split(" — ").collect();
        if !parts.is_empty() {
            let file = parts[0].trim();
            if has_code_extension(&file.to_lowercase()) || file.contains('.') {
                return Some(file.to_string());
            }
        }
        // Also try dash separator
        let parts: Vec<&str> = title.split(" - ").collect();
        if !parts.is_empty() {
            let file = parts[0].trim();
            if has_code_extension(&file.to_lowercase()) {
                return Some(file.to_string());
            }
        }
    }

    // Vim/Neovim pattern: "filename.rs - VIM" or "NVIM filename.rs"
    if title.contains("VIM") || title.contains("NVIM") {
        for part in title.split(|c: char| c == '-' || c == ' ') {
            let trimmed = part.trim();
            if has_code_extension(&trimmed.to_lowercase()) {
                return Some(trimmed.to_string());
            }
        }
    }

    // Sublime pattern: "filename.rs - Sublime Text"
    if title.contains("Sublime") {
        if let Some(file) = title.split(" - ").next() {
            let file = file.trim();
            if has_code_extension(&file.to_lowercase()) {
                return Some(file.to_string());
            }
        }
    }

    // JetBrains pattern: "[project] - filename.rs"
    if title.contains("IntelliJ") || title.contains("PyCharm") || title.contains("WebStorm") {
        for part in title.split(|c: char| c == '-' || c == '[' || c == ']') {
            let trimmed = part.trim();
            if has_code_extension(&trimmed.to_lowercase()) {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

/// Find a Jira ticket linked to a file via cortex relationships.
fn find_linked_ticket(conn: &Connection, file: &str) -> Option<String> {
    let file_entity = format!("file:{}", file.to_lowercase());

    // Check direct "works_on" relationship from file to ticket
    let related = schema::find_related(conn, &file_entity, "works_on");
    for entity_id in &related {
        if entity_id.starts_with("ticket:") {
            return Some(entity_id.strip_prefix("ticket:").unwrap_or(entity_id).to_uppercase());
        }
    }

    // Check "committed_to" relationships (file was in a commit that references a ticket)
    let committed = schema::find_related(conn, &file_entity, "committed_to");
    for entity_id in &committed {
        if entity_id.starts_with("ticket:") {
            return Some(entity_id.strip_prefix("ticket:").unwrap_or(entity_id).to_uppercase());
        }
    }

    None
}

/// Extract project name from a file path.
fn extract_project_name(file_path: &str) -> Option<String> {
    // Common patterns: "src/project/file.rs", "/home/user/project/src/file.rs"
    let parts: Vec<&str> = file_path.split('/').collect();
    if parts.len() >= 2 {
        // Look for common project markers
        for (i, part) in parts.iter().enumerate() {
            if *part == "src" || *part == "lib" || *part == "app" || *part == "crates" {
                if i > 0 {
                    return Some(parts[i - 1].to_string());
                }
            }
        }
    }
    None
}

/// Extract project name from window title.
fn extract_project_from_title(title: &str) -> Option<String> {
    // VSCode: "file — project — Visual Studio Code"
    if title.contains("Visual Studio Code") || title.contains("Code - OSS") {
        let parts: Vec<&str> = title.split(" — ").collect();
        if parts.len() >= 3 {
            return Some(parts[1].trim().to_string());
        }
    }
    None
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_activity() {
        let (a, _) = classify_activity("auth.rs — yantrik-os — Visual Studio Code", "code", 0);
        assert_eq!(a, ActivityType::Coding);

        let (a, _) = classify_activity("Zoom Meeting", "zoom", 0);
        assert_eq!(a, ActivityType::Meeting);

        let (a, _) = classify_activity("anything", "anything", 600);
        assert_eq!(a, ActivityType::Idle);

        let (a, _) = classify_activity("Gmail - Inbox", "chrome", 0);
        assert_eq!(a, ActivityType::Emailing);

        let (a, _) = classify_activity("YOS board - Jira", "firefox", 0);
        assert_eq!(a, ActivityType::Reviewing);
    }

    #[test]
    fn test_extract_file_from_title() {
        assert_eq!(
            extract_file_from_title("auth.rs — yantrik-os — Visual Studio Code"),
            Some("auth.rs".to_string())
        );
        assert_eq!(
            extract_file_from_title("Zoom Meeting"),
            None
        );
    }
}
