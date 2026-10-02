//! A harness whose memory is the machine's YantrikDB (#447): granting it, and saying so.
//!
//! Installing Hermes from this desktop was meant to give Hermes the memory Yantrik Mind keeps, so
//! the two remember the same person, and not a second memory of them in ~/.hermes. A manifest says
//! `memory: yantrikdb` for a harness built that way, and the person's Install click on its row,
//! once the install has worked, grants it ordinary recall, remember and believe, saved where the
//! person's grants are saved, so the shell's baseline holds it and a later edit behind the shell's
//! back cannot widen it.
//!
//! A harness the grants file names with nothing granted (`"hermes": {}`) is one the person took
//! the memory away from, and installing it again does not give it back: a reinstall is not the
//! person changing their mind about its memory. No entry at all is a harness nobody has decided
//! about yet, which Install may grant.
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

/// What the person's Install did about a harness's memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Its manifest does not say its memory is the machine's.
    NotAsked,
    Granted,
    /// The person had taken it away, and it stays taken away.
    KeptRevoked,
}

/// Whether the person has taken `id`'s memory away: an entry for it that grants nothing.
pub fn revoked(store: &Store, id: &str) -> bool {
    store.minds.get(id).is_some_and(|grants| !grants.any())
}

/// The grants after the person's Install click: ordinary recall, remember and believe on, and
/// whatever else the person chose for this harness left as it was. Unchanged for a harness the
/// person took the memory away from.
pub fn with_install_grant(mut store: Store, id: &str) -> Store {
    if revoked(&store, id) {
        return store;
    }
    let ordinary = Grants::ordinary();
    let grants = store.minds.entry(id.to_string()).or_default();
    grants.recall_ordinary = ordinary.recall_ordinary;
    grants.remember = ordinary.remember;
    grants.believe = ordinary.believe;
    store
}

/// Withdraw what `id` holds of the machine's memory, now: every credential its agents hold stops
/// naming anyone (so `memory_validate` stops vouching for it, and the memory server refuses it),
/// and the harness is told on its next poll to void every one it holds itself (`memory_revoked`).
/// Answers how many credentials were withdrawn. Writes no grant: for a failed install, where
/// whatever the harness held is not left standing and the person's own choices are not touched.
pub fn withdraw_live(host: Option<&yantrik_harness::Host>, id: &str) -> usize {
    let withdrawn = host.map(|host| host.revoke_memory_credentials(id).len()).unwrap_or(0);
    tracing::info!(harness = %id, withdrawn, "Withdrew a harness's live memory credentials (#447)");
    withdrawn
}

/// The person takes `id`'s memory away, or removes it: its grants are emptied (the entry stays, so
/// the row says it was taken away and a reinstall does not give it back) and then every live
/// credential is withdrawn. In that order, so `memory_validate` already says no when the harness
/// is told. The two real callers are the person's own take-away and an uninstall.
pub fn take_away(host: Option<&yantrik_harness::Host>, id: &str) -> Result<usize, String> {
    if [memory_grants::FIRST_PARTY_MIND, memory_grants::COMPANION].contains(&id) {
        return Err(format!("`{id}` has grants of its own; they are not taken away here"));
    }
    memory_grants::update(|store| {
        store.minds.insert(id.to_string(), Grants::default());
    })?;
    Ok(withdraw_live(host, id))
}

/// Grant `manifest`'s harness the machine's memory, for the person's own Install click and for
/// nothing else.
pub fn grant_on_persons_install(manifest: &Manifest) -> Result<Outcome, String> {
    if manifest.memory != Memory::Yantrikdb {
        return Ok(Outcome::NotAsked);
    }
    // These two have grants of their own by being what they are. A manifest that borrows one of
    // their names is not either of them, and writing its grant under that name would change what
    // the real one may do.
    if [memory_grants::FIRST_PARTY_MIND, memory_grants::COMPANION].contains(&manifest.id.as_str()) {
        return Err(format!("`{}` is not an id a harness can be granted memory under", manifest.id));
    }
    let mut outcome = Outcome::Granted;
    memory_grants::update(|store| {
        if revoked(store, &manifest.id) {
            outcome = Outcome::KeptRevoked;
        }
        *store = with_install_grant(std::mem::take(store), &manifest.id);
    })?;
    match outcome {
        Outcome::KeptRevoked => tracing::info!(
            harness = %manifest.id,
            "Installed again; its memory stays taken away, as the person left it (#447)"
        ),
        _ => tracing::info!(harness = %manifest.id, "The person's Install gave this harness ordinary memory grants (#447)"),
    }
    Ok(outcome)
}

/// The harnesses the person's grants give some use of their memory, as third parties: what each
/// row's memory line is decided from. None when the grants file cannot be trusted, which grants
/// nobody anything.
pub fn granted_minds() -> Vec<String> {
    granted_in(memory_grants::load().as_ref())
}

/// [`granted_minds`], and the harnesses the person took the memory away from, from one read.
pub fn decided_minds() -> (Vec<String>, Vec<String>) {
    let store = memory_grants::load();
    let revoked = store
        .as_ref()
        .map(|store| store.minds.keys().filter(|id| revoked(store, id)).cloned().collect())
        .unwrap_or_default();
    (granted_in(store.as_ref()), revoked)
}

fn granted_in(store: Option<&Store>) -> Vec<String> {
    store
        .map(|store| store.minds.keys().filter(|id| store.grants_for(id, false).any()).cloned().collect())
        .unwrap_or_default()
}

/// The row's line about memory. Empty for a harness that keeps its own. `installed` is whether
/// it is past installing, `granted` whether the person's grants give it anything now, `revoked`
/// whether the person took it away.
pub fn line(memory: Memory, installed: bool, granted: bool, revoked: bool) -> String {
    match (memory, granted, installed) {
        (Memory::Own, _, _) => String::new(),
        (Memory::Yantrikdb, true, _) => SHARED.to_string(),
        (Memory::Yantrikdb, false, _) if revoked => {
            "Memory: none. You took YantrikDB away from it, and installing it again does not give it back".to_string()
        }
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

    /// Taking the memory away (security review round 4, F2): the grant is emptied and the row says
    /// so, the credentials stop naming anyone, and the harness's next poll tells it to void them.
    #[test]
    fn taking_memory_away_empties_the_grant_withdraws_the_credentials_and_tells_the_harness() {
        use yantrik_harness::{protocol, Host};
        let dir = std::env::temp_dir().join(format!("yantrik-574-takeaway-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _using = memory_grants::test_file::using(dir.join("grants.json"));
        memory_grants::update(|store| *store = with_install_grant(std::mem::take(store), "memtest-take")).unwrap();
        assert_eq!(granted_minds(), ["memtest-take"]);

        let host = Host::new(vec![]);
        let attach = serde_json::json!({ "id": "memtest-take", "name": "t", "conversations": true });
        let session = host
            .handle_from(protocol::ATTACH, &attach, Some(std::process::id()), Some(1000))
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = host.start_agent("memtest-take").unwrap();
        let hash = yantrik_ipc_transport::reach::token_digest;
        let credential = host.memory_credential(&agent, hash).unwrap().unwrap();
        assert!(host.memory_credential_holder(&credential).is_some());

        assert_eq!(take_away(Some(&host), "memtest-take"), Ok(1));
        let store = memory_grants::load().expect("the grants read back");
        assert!(revoked(&store, "memtest-take"), "the entry stays, empty, so a reinstall does not give it back");
        assert!(decided_minds().1.contains(&"memtest-take".to_string()));
        assert_eq!(host.memory_credential_holder(&credential), None);
        let poll = host
            .handle_from(protocol::POLL, &serde_json::json!({ "session": session }), Some(std::process::id()), Some(1000))
            .unwrap();
        assert_eq!(poll["memory_revoked"], true, "{poll}");

        // Not for the first party or the companion, whose grants are their own.
        for id in [memory_grants::FIRST_PARTY_MIND, memory_grants::COMPANION] {
            assert!(take_away(Some(&host), id).is_err(), "{id}");
        }
        // With no host (before the shell has wired one) it still empties the grant and withdraws nothing.
        assert_eq!(take_away(None, "memtest-other"), Ok(0));
        assert_eq!(withdraw_live(None, "x"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        assert_eq!(grant_on_persons_install(&hermes(Memory::Own)), Ok(Outcome::NotAsked));
        assert!(!file.exists());
        assert_eq!(grant_on_persons_install(&hermes(Memory::Yantrikdb)), Ok(Outcome::Granted));
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
        assert_eq!(line(Memory::Own, true, true, false), "");
        assert_eq!(line(Memory::Yantrikdb, true, true, false), "Memory: YantrikDB (shared with Yantrik Mind)");
        assert_eq!(line(Memory::Yantrikdb, false, true, false), SHARED, "granted already, as during a reinstall");
        assert!(line(Memory::Yantrikdb, false, false, false).starts_with("Memory: Install gives it YantrikDB"));
        assert!(line(Memory::Yantrikdb, true, false, false).starts_with("Memory: not granted YantrikDB"));
        // Taken away: said so, before a reinstall as well as after, so Install is not read as
        // giving it back.
        for installed in [true, false] {
            let said = line(Memory::Yantrikdb, installed, false, true);
            assert!(said.contains("You took YantrikDB away") && said.contains("does not give it back"), "{said}");
        }
    }

    #[test]
    fn a_reinstall_does_not_give_back_memory_the_person_took_away() {
        let mut store = Store::default();
        store.minds.insert("hermes".into(), Grants::default());
        let after = with_install_grant(store.clone(), "hermes");
        assert_eq!(after, store, "an explicit empty entry is a revocation and is kept");
        // No entry at all is nobody having decided, which Install may grant.
        assert!(with_install_grant(Store::default(), "hermes").grants_for("hermes", false).any());
    }

    #[test]
    fn installing_again_after_a_revocation_writes_no_grant_and_says_so() {
        let dir = std::env::temp_dir().join(format!("yantrik-hm-revoked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("memory-grants.json");
        std::fs::write(&file, r#"{"minds":{"hermes":{}}}"#).unwrap();
        let _using = memory_grants::test_file::using(file.clone());
        assert_eq!(grant_on_persons_install(&hermes(Memory::Yantrikdb)), Ok(Outcome::KeptRevoked));
        assert!(granted_minds().is_empty());
        assert_eq!(decided_minds(), (vec![], vec!["hermes".to_string()]));
        let _ = std::fs::remove_dir_all(&dir);
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
