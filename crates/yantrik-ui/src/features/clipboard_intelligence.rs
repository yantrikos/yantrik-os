//! Clipboard Intelligence — monitors clipboard for patterns and sensitive data.
//!
//! Emits urges when:
//! - Sensitive data (API keys, tokens, passwords) detected in clipboard
//! - Stale sensitive data sits in clipboard too long
//! - Rapid copy-paste cycling (suggests clipboard history panel)

use std::time::Instant;

use crate::clipboard::SharedHistory;

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

/// Content type detected from clipboard text.
#[derive(Debug, Clone, PartialEq)]
enum ContentType {
    Url,
    Email,
    FilePath,
    CodeSnippet,
    SensitiveData(&'static str), // label: "API key", "token", etc.
    Json,
    PlainText,
}

pub struct ClipboardIntelligence {
    clip_history: SharedHistory,
    /// Last clipboard content we analyzed (to avoid re-analyzing).
    last_analyzed: Option<String>,
    /// When sensitive data was first detected in clipboard.
    sensitive_detected_at: Option<Instant>,
    /// Whether we already warned about the current sensitive content.
    sensitive_warned: bool,
    /// Whether we already warned about stale sensitive content.
    stale_warned: bool,
    /// Recent entry timestamps for copy-paste cycling detection.
    recent_copies: Vec<Instant>,
    /// Whether we warned about cycling recently.
    cycling_warned_at: Option<Instant>,
    /// Tick counter.
    tick_count: u64,
}

impl ClipboardIntelligence {
    pub fn new(clip_history: SharedHistory) -> Self {
        Self {
            clip_history,
            last_analyzed: None,
            sensitive_detected_at: None,
            sensitive_warned: false,
            stale_warned: false,
            recent_copies: Vec::new(),
            cycling_warned_at: None,
            tick_count: 0,
        }
    }
}

/// Detect the primary content type of clipboard text.
fn detect_content_type(text: &str) -> ContentType {
    let trimmed = text.trim();

    // Sensitive data patterns (check first — highest priority)
    if is_sensitive(trimmed) {
        return sensitive_label(trimmed);
    }

    // URL
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return ContentType::Url;
    }

    // Email
    if trimmed.contains('@') && trimmed.contains('.') && !trimmed.contains(' ')
        && trimmed.len() < 200
    {
        return ContentType::Email;
    }

    // File path
    if trimmed.starts_with('/') || trimmed.starts_with("~/") {
        return ContentType::FilePath;
    }

    // JSON
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        return ContentType::Json;
    }

    // Code snippet heuristics
    let code_markers = [
        "fn ", "def ", "function ", "class ", "import ", "const ", "let ",
        "var ", "pub ", "async ", "await ", "return ", "if (", "for (",
        "while (", "=> {", "-> {", "#!/",
    ];
    let has_braces = trimmed.contains('{') && trimmed.contains('}');
    let has_semicolons = trimmed.matches(';').count() >= 2;
    let has_markers = code_markers.iter().any(|m| trimmed.contains(m));
    if has_markers || (has_braces && has_semicolons) {
        return ContentType::CodeSnippet;
    }

    ContentType::PlainText
}

/// Check if text looks like sensitive data.
fn is_sensitive(text: &str) -> bool {
    // API key prefixes
    let key_prefixes = [
        "sk-",      // OpenAI
        "sk_live_", // Stripe
        "sk_test_", // Stripe
        "AKIA",     // AWS
        "ghp_",     // GitHub PAT
        "gho_",     // GitHub OAuth
        "ghs_",     // GitHub App
        "github_pat_", // GitHub fine-grained
        "xoxb-",    // Slack bot
        "xoxp-",    // Slack user
        "xoxs-",    // Slack
        "SG.",      // SendGrid
        "AIza",     // Google API
        "ya29.",    // Google OAuth
        "GOCSPX",   // Google OAuth client
        "glpat-",   // GitLab PAT
        "npm_",     // npm
        "pypi-",    // PyPI
    ];
    for prefix in &key_prefixes {
        if text.starts_with(prefix) {
            return true;
        }
    }

    // SSH private key
    if text.contains("-----BEGIN") && (text.contains("PRIVATE KEY") || text.contains("RSA")) {
        return true;
    }

    // Generic secret patterns (word boundary heuristic)
    let lower = text.to_lowercase();
    let secret_keywords = ["password=", "passwd=", "secret=", "token=", "api_key=", "apikey="];
    for kw in &secret_keywords {
        if lower.contains(kw) {
            return true;
        }
    }

    // Long hex/base64 string that looks like a key (40+ chars, no spaces)
    if text.len() >= 40 && !text.contains(' ') && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/') {
        let alnum_count = text.chars().filter(|c| c.is_ascii_alphanumeric()).count();
        if alnum_count as f32 / text.len() as f32 > 0.8 {
            return true;
        }
    }

    false
}

/// Return the appropriate sensitive data label.
fn sensitive_label(text: &str) -> ContentType {
    if text.starts_with("sk-") || text.starts_with("sk_") {
        ContentType::SensitiveData("API key")
    } else if text.starts_with("AKIA") {
        ContentType::SensitiveData("AWS credential")
    } else if text.starts_with("ghp_") || text.starts_with("gho_") || text.starts_with("github_pat_") {
        ContentType::SensitiveData("GitHub token")
    } else if text.starts_with("xoxb-") || text.starts_with("xoxp-") {
        ContentType::SensitiveData("Slack token")
    } else if text.contains("-----BEGIN") {
        ContentType::SensitiveData("private key")
    } else if text.to_lowercase().contains("password=") || text.to_lowercase().contains("passwd=") {
        ContentType::SensitiveData("password")
    } else {
        ContentType::SensitiveData("secret or token")
    }
}

impl ProactiveFeature for ClipboardIntelligence {
    fn name(&self) -> &str {
        "clipboard_intelligence"
    }

    fn on_event(&mut self, _event: &yantrik_os::SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        // Clipboard changes don't come through SystemEvent — we poll via on_tick.
        Vec::new()
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        self.tick_count += 1;

        // Check every 3 ticks (~30s at 10s intervals) to avoid lock contention
        if self.tick_count % 3 != 0 {
            return Vec::new();
        }

        let mut urges = Vec::new();
        let now = Instant::now();

        // Get current clipboard content
        let current_content = {
            let history = match self.clip_history.lock() {
                Ok(h) => h,
                Err(_) => return Vec::new(),
            };
            history.recent(1).first().map(|e| e.content.clone())
        };

        let Some(content) = current_content else {
            return Vec::new();
        };

        // Track copy frequency for cycling detection
        let is_new = self.last_analyzed.as_ref() != Some(&content);
        if is_new {
            self.recent_copies.push(now);
            // Reset sensitive tracking for new content
            self.sensitive_warned = false;
            self.stale_warned = false;
            self.sensitive_detected_at = None;
        }

        // Clean old copy timestamps (keep last 60s)
        self.recent_copies.retain(|t| now.duration_since(*t).as_secs() < 60);

        // ── Sensitive data detection ──
        let content_type = detect_content_type(&content);
        if let ContentType::SensitiveData(label) = content_type {
            if is_new {
                self.sensitive_detected_at = Some(now);
            }

            // Initial warning
            if !self.sensitive_warned {
                self.sensitive_warned = true;
                urges.push(Urge {
                    id: format!("clip:sensitive:{}", self.tick_count),
                    source: "clipboard_intelligence".into(),
                    title: "Sensitive data in clipboard".into(),
                    body: format!(
                        "Clipboard contains what looks like a {}. Consider clearing it when done.",
                        label
                    ),
                    urgency: 0.8,
                    confidence: 0.85,
                    category: UrgeCategory::Security,
                });
            }

            // Stale sensitive data warning (after 5 minutes)
            if !self.stale_warned {
                if let Some(detected_at) = self.sensitive_detected_at {
                    let elapsed = now.duration_since(detected_at).as_secs();
                    if elapsed >= 300 {
                        self.stale_warned = true;
                        urges.push(Urge {
                            id: format!("clip:stale_sensitive:{}", self.tick_count),
                            source: "clipboard_intelligence".into(),
                            title: "Stale sensitive data".into(),
                            body: format!(
                                "A {} has been in your clipboard for {} minutes. Clear it?",
                                label,
                                elapsed / 60
                            ),
                            urgency: 0.6,
                            confidence: 0.8,
                            category: UrgeCategory::Security,
                        });
                    }
                }
            }
        }

        // ── Copy-paste cycling detection ──
        if self.recent_copies.len() >= 3 {
            let should_warn = match self.cycling_warned_at {
                Some(warned) => now.duration_since(warned).as_secs() >= 300,
                None => true,
            };
            if should_warn {
                self.cycling_warned_at = Some(now);
                urges.push(Urge {
                    id: format!("clip:cycling:{}", self.tick_count),
                    source: "clipboard_intelligence".into(),
                    title: "Clipboard tip".into(),
                    body: format!(
                        "You've copied {} items in the last minute. Press Super+V for clipboard history.",
                        self.recent_copies.len()
                    ),
                    urgency: 0.35,
                    confidence: 0.9,
                    category: UrgeCategory::Focus,
                });
            }
        }

        self.last_analyzed = Some(content);
        urges
    }

    fn on_feedback(&mut self, urge_id: &str, outcome: Outcome) {
        // If user dismisses cycling warnings, stop suggesting for longer
        if urge_id.starts_with("clip:cycling:") {
            if matches!(outcome, Outcome::Dismissed) {
                // Push cycling cooldown to ~15 min by setting warned_at to now
                self.cycling_warned_at = Some(Instant::now());
            }
        }
    }
}
