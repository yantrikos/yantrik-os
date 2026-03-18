//! Permission levels for tool access control.

/// Risk level for a tool. Ordered so a single comparison gates access:
/// `tool.permission() > ctx.max_permission` → deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionLevel {
    /// Read-only, no state change. Always allowed.
    Safe,
    /// Writes data but reversible (write file, clipboard, remember).
    Standard,
    /// System state changes (kill process, volume, resolution).
    Sensitive,
    /// Destructive/irreversible (delete files, shutdown).
    Dangerous,
}

impl std::fmt::Display for PermissionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Safe => write!(f, "safe"),
            Self::Standard => write!(f, "standard"),
            Self::Sensitive => write!(f, "sensitive"),
            Self::Dangerous => write!(f, "dangerous"),
        }
    }
}

/// Parse a permission level from config string. Defaults to Sensitive.
pub fn parse_permission(s: &str) -> PermissionLevel {
    match s.to_lowercase().as_str() {
        "safe" => PermissionLevel::Safe,
        "standard" => PermissionLevel::Standard,
        "sensitive" => PermissionLevel::Sensitive,
        "dangerous" => PermissionLevel::Dangerous,
        _ => PermissionLevel::Sensitive,
    }
}
