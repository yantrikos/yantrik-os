//! No path writes a key into `providers.yaml`, and no reader takes one from the entry directly.

use super::*;

// ── No path writes a key into the file ──

/// Source files under `src`, with their text, test files left out.
pub(super) fn sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path.to_string_lossy().replace('\\', "/");
                if !name.ends_with("tests.rs") && !name.contains("/tests/") && !name.contains("/bin/") {
                    out.push((name, std::fs::read_to_string(&path).unwrap()));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

/// The code before a file's first test module.
pub(super) fn code_of(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or(text)
}

#[test]
fn only_the_installer_ever_writes_a_key_into_providers_yaml() {
    // The one field that reaches the file as `api_key:` is `unmigrated_key`. Setting it to a
    // value is for the installer, which has no vault in the new home to put the key in. Anywhere
    // else would be a path that writes a key into the file.
    let mut setters = Vec::new();
    for (name, text) in sources() {
        let code = code_of(&text);
        for line in code.lines() {
            let l = line.trim_start();
            if l.starts_with("//") {
                continue;
            }
            if l.contains("unmigrated_key: Some(")
                || l.contains("unmigrated_key = Some(")
                || (l.contains("unmigrated_key:") && l.contains(".then("))
            {
                setters.push(name.clone());
            }
        }
    }
    setters.sort();
    setters.dedup();
    assert_eq!(setters.len(), 1, "{setters:?}");
    assert!(setters[0].ends_with("wire/installer.rs"), "{setters:?}");

    // And nothing spells `api_key:` into a yaml string by hand.
    for (name, text) in sources() {
        for line in code_of(&text).lines() {
            let l = line.trim_start();
            if l.starts_with("//") || name.ends_with("wire/settings.rs") && l.contains("rename") {
                continue;
            }
            assert!(!(l.contains("\"api_key: ") || l.contains("\\n  api_key:") || l.contains("format!(\"api_key:")), "{name}: {l}");
        }
    }
}

#[test]
fn the_entry_serialises_without_its_key_whatever_it_holds() {
    let mut e = entry("a", "A");
    e.api_key = Some(KEY.into());
    e.api_key_ref = Some(reference("a"));
    let text = serde_yaml::to_string(&e).unwrap();
    assert!(!text.contains(KEY) && !text.contains("api_key:"), "{text}");
    e.api_key = None;
    e.unmigrated_key = Some(OTHER.into());
    let text = serde_yaml::to_string(&e).unwrap();
    assert!(text.contains("api_key: "), "an older file's key is written back until it is moved:\n{text}");
}

// ── The call sites ──

#[test]
fn no_reader_takes_a_saved_providers_key_from_the_entry_directly() {
    // A reader that used `entry.api_key` would send nothing, or the wrong thing, once the key is
    // in the vault. Every reader goes through `resolve_key`, `with_resolved_key` or `key_source`.
    // `api_key` itself is touched only by the module that owns the rules, the form that fills it
    // in, the hand-off adapters (given a copy that holds the key) and the Debug that hides it.
    let allowed_file = |name: &str| name.ends_with("/provider_vault.rs") || name.ends_with("/provider_handoff/deepseek.rs");
    let mut found = Vec::new();
    for (name, text) in sources() {
        if allowed_file(&name) {
            continue;
        }
        for line in code_of(&text).lines() {
            let l = line.trim_start();
            // `.api_key` as a whole word: `.api_key_ref` is the reference, and is fine.
            let whole = l.match_indices(".api_key").any(|(i, m)| {
                !l[i + m.len()..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_')
            });
            if l.starts_with("//") || !whole {
                continue;
            }
            let other_struct = l.contains("llm.api_key") || l.contains("config.api_key");
            let form_fill = name.ends_with("/wire/provider_panel.rs") && (l.contains("form.api_key") || l.contains("e.api_key = "));
            let hidden = name.ends_with("/wire/settings.rs") && l.contains("self.api_key");
            if !(other_struct || form_fill || hidden) {
                found.push(format!("{name}: {l}"));
            }
        }
    }
    assert!(found.is_empty(), "read through resolve_key() instead:\n{}", found.join("\n"));
}
