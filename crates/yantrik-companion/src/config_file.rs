//! Writing one section of the machine's `config.yaml`.
//!
//! The file is the machine's own, with its comments and its defaults, and other things read it.
//! The companion's in-memory configuration is not that file: a provider chosen in Settings puts
//! its URL, model and key into the in-memory `llm` section, and none of that may reach the file.
//! So a change to one section is written by reading the file as it is on disk and replacing
//! that section's lines alone. The in-memory configuration is never serialised to disk.

use std::io::Write;
use std::path::Path;

use yantrik_companion_core::judge_config::JudgeConfig;

/// Replace the top-level `judge:` section of the YAML file at `path` with `judge`, leaving every
/// other line as it is. The write goes to a temporary file beside it and is renamed over it,
/// with the file's mode kept.
pub fn write_judge_section(path: &Path, judge: &JudgeConfig) -> std::io::Result<()> {
    let current = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let section = serde_yaml::to_string(&std::collections::BTreeMap::from([("judge", judge)]))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomically(path, replace_section(&current, "judge", &section).as_bytes())
}

/// `text` with the top-level `key:` section replaced by `section` (which ends in a newline), or
/// appended when `text` has none. A section runs from its key line to the next line that starts
/// in column 0 with something other than a comment; the comment and blank lines just before that
/// next key belong to it and are kept.
fn replace_section(text: &str, key: &str, section: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let is_key = |l: &str| l.strip_prefix(key).is_some_and(|r| r.trim_start().starts_with(':'));
    let Some(start) = lines.iter().position(|l| is_key(l)) else {
        let mut out = text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(section);
        return out;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.chars().next().is_some_and(|c| !c.is_whitespace() && c != '#'))
        .map_or(lines.len(), |i| start + 1 + i);
    let mut keep_from = end;
    while keep_from > start + 1 {
        let l = lines[keep_from - 1];
        if l.trim().is_empty() || l.starts_with('#') {
            keep_from -= 1;
        } else {
            break;
        }
    }
    let mut out: String = lines[..start].concat();
    out.push_str(section);
    out.push_str(&lines[keep_from..].concat());
    out
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompanionConfig;

    const FAKE_KEY: &str = "sk-fake-test-key-0000";
    const FAKE_URL: &str = "https://saved-provider.invalid/v1";

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yos-cfgfile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const DISK: &str = "# the machine's file\nllm:\n  backend: \"api\"\n  api_base_url: \"http://machine.invalid:11434\"\n  api_model: \"machine-model\"\n\n# judge below\njudge:\n  provider: kev\n  endpoint: \"http://127.0.0.1:8009\"\n\n# after judge\ntools:\n  enabled: true\n";

    /// The in-memory configuration after "Use for the companion" picked a saved provider.
    fn reloaded() -> CompanionConfig {
        let mut c = CompanionConfig::default();
        c.llm.backend = "api".into();
        c.llm.api_base_url = Some(FAKE_URL.into());
        c.llm.api_model = Some("saved-model".into());
        c.llm.api_key = Some(FAKE_KEY.into());
        c
    }

    /// What the old whole-config save wrote: the proof that it leaked.
    #[test]
    fn serialising_the_in_memory_config_carries_the_saved_providers_key() {
        let yaml = serde_yaml::to_string(&reloaded()).unwrap();
        assert!(yaml.contains(FAKE_KEY) && yaml.contains(FAKE_URL));
    }

    #[test]
    fn a_decision_model_save_leaves_the_llm_section_and_key_untouched() {
        let d = dir("save");
        let path = d.join("config.yaml");
        std::fs::write(&path, DISK).unwrap();
        let mut config = reloaded();
        config.judge.provider = "jev".into();
        config.judge.endpoint = "https://api.typesafe.ai".into();
        write_judge_section(&path, &config.judge).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains(FAKE_KEY), "a key reached the file");
        assert!(!after.contains(FAKE_URL), "the saved provider's endpoint reached the file");
        let before_judge = &DISK[..DISK.find("judge:").unwrap()];
        assert!(after.starts_with(before_judge), "lines before judge changed");
        assert!(after.ends_with("\n# after judge\ntools:\n  enabled: true\n"), "lines after judge changed");
        assert!(after.contains("provider: jev") && !after.contains("provider: kev"));
        let back: serde_yaml::Value = serde_yaml::from_str(&after).unwrap();
        assert_eq!(back["judge"]["endpoint"], "https://api.typesafe.ai");
        assert_eq!(back["llm"]["api_model"], "machine-model");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_file_without_a_judge_section_gets_one_appended() {
        let d = dir("append");
        let path = d.join("config.yaml");
        std::fs::write(&path, "llm:\n  backend: \"api\"").unwrap();
        write_judge_section(&path, &CompanionConfig::default().judge).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with("llm:\n  backend: \"api\"\njudge:\n"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn the_files_mode_is_kept_and_no_temporary_file_is_left() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("mode");
        let path = d.join("config.yaml");
        std::fs::write(&path, DISK).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_judge_section(&path, &CompanionConfig::default().judge).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(d);
    }
}
