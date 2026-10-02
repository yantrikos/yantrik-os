//! A harness whose memory is the machine's YantrikDB (#447): granting it, and saying so.
//!
//! Installing Hermes from this desktop was meant to give Hermes the memory Yantrik Mind keeps, so
//! the two remember the same person, and not a second memory of them in ~/.hermes. A manifest says
//! `memory: yantrikdb` for a harness built that way, and the person's Install click on its row
//! grants it ordinary recall, remember and believe, saved where the person's grants are saved, so
//! the shell's baseline holds it and a later edit behind the shell's back cannot widen it.
//!
//! Only the click does this. The control surface's `install_harness` runs the same install and
//! grants nothing: a mind able to install another mind must not be able to hand it the person's
//! memory by doing so. Health, finance and household memory are never granted here, because the
//! person grants those separately, and credentials are never a grant at all.
//!
//! Nothing restarts afterwards. The host asks for the grants at every turn a harness takes, so the
//! next turn Hermes takes carries its memory credential.

use crate::harness_catalogue::{Manifest, Memory};
use crate::memory_grants::{self, Grants, Store};

/// What the row and `describe shell` say about a harness that has the machine's memory.
pub const SHARED: &str = "Memory: YantrikDB (shared with Yantrik Mind)";

/// The grants after the person's Install click: ordinary recall, remember and believe on, and
/// whatever else the person chose for this harness left as it was.
pub fn with_install_grant(mut store: Store, id: &str) -> Store {
    let ordinary = Grants::ordinary();
    let grants = store.minds.entry(id.to_string()).or_default();
    grants.recall_ordinary = ordinary.recall_ordinary;
    grants.remember = ordinary.remember;
    grants.believe = ordinary.believe;
    store
}

/// Grant `manifest`'s harness the machine's memory, for the person's own Install click and for
/// nothing else. `Ok(false)` for a harness whose manifest does not ask for it.
pub fn grant_on_persons_install(manifest: &Manifest) -> Result<bool, String> {
    if manifest.memory != Memory::Yantrikdb {
        return Ok(false);
    }
    // These two have grants of their own by being what they are. A manifest that borrows one of
    // their names is not either of them, and writing its grant under that name would change what
    // the real one may do.
    if [memory_grants::FIRST_PARTY_MIND, memory_grants::COMPANION].contains(&manifest.id.as_str()) {
        return Err(format!("`{}` is not an id a harness can be granted memory under", manifest.id));
    }
    memory_grants::update(|store| *store = with_install_grant(std::mem::take(store), &manifest.id))?;
    tracing::info!(harness = %manifest.id, "The person's Install gave this harness ordinary memory grants (#447)");
    Ok(true)
}

/// The harnesses the person's grants give some use of their memory, as third parties: what each
/// row's memory line is decided from. None when the grants file cannot be trusted, which grants
/// nobody anything.
pub fn granted_minds() -> Vec<String> {
    granted_in(memory_grants::load().as_ref())
}

fn granted_in(store: Option<&Store>) -> Vec<String> {
    store
        .map(|store| store.minds.keys().filter(|id| store.grants_for(id, false).any()).cloned().collect())
        .unwrap_or_default()
}

/// The row's line about memory. Empty for a harness that keeps its own. `installed` is whether
/// it is past installing, `granted` whether the person's grants give it anything now.
pub fn line(memory: Memory, installed: bool, granted: bool) -> String {
    match (memory, granted, installed) {
        (Memory::Own, _, _) => String::new(),
        (Memory::Yantrikdb, true, _) => SHARED.to_string(),
        // Said before the click, so the person knows what Install will do with their memory.
        (Memory::Yantrikdb, false, false) => "Memory: Install gives it YantrikDB, shared with Yantrik Mind".to_string(),
        // Installed some other way, or the grant was taken away since.
        (Memory::Yantrikdb, false, true) => {
            "Memory: not granted YantrikDB (the grants are in ~/.config/yantrik/memory-grants.json)".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hermes(memory: Memory) -> Manifest {
        Manifest { id: "hermes".into(), memory, ..Default::default() }
    }

    #[test]
    fn the_install_grant_is_ordinary_memory_and_never_health_finance_or_household() {
        let store = with_install_grant(Store::default(), "hermes");
        assert_eq!(store.grants_for("hermes", false), Grants::ordinary());
        assert_eq!(store.grants_for("hermes", false).names(), ["recall_ordinary", "remember", "believe"]);
        // Nobody else is given anything by it.
        assert_eq!(store.minds.len(), 1);
        assert!(!store.grants_for("pi", false).any());
    }

    #[test]
    fn the_install_grant_takes_nothing_the_person_gave_and_widens_nothing_else() {
        let mut store = Store::default();
        store.minds.insert("hermes".into(), Grants { recall_health: true, ..Grants::default() });
        store.minds.insert(memory_grants::FIRST_PARTY_MIND.into(), Grants::default());
        let after = with_install_grant(store, "hermes");
        assert!(after.grants_for("hermes", false).recall_health, "the person's own choice stays");
        assert!(!after.grants_for("hermes", false).recall_finance && !after.grants_for("hermes", false).household);
        assert!(!after.grants_for(memory_grants::FIRST_PARTY_MIND, true).any(), "a revocation elsewhere stays");
    }

    #[test]
    fn a_harness_that_keeps_its_own_memory_is_granted_nothing_and_the_file_is_untouched() {
        let dir = std::env::temp_dir().join(format!("yantrik-hm-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("memory-grants.json");
        let _using = memory_grants::test_file::using(file.clone());
        assert_eq!(grant_on_persons_install(&hermes(Memory::Own)), Ok(false));
        assert!(!file.exists());
        assert_eq!(grant_on_persons_install(&hermes(Memory::Yantrikdb)), Ok(true));
        assert_eq!(granted_minds(), ["hermes"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_manifest_borrowing_the_first_party_names_is_not_granted_under_them() {
        for id in [memory_grants::FIRST_PARTY_MIND, memory_grants::COMPANION] {
            let manifest = Manifest { id: id.into(), memory: Memory::Yantrikdb, ..Default::default() };
            let err = grant_on_persons_install(&manifest).unwrap_err();
            assert!(err.contains("not an id"), "{err}");
        }
    }

    #[test]
    fn a_grants_file_that_cannot_be_trusted_is_neither_overwritten_nor_read_as_a_grant() {
        let dir = std::env::temp_dir().join(format!("yantrik-hm-damaged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("memory-grants.json");
        std::fs::write(&file, "{ not json").unwrap();
        let _using = memory_grants::test_file::using(file.clone());
        assert!(grant_on_persons_install(&hermes(Memory::Yantrikdb)).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
        assert!(granted_minds().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_row_says_where_its_memory_is_in_plain_words() {
        assert_eq!(line(Memory::Own, true, true), "");
        assert_eq!(line(Memory::Yantrikdb, true, true), "Memory: YantrikDB (shared with Yantrik Mind)");
        assert_eq!(line(Memory::Yantrikdb, false, true), SHARED, "granted at the click, while it installs");
        assert!(line(Memory::Yantrikdb, false, false).starts_with("Memory: Install gives it YantrikDB"));
        assert!(line(Memory::Yantrikdb, true, false).starts_with("Memory: not granted YantrikDB"));
    }

    #[test]
    fn only_a_mind_the_grants_give_something_counts_as_granted() {
        let mut store = Store::default();
        store.minds.insert("hermes".into(), Grants::ordinary());
        store.minds.insert("pi".into(), Grants::default());
        assert_eq!(granted_in(Some(&store)), ["hermes"]);
        assert!(granted_in(None).is_empty());
    }
}
