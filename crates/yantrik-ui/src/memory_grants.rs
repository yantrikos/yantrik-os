//! What each mind may do with the person's memory (#447), and the answer the memory server is
//! given when a mind shows it a credential.
//!
//! The person decides; nothing here widens by itself. A mind with no entry of its own gets the
//! defaults for what it is: the first-party Yantrik Mind and the shell's companion recall and keep
//! ordinary memories, every other mind gets nothing until the person enables it. Health and
//! finance are separate grants, household memory another, and credentials are never a grant at
//! all.
//!
//! The list lives in ~/.config/yantrik, where the desktop's own tools refuse every agent's write
//! (#443). That is not a wall around the file. A mind that runs as the person, as the third-party
//! harnesses (pi, hermes, openclaw, deepseek) still do, can open it and write it like any other
//! file of the person's. (A read-only mount in their units was tried and withdrawn: in a user
//! unit it takes PrivateUsers with it, under which ssh refuses the root-owned configuration it
//! reads, so a coding agent could no longer push over ssh; and a process running as the person
//! can ask the user manager to run something outside its unit anyway.) So until third-party minds run under
//! accounts of their own, per-mind grants among the person's own processes are advisory: they
//! hold against a mind that plays by the desktop's rules, not against one that edits the file.
//!
//! What the shell can do is not believe a widening it did not make. It keeps, in memory, the
//! grants it last saved or accepted (the baseline), and a file that grants any mind more than
//! that is read with the widening taken out, and a warning logged. The limit: on the first read
//! after the shell starts there is nothing to compare with, so whatever the file says then is
//! accepted.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

/// How long the memory server may rely on one answer before asking again (#447): short, so a
/// revoked grant is not honoured for long even if the push that revokes it is lost.
pub const VALID_FOR_MS: u64 = 2000;

/// The harness id the first-party Yantrik Mind attaches as.
pub const FIRST_PARTY_MIND: &str = "mind";
/// The shell's own companion, which is not an attached harness but holds grants like one.
pub const COMPANION: &str = "companion";

/// The largest grants file read. A few minds' worth of booleans is a few hundred bytes; anything
/// near this is not a list the person made, and is not read into memory to find out.
const LIMIT: u64 = 64 * 1024;

/// One mind's grants, by the names the memory server checks (the tool -> grant map on #447).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grants {
    #[serde(default)]
    pub recall_ordinary: bool,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    pub believe: bool,
    #[serde(default)]
    pub recall_health: bool,
    #[serde(default)]
    pub recall_finance: bool,
    #[serde(default)]
    pub household: bool,
}

impl Grants {
    /// Ordinary recall, remember and believe: the first-party defaults.
    pub fn ordinary() -> Grants {
        Grants { recall_ordinary: true, remember: true, believe: true, ..Grants::default() }
    }

    fn flags(&self) -> [(&'static str, bool); 6] {
        [
            ("recall_ordinary", self.recall_ordinary),
            ("remember", self.remember),
            ("believe", self.believe),
            ("recall_health", self.recall_health),
            ("recall_finance", self.recall_finance),
            ("household", self.household),
        ]
    }

    /// The grant names that are on, in the order the server's map lists them.
    pub fn names(&self) -> Vec<&'static str> {
        self.flags().into_iter().filter_map(|(name, on)| on.then_some(name)).collect()
    }

    /// Whether this grants anything at all. A mind with nothing is handed no credential.
    pub fn any(&self) -> bool {
        !self.names().is_empty()
    }

    /// Whether this grants anything `than` does not.
    fn widens(&self, than: &Grants) -> bool {
        self.flags().iter().zip(than.flags()).any(|((_, now), (_, was))| *now && !was)
    }

    /// Only what both grant: a widening taken out, and any narrowing kept, since a narrowing
    /// written behind the shell's back still takes nothing from the person.
    fn within(&self, bound: &Grants) -> Grants {
        Grants {
            recall_ordinary: self.recall_ordinary && bound.recall_ordinary,
            remember: self.remember && bound.remember,
            believe: self.believe && bound.believe,
            recall_health: self.recall_health && bound.recall_health,
            recall_finance: self.recall_finance && bound.recall_finance,
            household: self.household && bound.household,
        }
    }
}

/// The person's choices, by mind id. A mind missing from `minds` has its defaults.
///
/// A mind listed twice makes the whole file unreadable, rather than letting the second entry
/// quietly win: a reader of the file would see the first and believe it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default, deserialize_with = "each_mind_once")]
    pub minds: BTreeMap<String, Grants>,
}

impl Store {
    /// What `mind` may do. `first_party` says whether the process that attached as it is the
    /// person's own mind account, from the uid the kernel named at attach. A harness that merely
    /// calls itself `mind` gets nothing at all, not even an entry the file lists for `mind`: the
    /// name alone never earns what the person gave their own Mind.
    pub fn grants_for(&self, mind: &str, first_party: bool) -> Grants {
        if mind == FIRST_PARTY_MIND && !first_party {
            return Grants::default();
        }
        self.minds.get(mind).cloned().unwrap_or_else(|| defaults(mind))
    }

    /// The most `mind` could be granted by this list: its entry, or its defaults as if it were
    /// the first party. What a widening is measured by.
    fn most_for(&self, mind: &str) -> Grants {
        self.minds.get(mind).cloned().unwrap_or_else(|| defaults(mind))
    }
}

fn defaults(mind: &str) -> Grants {
    match mind {
        FIRST_PARTY_MIND | COMPANION => Grants::ordinary(),
        _ => Grants::default(),
    }
}

fn each_mind_once<'de, D: Deserializer<'de>>(from: D) -> Result<BTreeMap<String, Grants>, D::Error> {
    struct EachOnce;
    impl<'de> Visitor<'de> for EachOnce {
        type Value = BTreeMap<String, Grants>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("each mind's grants, each mind once")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
            let mut minds = BTreeMap::new();
            while let Some((mind, grants)) = entries.next_entry::<String, Grants>()? {
                if minds.insert(mind.clone(), grants).is_some() {
                    return Err(A::Error::custom(format!("`{mind}` is listed twice")));
                }
            }
            Ok(minds)
        }
    }
    from.deserialize_map(EachOnce)
}

/// Take out of `on_disk` whatever grants a mind more than `baseline` did, mind by mind. Answers
/// what is left, and which minds the file tried to widen.
fn settle(baseline: &Store, on_disk: Store) -> (Store, Vec<String>) {
    let mut settled = on_disk.clone();
    let mut widened = Vec::new();
    let named: std::collections::BTreeSet<&String> = baseline.minds.keys().chain(on_disk.minds.keys()).collect();
    for mind in named {
        let (was, now) = (baseline.most_for(mind), on_disk.most_for(mind));
        if now.widens(&was) {
            settled.minds.insert(mind.clone(), now.within(&was));
            widened.push(mind.clone());
        }
    }
    (settled, widened)
}

/// The grants file as this shell has seen it: what it last saved or accepted, and which minds a
/// widening was last logged for, so one edit is one warning and not one per question asked.
struct Keeper {
    baseline: Option<Store>,
    warned: Vec<String>,
}

static KEEPER: Mutex<Keeper> = Mutex::new(Keeper { baseline: None, warned: Vec::new() });
static SERIAL: AtomicU64 = AtomicU64::new(0);

impl Keeper {
    /// The person's choices at `path`, or `None` when the file is there and cannot be trusted:
    /// unreadable, not a plain file, too large, not the format, or a mind listed twice. `None`
    /// grants nobody anything, the first-party Mind and the companion included. Answering the
    /// defaults instead would undo every revocation the person made, which is what a damaged
    /// file must never do. A file that is not there at all is the defaults: nobody has chosen
    /// anything yet.
    fn load_from(&mut self, path: &Path) -> Option<Store> {
        let on_disk = match read(path) {
            Ok(None) => Store::default(),
            Ok(Some(text)) => match serde_json::from_str::<Store>(&text) {
                Ok(store) => store,
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "The memory grants file cannot be read as grants; no mind has memory until it is fixed (#447)");
                    return None;
                }
            },
            Err(why) => {
                tracing::error!(path = %path.display(), error = %why, "The memory grants file cannot be read; no mind has memory until it is fixed (#447)");
                return None;
            }
        };
        let (settled, widened) = match &self.baseline {
            Some(baseline) => settle(baseline, on_disk),
            None => (on_disk, Vec::new()),
        };
        if !widened.is_empty() && widened != self.warned {
            tracing::warn!(
                path = %path.display(),
                minds = ?widened,
                "The memory grants file grants more than this desktop saved; the widening is ignored (#447)"
            );
        }
        self.warned = widened;
        self.baseline = Some(settled.clone());
        Some(settled)
    }

    /// Replace the file at `path` whole with `store`, and make it the baseline.
    fn save_to(&mut self, path: &Path, store: &Store) -> Result<(), String> {
        let dir = path.parent().ok_or("the memory grants file has no folder")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let text = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
        // A name nobody else is using, made by this call alone (create_new) and private from its
        // first byte: never a fixed name that a link planted beforehand could send elsewhere.
        let temp = dir.join(format!(
            ".memory-grants-{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .and_then(|mut file| file.write_all(text.as_bytes()).and_then(|()| file.sync_all()))
            .and_then(|()| std::fs::rename(&temp, path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("{}: {e}", path.display()));
        }
        self.baseline = Some(store.clone());
        self.warned.clear();
        Ok(())
    }
}

/// The file's text, `None` when there is none. Not through a link, only a plain file, only up to
/// [`LIMIT`]: the same care `config_store` takes with the person's preferences.
fn read(path: &Path) -> Result<Option<String>, String> {
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("it is not a plain file".into());
    }
    if meta.len() > LIMIT {
        return Err(format!("it is larger than {LIMIT} bytes"));
    }
    let mut text = String::new();
    file.take(LIMIT + 1).read_to_string(&mut text).map_err(|e| e.to_string())?;
    if text.len() as u64 > LIMIT {
        return Err(format!("it is larger than {LIMIT} bytes"));
    }
    Ok(Some(text))
}

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let home = PathBuf::from(home);
    home.is_absolute().then(|| home.join(".config/yantrik/memory-grants.json"))
}

/// The person's choices as saved, with any widening the shell did not make taken out. `None`
/// when they cannot be trusted, which grants nobody anything; see [`Keeper::load_from`].
pub fn load() -> Option<Store> {
    let Some(path) = path() else {
        tracing::error!("There is no home directory to read memory grants from; no mind has memory (#447)");
        return None;
    };
    KEEPER.lock().unwrap_or_else(|e| e.into_inner()).load_from(&path)
}

/// Save the person's choices, replacing the file whole. What is saved is what the next read is
/// held to.
#[allow(dead_code)] // Settings -> Memory (#447 piece c) is its caller.
pub fn save(store: &Store) -> Result<(), String> {
    let path = path().ok_or("there is no home directory to keep memory grants in")?;
    KEEPER.lock().unwrap_or_else(|e| e.into_inner()).save_to(&path, store)
}

/// Whether a mind is handed a memory credential with its turns (#447): whether the person's
/// grants give it anything, judged as [`validate`] judges it, the first-party defaults going only
/// to the account that attached as the mind account. `store` is `None` when the grants file cannot
/// be trusted, which hands nobody anything. `uid` is the one the kernel named at attach.
pub fn carries_memory(store: Option<&Store>, harness: &str, uid: Option<u32>, is_mind: impl Fn(u32) -> bool) -> bool {
    store.is_some_and(|store| store.grants_for(harness, uid.is_some_and(is_mind)).any())
}

/// The answer the memory server is given for a credential the desktop issued (#447): who it is,
/// which mind, which attach, what it may do and for how long the answer holds. `mind` is the
/// stable id that writes are stamped with.
pub fn answer(person_uid: u32, mind: &str, attach: &str, grants: &Grants) -> Value {
    json!({
        "v": 1,
        "person_uid": person_uid,
        "mind": mind,
        "attach": attach,
        "grants": grants.names(),
        "valid_for_ms": VALID_FOR_MS,
    })
}

/// The shell's `memory_validate` (#447): the memory server's question, answered only when the
/// kernel says the asker is the mind account, the account that serves the memory. Asked by anyone
/// else it would be a way to test whether a stolen credential is still good.
///
/// The server is not an agent acting on the desktop, so it comes with no agent token; the
/// surface's standing rule lets this one action through without one (`yantrik_surface`'s
/// `STANDING_NOT_NEEDED`), and this check is what stands in its place.
///
/// `is_mind` and `choices` are the real ones in the shell and stand-ins in tests: no test machine
/// has the mind account.
pub fn validate(
    args: &Value,
    host: Option<&yantrik_harness::Host>,
    is_mind: impl Fn(u32) -> bool,
    choices: impl FnOnce() -> Option<Store>,
) -> Result<Value, String> {
    let asker = yantrik_app_runtime::control::caller();
    if !asker.is_some_and(|c| is_mind(c.uid)) {
        return Err("only the person's memory server asks this".into());
    }
    let digest = args["memory_sha256"].as_str().unwrap_or_default();
    let Some(held) = host.and_then(|host| host.memory_credential_holder_by_digest(digest)) else {
        return Ok(Value::Null);
    };
    let mind = held.agent.harness();
    // First party from the account the kernel named when the harness attached, kept since:
    // never a look at the pid now, which may belong to another process by this time.
    let first_party = held.uid.is_some_and(&is_mind);
    let grants = choices().map(|store| store.grants_for(mind, first_party)).unwrap_or_default();
    if !grants.any() {
        return Ok(Value::Null);
    }
    // SAFETY: geteuid cannot fail. The shell runs as the person whose memory this is.
    let person = unsafe { libc::geteuid() };
    Ok(answer(person, mind, &format!("{}@{}", held.agent, held.session), &grants))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_third_party_mind_gets_nothing_until_the_person_says_so() {
        let store = Store::default();
        for mind in ["hermes", "pi", "openclaw", "deepseek"] {
            assert_eq!(store.grants_for(mind, false), Grants::default(), "{mind}");
            assert!(!store.grants_for(mind, false).any());
        }
        let mut chosen = Store::default();
        chosen.minds.insert("hermes".into(), Grants { recall_ordinary: true, ..Grants::default() });
        assert_eq!(chosen.grants_for("hermes", false).names(), ["recall_ordinary"]);
    }

    #[test]
    fn the_first_party_defaults_go_only_to_the_real_first_party_mind() {
        let store = Store::default();
        assert_eq!(store.grants_for(FIRST_PARTY_MIND, true), Grants::ordinary());
        // A harness that attaches calling itself `mind` from another account earns nothing.
        assert_eq!(store.grants_for(FIRST_PARTY_MIND, false), Grants::default());
        assert_eq!(store.grants_for(COMPANION, false), Grants::ordinary());
    }

    #[test]
    fn what_the_person_gave_their_own_mind_does_not_go_to_a_harness_borrowing_its_name() {
        let mut store = Store::default();
        store.minds.insert(
            FIRST_PARTY_MIND.into(),
            Grants { recall_health: true, recall_finance: true, ..Grants::ordinary() },
        );
        assert_eq!(store.grants_for(FIRST_PARTY_MIND, false), Grants::default());
        assert!(store.grants_for(FIRST_PARTY_MIND, true).recall_finance);
    }

    #[test]
    fn the_persons_choice_outranks_the_defaults_both_ways() {
        let mut store = Store::default();
        store.minds.insert(FIRST_PARTY_MIND.into(), Grants::default());
        assert!(!store.grants_for(FIRST_PARTY_MIND, true).any(), "revoked from the first party too");
        store.minds.insert(COMPANION.into(), Grants { recall_health: true, ..Grants::ordinary() });
        assert_eq!(
            store.grants_for(COMPANION, false).names(),
            ["recall_ordinary", "remember", "believe", "recall_health"]
        );
    }

    #[test]
    fn the_answer_has_the_shape_both_sides_pinned() {
        let v = answer(1000, "hermes", "hermes:c-1a2b3c@s4", &Grants::ordinary());
        assert_eq!(v["v"], 1);
        assert_eq!(v["person_uid"], 1000);
        assert_eq!(v["mind"], "hermes");
        assert_eq!(v["attach"], "hermes:c-1a2b3c@s4");
        assert_eq!(v["grants"], json!(["recall_ordinary", "remember", "believe"]));
        assert_eq!(v["valid_for_ms"], VALID_FOR_MS);
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 6, "nothing more: {keys:?}");
    }

    #[test]
    fn a_credential_is_carried_only_by_a_mind_the_grants_give_something() {
        let is_mind = |uid: u32| uid == 990;
        let store = Store::default();
        // The first-party Mind's defaults, only from the mind account.
        assert!(carries_memory(Some(&store), FIRST_PARTY_MIND, Some(990), is_mind));
        assert!(!carries_memory(Some(&store), FIRST_PARTY_MIND, Some(1000), is_mind), "the name alone earns nothing");
        assert!(!carries_memory(Some(&store), FIRST_PARTY_MIND, None, is_mind), "nor does an account nobody named");
        // A third party with no entry has nothing until the person gives it something.
        assert!(!carries_memory(Some(&store), "pi", Some(1000), is_mind));
        let mut chosen = Store::default();
        chosen.minds.insert("pi".into(), Grants { remember: true, ..Grants::default() });
        assert!(carries_memory(Some(&chosen), "pi", Some(1000), is_mind));
        // A grants file that cannot be trusted hands nobody anything, the first party included.
        assert!(!carries_memory(None, FIRST_PARTY_MIND, Some(990), is_mind));
        assert!(!carries_memory(None, "pi", Some(1000), is_mind));
    }

    #[test]
    fn unknown_and_missing_grant_names_read_as_off_never_on() {
        let partial: Store = serde_json::from_str(r#"{"minds":{"pi":{"remember":true,"credentials":true}}}"#).unwrap();
        assert_eq!(partial.grants_for("pi", false).names(), ["remember"]);
    }

    #[test]
    fn a_mind_listed_twice_is_not_a_list_at_all() {
        let twice = r#"{"minds":{"pi":{},"pi":{"recall_health":true}}}"#;
        let err = serde_json::from_str::<Store>(twice).unwrap_err().to_string();
        assert!(err.contains("`pi` is listed twice"), "{err}");
    }

    /// A folder of its own under the temp dir, for one test's grants file.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-447-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fresh() -> Keeper {
        Keeper { baseline: None, warned: Vec::new() }
    }

    #[test]
    fn no_file_is_the_defaults_and_a_damaged_one_grants_nobody_anything() {
        let dir = scratch("damaged");
        let path = dir.join("memory-grants.json");
        let store = fresh().load_from(&path).expect("nothing chosen yet: the defaults");
        assert_eq!(store.grants_for(FIRST_PARTY_MIND, true), Grants::ordinary());

        // Not grants, too large, or a mind twice: nobody at all, not even the first party. The
        // defaults here would give back what the person revoked.
        for text in [
            "{ not json".to_string(),
            format!("{{\"minds\":{{}},\"pad\":\"{}\"}}", "x".repeat(LIMIT as usize)),
            r#"{"minds":{"mind":{},"mind":{}}}"#.to_string(),
        ] {
            std::fs::write(&path, &text).unwrap();
            assert_eq!(fresh().load_from(&path), None, "{}", &text[..text.len().min(40)]);
        }

        // Not a plain file, and not through a link.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(fresh().load_from(&path), None, "a folder");
        std::fs::remove_dir(&path).unwrap();
        let elsewhere = dir.join("elsewhere.json");
        std::fs::write(&elsewhere, r#"{"minds":{}}"#).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
        assert_eq!(fresh().load_from(&path), None, "a link");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_list_is_private_and_leaves_nothing_beside_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("save");
        let path = dir.join("yantrik/memory-grants.json");
        let mut store = Store::default();
        store.minds.insert("pi".into(), Grants { remember: true, ..Grants::default() });
        let mut keeper = fresh();
        keeper.save_to(&path, &store).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let beside: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(beside, [std::ffi::OsString::from("memory-grants.json")], "no temp file left");
        assert_eq!(keeper.load_from(&path), Some(store.clone()));
        // Saved again over it: replaced whole.
        store.minds.clear();
        keeper.save_to(&path, &store).unwrap();
        assert_eq!(fresh().load_from(&path), Some(Store::default()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_widening_the_shell_did_not_make_is_ignored_and_a_narrowing_is_kept() {
        let dir = scratch("widen");
        let path = dir.join("memory-grants.json");
        let mut keeper = fresh();
        let mut saved = Store::default();
        saved.minds.insert("pi".into(), Grants { remember: true, recall_ordinary: true, ..Grants::default() });
        saved.minds.insert(FIRST_PARTY_MIND.into(), Grants::default());
        keeper.save_to(&path, &saved).unwrap();

        // Written behind the shell's back: pi gains health and loses recall, a stranger gains
        // everything, and the revoked first-party Mind is given its defaults back by deleting
        // its entry.
        std::fs::write(
            &path,
            r#"{"minds":{"pi":{"remember":true,"recall_health":true},"stranger":{"recall_ordinary":true,"household":true}}}"#,
        )
        .unwrap();
        let read = keeper.load_from(&path).unwrap();
        assert_eq!(read.grants_for("pi", false).names(), ["remember"], "health ignored, the lost recall stays lost");
        assert!(!read.grants_for("stranger", false).any());
        assert!(!read.grants_for(FIRST_PARTY_MIND, true).any(), "a deleted revocation is still a revocation");
        assert_eq!(keeper.warned, ["mind", "pi", "stranger"], "each named in the warning");

        // A file removed outright cannot give the first party back what the person took.
        std::fs::remove_file(&path).unwrap();
        assert!(!keeper.load_from(&path).unwrap().grants_for(FIRST_PARTY_MIND, true).any());

        // What the shell itself saves is believed: the person widened it in Settings.
        let mut wider = Store::default();
        wider.minds.insert("stranger".into(), Grants { household: true, ..Grants::default() });
        keeper.save_to(&path, &wider).unwrap();
        assert!(keeper.load_from(&path).unwrap().grants_for("stranger", false).household);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_read_after_start_has_nothing_to_compare_with_and_takes_the_file() {
        // The documented limit: a shell that has saved nothing yet believes what it finds.
        let dir = scratch("first");
        let path = dir.join("memory-grants.json");
        std::fs::write(&path, r#"{"minds":{"stranger":{"household":true}}}"#).unwrap();
        assert!(fresh().load_from(&path).unwrap().grants_for("stranger", false).household);
        let _ = std::fs::remove_dir_all(&dir);
    }

    mod validate {
        use super::super::*;
        use yantrik_app_runtime::control::{Caller, CallerScope};
        use yantrik_harness::{protocol, Host};

        /// The mind account, as these tests pretend it is: no test machine has one.
        const MIND_UID: u32 = 990;
        fn is_mind(uid: u32) -> bool {
            uid == MIND_UID
        }

        fn person() -> u32 {
            unsafe { libc::geteuid() }
        }

        /// A host with `id` attached by a process running as `uid`, and one agent's memory
        /// credential digest.
        fn host_with(id: &str, uid: u32) -> (Host, String) {
            let host = Host::new(vec![]);
            host.handle_from(
                protocol::ATTACH,
                &json!({ "id": id, "name": id, "conversations": true }),
                Some(std::process::id()),
                Some(uid),
            )
            .unwrap();
            let agent = host.start_agent(id).unwrap();
            let credential = host.memory_credential(&agent, yantrik_ipc_transport::reach::token_digest).unwrap().unwrap();
            (host, yantrik_ipc_transport::reach::token_digest(&credential))
        }

        fn ask(caller: Option<Caller>, digest: &str, host: Option<&Host>, choices: Option<Store>) -> Result<Value, String> {
            let _who = CallerScope::enter(caller);
            validate(&json!({ "memory_sha256": digest }), host, is_mind, || choices)
        }

        fn caller(uid: u32) -> Option<Caller> {
            Some(Caller { pid: 4242, uid, gid: uid })
        }

        #[test]
        fn only_the_mind_account_is_answered_and_it_needs_no_agent_token() {
            let unknown = "0".repeat(64);
            assert!(ask(None, &unknown, None, Some(Store::default())).is_err(), "nobody the kernel named");
            let err = ask(caller(person()), &unknown, None, Some(Store::default())).unwrap_err();
            assert!(err.contains("memory server"), "the person is refused too: {err}");
            // The memory server: no agent token anywhere in this call, and it is answered.
            assert_eq!(ask(caller(MIND_UID), &unknown, None, Some(Store::default())), Ok(Value::Null));
        }

        #[test]
        fn a_known_credential_is_answered_with_its_mind_its_attach_and_its_grants() {
            let (host, digest) = host_with("pi", person());
            let mut store = Store::default();
            store.minds.insert("pi".into(), Grants { remember: true, ..Grants::default() });
            let answered = ask(caller(MIND_UID), &digest, Some(&host), Some(store)).unwrap();
            assert_eq!(answered["mind"], "pi");
            assert_eq!(answered["grants"], json!(["remember"]));
            assert_eq!(answered["person_uid"], person());
            let attach = answered["attach"].as_str().unwrap();
            assert!(attach.starts_with("pi:") && attach.contains("@s1-"), "{attach}");
            // A third party the person never enabled holds a credential that grants nothing.
            assert_eq!(ask(caller(MIND_UID), &digest, Some(&host), Some(Store::default())), Ok(Value::Null));
        }

        #[test]
        fn the_first_party_is_decided_by_the_account_that_attached() {
            let (real, digest) = host_with(FIRST_PARTY_MIND, MIND_UID);
            let answered = ask(caller(MIND_UID), &digest, Some(&real), Some(Store::default())).unwrap();
            assert_eq!(answered["grants"], json!(["recall_ordinary", "remember", "believe"]));

            // The same name from the person's own account earns nothing, even what the person
            // listed for their Mind by name.
            let (borrowed, digest) = host_with(FIRST_PARTY_MIND, person());
            let mut store = Store::default();
            store.minds.insert(FIRST_PARTY_MIND.into(), Grants::ordinary());
            assert_eq!(ask(caller(MIND_UID), &digest, Some(&borrowed), Some(store)), Ok(Value::Null));
        }

        #[test]
        fn a_grants_file_that_cannot_be_trusted_answers_nobody() {
            let (real, digest) = host_with(FIRST_PARTY_MIND, MIND_UID);
            assert_eq!(ask(caller(MIND_UID), &digest, Some(&real), None), Ok(Value::Null));
        }
    }
}
