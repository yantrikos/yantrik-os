//! The vendors a person can bring an account from, and what the machine knows about each one
//! without asking it anything.
//!
//! One row per vendor, and every field is a constant: which program is theirs, where it keeps its
//! sign-in, how a second account is kept apart from the first, and the command that signs in —
//! the vendor's own, run in a terminal the person watches. Nothing a person types reaches a
//! command line from this table.

/// How a vendor's account is signed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignIn {
    /// The vendor's program signs in itself (a browser or a device code), and keeps what it gets
    /// in its own directory: `login` is the command, run in a terminal.
    Program { login: &'static str },
    /// An API key, set up where the desktop keeps its AI providers (Settings → AI). The vendor is
    /// here when the desktop's provider is one of `endpoints`.
    Key { endpoints: &'static [&'static str] },
}

/// One vendor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vendor {
    /// Stable: the first half of every account id (`claude:primary`).
    pub id: &'static str,
    pub name: &'static str,
    /// Which icon the panel draws.
    pub glyph: &'static str,
    /// The program, looked for on `PATH`. Empty for a key-only vendor.
    pub binary: &'static str,
    /// What the program is called, for "Chosen for Claude Code". Empty for a key-only vendor.
    pub program: &'static str,
    /// Its directory under the home directory, for the first account.
    pub home: &'static str,
    /// The environment variable that points the program at another directory, which is how a
    /// second account is kept apart from the first. `None`: the program has one, so one account.
    pub home_env: Option<&'static str>,
    /// A file in that directory that exists once an account is signed in. Only ever looked for,
    /// never opened — except where a vendor module says what it reads and why.
    pub marker: &'static str,
    pub sign_in: SignIn,
    /// How to get the program, for the row that says it is not installed. Run in a terminal the
    /// person watches, only when they press Install.
    pub install: &'static str,
}

impl Vendor {
    /// Whether this vendor can hold more than one account on the machine.
    pub fn many(&self) -> bool {
        self.home_env.is_some()
    }
}

/// Every vendor, in the order the panel lists them.
pub const VENDORS: [Vendor; 5] = [
    Vendor {
        id: "claude",
        name: "Claude",
        glyph: "claude",
        binary: "claude",
        program: "Claude Code",
        home: ".claude",
        home_env: Some("CLAUDE_CONFIG_DIR"),
        marker: ".credentials.json",
        sign_in: SignIn::Program { login: "claude" },
        install: "npm install -g @anthropic-ai/claude-code",
    },
    Vendor {
        id: "codex",
        name: "Codex",
        glyph: "codex",
        binary: "codex",
        program: "Codex",
        home: ".codex",
        home_env: Some("CODEX_HOME"),
        marker: "auth.json",
        sign_in: SignIn::Program { login: "codex login --device-auth" },
        install: "npm install -g @openai/codex",
    },
    // Qwen's own sign-in (Qwen Code's OAuth) ended on 15 April 2026; what is left is Alibaba's
    // Coding Plan and Token Plan, which are keys.
    Vendor {
        id: "qwen",
        name: "Qwen",
        glyph: "qwen",
        binary: "",
        program: "",
        home: "",
        home_env: None,
        marker: "",
        sign_in: SignIn::Key { endpoints: &["dashscope.aliyuncs.com", "dashscope-intl.aliyuncs.com", "maas.aliyuncs.com"] },
        install: "",
    },
    Vendor {
        id: "gemini",
        name: "Gemini",
        glyph: "gemini",
        binary: "gemini",
        program: "Gemini CLI",
        home: ".gemini",
        home_env: None,
        marker: "oauth_creds.json",
        sign_in: SignIn::Program { login: "gemini" },
        install: "npm install -g @google/gemini-cli",
    },
    Vendor {
        id: "xai",
        name: "xAI",
        glyph: "xai",
        binary: "",
        program: "",
        home: "",
        home_env: None,
        marker: "",
        sign_in: SignIn::Key { endpoints: &["api.x.ai"] },
        install: "",
    },
];

impl Vendor {
    /// For a key vendor: whether the desktop's AI endpoint is this vendor's.
    pub fn serves(&self, url: Option<&str>) -> bool {
        let SignIn::Key { endpoints } = self.sign_in else { return false };
        let Some(host) = url.and_then(host_of) else { return false };
        endpoints.iter().any(|e| host == *e || host.ends_with(&format!(".{e}")))
    }
}

/// The host of an http(s) URL, lowercased.
fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    // `\` too: a browser and the `url` crate end the authority there, so `evil.example\@api.x.ai`
    // is evil.example, not xAI.
    let authority = rest.split(['/', '\\', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The vendor with this id.
pub fn by_id(id: &str) -> Option<&'static Vendor> {
    VENDORS.iter().find(|v| v.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_every_program_vendor_can_be_signed_in_and_installed() {
        let mut ids: Vec<_> = VENDORS.iter().map(|v| v.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), VENDORS.len());
        for v in VENDORS {
            match v.sign_in {
                SignIn::Program { login } => {
                    assert!(!v.binary.is_empty() && !v.home.is_empty() && !v.marker.is_empty(), "{}", v.id);
                    assert!(login == v.binary || login.starts_with(&format!("{} ", v.binary)), "{}: signs in with its own program", v.id);
                    assert!(v.install.starts_with("npm install -g @"), "{}", v.id);
                }
                SignIn::Key { endpoints } => {
                    assert!(v.binary.is_empty() && v.home_env.is_none() && !endpoints.is_empty(), "{}", v.id)
                }
            }
            // An account id is `vendor:label`, and the label is never allowed a colon.
            assert!(!v.id.contains(':'));
        }
    }

    #[test]
    fn a_key_vendor_is_known_by_its_host_and_not_by_a_lookalike() {
        let xai = by_id("xai").unwrap();
        assert!(xai.serves(Some("https://api.x.ai/v1")));
        assert!(!xai.serves(Some("https://api.x.ai.evil.example/v1")));
        assert!(!xai.serves(Some("https://evil.example/api.x.ai")));
        assert!(!xai.serves(Some("https://evil.example\\@api.x.ai/v1")));
        assert!(!xai.serves(None));
        let qwen = by_id("qwen").unwrap();
        assert!(qwen.serves(Some("https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1")));
        assert!(qwen.serves(Some("https://coding-intl.dashscope.aliyuncs.com/v1")));
        assert!(qwen.serves(Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1")));
        assert!(!by_id("claude").unwrap().serves(Some("https://api.x.ai/v1")));
    }

    /// Every command here is a constant the shell hands to `sh -lc`: none may carry anything a
    /// shell would read as more than words.
    #[test]
    fn no_command_carries_shell_syntax() {
        for v in VENDORS {
            let login = match v.sign_in {
                SignIn::Program { login } => login,
                SignIn::Key { .. } => "",
            };
            for cmd in [login, v.install] {
                assert!(
                    cmd.chars().all(|c| c.is_ascii_alphanumeric() || " -@/._".contains(c)),
                    "{}: {cmd}",
                    v.id
                );
            }
        }
    }
}
