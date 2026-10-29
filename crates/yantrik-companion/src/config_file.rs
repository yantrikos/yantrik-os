//! Writing one section of the machine's `config.yaml`.
//!
//! The file is the machine's own, with its comments and its defaults, and other things read it.
//! The companion's in-memory configuration is not that file: a provider chosen in Settings puts
//! its URL, model and key into the in-memory `llm` section, and none of that may reach the file.
//! So a change to one section is written by reading the file as it is on disk and replacing
//! that section's lines alone. The in-memory configuration is never serialised to disk.

use std::io::Write;
use std::path::{Path, PathBuf};

use yantrik_companion_core::judge_config::JudgeConfig;

use crate::CompanionConfig;

/// Where the machine's config is: the file the shell was started with, else the installed one.
pub fn machine_config_path(loaded: Option<&Path>) -> PathBuf {
    loaded.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/opt/yantrik/config.yaml"))
}

/// Replace the top-level `judge:` section of the YAML file at `path` with `judge`, leaving every
/// other line as it is. The new text is parsed as a `CompanionConfig` first and must hold exactly
/// `judge`; otherwise nothing is written and the error says why.
///
/// The write is in place, not through a temporary file: on an installed machine `/opt/yantrik`
/// belongs to root and only `config.yaml` is the person's, so no file can be made beside it. In
/// place keeps the file's owner and mode, and a symlink is refused rather than followed.
pub fn write_judge_section(path: &Path, judge: &JudgeConfig) -> std::io::Result<()> {
    let current = std::fs::read_to_string(path)?;
    let section = serde_yaml::to_string(&std::collections::BTreeMap::from([("judge", judge)]))
        .map_err(invalid)?;
    let text = replace_section(&current, "judge", &section);
    let parsed: CompanionConfig = serde_yaml::from_str(&text)
        .map_err(|e| invalid(format!("the config would no longer load ({e})")))?;
    if parsed.judge != *judge {
        return Err(invalid("the decision model could not be placed in the config without changing something else"));
    }
    write_in_place(path, text.as_bytes())
}

fn invalid(why: impl ToString) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string())
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

fn write_in_place(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
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
    fn the_files_mode_and_inode_are_kept() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let d = dir("mode");
        let path = d.join("config.yaml");
        std::fs::write(&path, DISK).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        write_judge_section(&path, &CompanionConfig::default().judge).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(meta.ino(), inode);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    /// An installed machine: /opt/yantrik is not the writer's to write in, only config.yaml is.
    #[cfg(unix)]
    #[test]
    fn the_save_works_when_the_directory_cannot_be_written() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("readonly");
        let path = d.join("config.yaml");
        std::fs::write(&path, DISK).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        let mut judge = reloaded().judge;
        judge.provider = "jev".into();
        judge.endpoint = "https://api.typesafe.ai".into();
        let result = write_judge_section(&path, &judge);
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        result.unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("provider: jev"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_is_refused_and_its_target_left_alone() {
        let d = dir("link");
        let target = d.join("real.yaml");
        std::fs::write(&target, DISK).unwrap();
        let link = d.join("config.yaml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_judge_section(&link, &CompanionConfig::default().judge).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), DISK);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_file_the_section_cannot_be_placed_in_is_refused_and_left_as_it_was() {
        let mut judge = CompanionConfig::default().judge;
        judge.provider = "jev".into();
        judge.endpoint = "https://api.typesafe.ai".into();
        let cases = [
            ("anchor and alias", "judge: &j\n  provider: kev\nother: *j\n"),
            ("multi-line flow map", "judge: {\n  provider: kev\n}\ntools:\n  enabled: true\n"),
            ("byte order mark", "\u{feff}judge:\n  provider: kev\n"),
            ("quoted key", "\"judge\":\n  provider: kev\n"),
        ];
        for (name, text) in cases {
            let d = dir("refuse");
            let path = d.join("config.yaml");
            std::fs::write(&path, text).unwrap();
            assert!(write_judge_section(&path, &judge).is_err(), "{name} was written");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{name} changed the file");
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn the_loaded_path_is_the_one_saved_to() {
        assert_eq!(machine_config_path(Some(Path::new("/x/c.yaml"))), PathBuf::from("/x/c.yaml"));
        assert_eq!(machine_config_path(None), PathBuf::from("/opt/yantrik/config.yaml"));
    }
}
