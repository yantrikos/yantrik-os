//! What an install carries from the live session into the new account, and what the copy of the
//! live system leaves behind (#1165; #1168, finding 7).

/// The first run's choices, carried from the live session to the new home: the sharing choice
/// (ai-accounts.json), the text size (first-run.json) and the one model pick for every mind
/// (picker.json). Choices only: no key in any of them. The free AI keys are not carried by name:
/// they cross inside the copied system, in the vault under `/opt/yantrik/data`, which
/// `COPY_EXCLUDES` leaves in, and under LUKS when the disk is encrypted.
pub(crate) const CARRIED_CHOICES: [&str; 3] = ["ai-accounts.json", "first-run.json", "picker.json"];

/// What rsync leaves out of the copy of `/`: the kernel's and the live medium's own trees, and
/// the EFI partition, which is shared with macOS or Windows and which only installer_boot.rs
/// writes to (GRUB and the \EFI\BOOT set). Anchored at `/`, as rsync reads them.
pub(crate) const COPY_EXCLUDES: [&str; 9] =
    ["/proc/*", "/sys/*", "/dev/*", "/run/*", "/tmp/*", "/mnt/*", "/live/*", "/cdrom/*", "/boot/efi/*"];

/// Whether a choices file is free of anything key-like: a field named like a secret with a value,
/// or a value shaped like a provider key. A file that is not JSON is not carried either.
pub(crate) fn holds_no_key(text: &str) -> bool {
    fn clean(v: &serde_json::Value) -> bool {
        match v {
            serde_json::Value::Object(map) => map.iter().all(|(k, v)| {
                let k = k.to_ascii_lowercase();
                let secret_name = ["key", "token", "secret", "password", "authorization"]
                    .iter()
                    .any(|w| k == *w || k.ends_with(&format!("_{w}")) || k.ends_with(&format!("-{w}")) || k == "apikey");
                let empty = v.is_null() || v.as_str().is_some_and(|s| s.is_empty());
                (!secret_name || empty) && clean(v)
            }),
            serde_json::Value::Array(a) => a.iter().all(clean),
            serde_json::Value::String(s) => {
                let s = s.trim();
                !["sk-", "nvapi-", "gsk_", "hf_", "AIza", "Bearer "].iter().any(|p| s.starts_with(p))
            }
            _ => true,
        }
    }
    serde_json::from_str::<serde_json::Value>(text).is_ok_and(|v| clean(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vault: the companion's database, where every `os:provider-key:*` row is.
    const VAULT_DB: &str = "/opt/yantrik/data/memory.db";

    /// rsync's reading of an anchored `<dir>/*` pattern: everything under `<dir>`.
    fn excluded(path: &str) -> bool {
        COPY_EXCLUDES.iter().any(|e| path.starts_with(e.trim_end_matches('*')))
    }

    /// Decision on #1168, finding 7: the free AI keys cross the install in the copied database and
    /// nowhere else, so a later "fresh data" change cannot drop them silently.
    #[test]
    fn the_vault_crosses_the_install_in_the_copied_system() {
        assert!(!excluded(VAULT_DB));
        assert!(!excluded("/opt/yantrik/data"));
        assert!(excluded("/proc/1/status") && excluded("/boot/efi/EFI/BOOT/BOOTX64.EFI") && excluded("/live/medium"));
        assert!(!excluded("/boot/grub/grub.cfg"), "only the EFI partition under /boot is left out");
        for e in COPY_EXCLUDES {
            assert!(e.starts_with('/') && e.ends_with("/*"), "{e}: anchored, and a tree");
        }
    }

    #[test]
    fn the_install_carries_the_pick_with_the_other_choices_and_no_key() {
        assert!(CARRIED_CHOICES.contains(&"picker.json"), "the person's model pick reaches the new home");
        assert!(CARRIED_CHOICES.contains(&"ai-accounts.json") && CARRIED_CHOICES.contains(&"first-run.json"));
        // No vault row and no key file is carried by name (#1168, finding 7): choices only.
        for f in CARRIED_CHOICES {
            assert!(f.ends_with(".json") && !f.contains("memory") && !f.contains("vault") && !f.contains("provider"), "{f}");
        }
        // Each file as its own module writes it, with a choice in it: no key.
        let mut picks = crate::picker::choices::Choices::default();
        picks.pick_model("", "free-groq/openai/gpt-oss-120b");
        let picker = serde_json::to_string(&picks).unwrap();
        assert!(picker.contains("free-groq/openai/gpt-oss-120b"));
        let first_run = serde_json::to_string(&crate::wire::onboarding::Saved { text_scale: 1.25, sharing: "share".into(), provider_chosen: true }).unwrap();
        let mut consent = crate::ai_accounts::consent::Consent::default();
        consent.lane = Some("on-this-computer/qwen".into());
        let accounts = serde_json::to_string(&consent).unwrap();
        for text in [&picker, &first_run, &accounts] {
            assert!(holds_no_key(text), "{text}");
        }
        // And one that somehow held a key is not carried.
        assert!(!holds_no_key(r#"{"model":"a/b","api_key":"x"}"#));
        assert!(!holds_no_key(r#"{"minds":{"m":{"model":"sk-ant-api03-abc"}}}"#));
        assert!(!holds_no_key(r#"{"model":"gsk_abcdefghijklmnopqrstuvwxyz0123456789abcdefghij"}"#), "a free key is a key");
        assert!(!holds_no_key("not json"));
        assert!(holds_no_key(r#"{"api_key":""}"#), "an empty field holds nothing");
    }
}
