//! Hermes (harnesses/hermes): its model is Hermes's own setting, in ~/.hermes/config.yaml under
//! `model` — `default`, `provider`, `base_url`, `api_key` for a custom OpenAI-compatible endpoint,
//! which is what `hermes model` writes for one. Using Yantrik models sets those four, with
//! Hermes's own gateway token as the key, and leaves the rest of the file's settings as they were.
//! Its comments are not kept in the new file (YAML read and written again); Revert puts the
//! person's own file back, comments and all, and the card says so.
//!
//! **Moved to the mind account (#411).** Hermes's file is then in that account's home, which the
//! person's desktop cannot write (that is the point of the move), so nothing is written here:
//! the settings go to `yantrik-update harness model hermes`, on its stdin, and the account's own
//! tool writes the four `model` keys inside the gateway's sandbox ([`post_model`]). Revert asks it
//! to put the earlier `model` section back. The desktop cannot read that file either, so it
//! reports the model it asked for.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_yaml::{Mapping, Value};

use super::{Handoff, Offer, Plan, Token, Write};
use crate::wire::settings::ProviderStoreEntry;

pub(crate) struct Hermes;

pub(crate) const ID: &str = "hermes";
/// Root's record that Hermes has moved to the mind account (harness-isolation.sh, `hi_migrated`).
const MOVED_MARK: &str = "/var/lib/yantrik-harness/hermes.isolated";
/// The updater, whose sudo rule lets the person ask for the one thing below.
const UPDATER: &str = "/opt/yantrik/bin/yantrik-update";

/// Has Hermes been moved to the mind account here? A regular file, not a link.
pub(crate) fn moved() -> bool {
    std::fs::symlink_metadata(MOVED_MARK).is_ok_and(|m| m.file_type().is_file())
}

/// Is `path` the config Hermes had in the person's home (the caller knows it has moved)? A marker written by
/// "Use Yantrik models" before the move names it.
pub(crate) fn was_before_the_move(home: &Path, path: &Path) -> bool {
    path == config(home)
}

/// The settings the account's tool writes: what `hermes model` writes for a custom endpoint.
fn model_body(offer: &Offer) -> String {
    serde_json::json!({
        "default": yantrik_gateway::PICKED,
        "provider": "custom",
        "base_url": offer.base_url(),
        "api_key": offer.token.0,
    })
    .to_string()
}

/// The sentences the tool asked to be shown, from its relayed output. The updater's relay (`hi_relay` →
/// `info`) indents every line by two spaces, so a `NOTE: ` line arrives as `  NOTE: …`.
fn notes(stdout: &str) -> Option<String> {
    let notes: Vec<&str> = stdout.lines().filter_map(|l| l.trim_start().strip_prefix("NOTE: ")).collect();
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// `yantrik-update harness model hermes apply|revert`, with the settings (apply) on stdin, never in
/// an argument list. `Ok` when the account's tool changed the file, with what it asked to be told to
/// the person (its `NOTE: ` lines: where it kept a copy of their own model settings); otherwise its
/// own sentence, which never holds the settings.
pub(crate) fn post_model(op: &str, body: Option<&str>) -> Result<Option<String>, String> {
    let mut child = Command::new("sudo")
        .args(["-n", UPDATER, "harness", "model", "hermes", op])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not ask the updater to change Hermes's model: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A tool that stops reading early closes the pipe; its exit status says what happened.
        let _ = stdin.write_all(body.unwrap_or_default().as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        tracing::info!(op, "changed the model of the moved Hermes");
        return Ok(notes(&String::from_utf8_lossy(&out.stdout)));
    }
    let said = String::from_utf8_lossy(&out.stderr);
    let line = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    Err(format!(
        "Hermes runs under its own account here, and changing its model did not work: {}. Run once in a terminal: sudo yantrik-update harness model hermes {op}",
        if line.is_empty() { "no reason given" } else { line }
    ))
}

/// Hermes's own gateway service, which loads the desktop plugin (harnesses/lib/install/hermes.sh).
const UNIT: &str = "hermes-gateway.service";

fn config(home: &Path) -> PathBuf {
    home.join(".hermes/config.yaml")
}

impl Handoff for Hermes {
    fn harness(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Hermes"
    }

    fn files(&self, home: &Path) -> Vec<PathBuf> {
        if moved() {
            return Vec::new();
        }
        vec![config(home)]
    }

    fn unit(&self) -> Option<&'static str> {
        Some(UNIT)
    }

    fn plan(&self, _home: &Path, _provider: &ProviderStoreEntry) -> Result<Plan, String> {
        Err("Hermes chooses its model itself (Choose model); give it Yantrik models instead".into())
    }

    /// `model.default`, while `model.base_url` is the gateway's.
    fn asks_gateway_for(&self, home: &Path) -> Option<String> {
        if moved() {
            return Some(yantrik_gateway::PICKED.into());
        }
        let doc: Value = serde_yaml::from_str(&super::read_config(&config(home)).ok()??).ok()?;
        let model = doc.get("model")?;
        let s = |k: &str| model.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        super::is_gateway(&s("base_url")?).then(|| s("default")).flatten()
    }

    fn plan_gateway(&self, home: &Path, offer: &Offer) -> Result<Plan, String> {
        plan_gateway_as(self, home, offer, moved())
    }
}

/// `moved`: whether Hermes is the mind account's. A plan for that has no file to write.
pub(super) fn plan_gateway_as(adapter: &Hermes, home: &Path, offer: &Offer, moved: bool) -> Result<Plan, String> {
    if moved {
        let mut plan = super::gateway_plan(adapter, offer, Vec::new());
        plan.moved_post = Some(Token(model_body(offer)));
        return Ok(plan);
    }
    plan_file(adapter, home, offer)
}

fn plan_file(adapter: &Hermes, home: &Path, offer: &Offer) -> Result<Plan, String> {
    let path = config(home);
    let mut doc: Mapping = match super::read_config(&path)? {
        Some(text) if !text.trim().is_empty() => match serde_yaml::from_str::<Value>(&text) {
            Ok(Value::Mapping(m)) => m,
            _ => return Err(format!("{} is not a YAML mapping, so it is left alone", path.display())),
        },
        _ => Mapping::new(),
    };
    let model = doc.entry(Value::from("model")).or_insert_with(|| Value::Mapping(Mapping::new()));
    // An older config names the model as a bare string; it becomes the mapping Hermes reads now.
    if !model.is_mapping() {
        *model = Value::Mapping(Mapping::new());
    }
    let m = model.as_mapping_mut().expect("made a mapping above");
    m.insert("default".into(), yantrik_gateway::PICKED.into());
    m.insert("provider".into(), "custom".into());
    m.insert("base_url".into(), offer.base_url().into());
    m.insert("api_key".into(), offer.token.0.clone().into());
    let content = serde_yaml::to_string(&Value::Mapping(doc)).map_err(|e| e.to_string())?;
    Ok(super::gateway_plan(
        adapter,
        offer,
        vec![Write {
            path,
            content,
            what: "its model, the gateway's address and its gateway token (the file's comments are not kept; Revert puts yours back)".into(),
        }],
    ))
}

#[cfg(test)]
mod notes_tests {
    use super::notes;

    #[test]
    fn relayed_note_lines_are_found_through_their_indent() {
        // Exactly what `hi_relay` + `info` print: `printf '  %s\n'` on every line.
        let relayed = "  hermes's model settings changed; Hermes finishes its turns and restarts\n  \
                       NOTE: Your own model settings that this replaced are kept at /x/config.yaml.model-replaced-1.\n  \
                       NOTE: Put back an older model section that was already kept beside it.\n  \
                       the gateway reloaded\n";
        assert_eq!(
            notes(relayed).as_deref(),
            Some("Your own model settings that this replaced are kept at /x/config.yaml.model-replaced-1. Put back an older model section that was already kept beside it.")
        );
    }

    #[test]
    fn unindented_note_lines_and_no_notes() {
        assert_eq!(notes("NOTE: bare.\n").as_deref(), Some("bare."));
        assert_eq!(notes("  settings changed\n  a NOTE: in the middle\n"), None);
        assert_eq!(notes(""), None);
    }
}
