//! The agent catalog: reusable roles that work can be handed to.
//!
//! See `design/desk-and-mind-2026-09-23.md`, section 5. A catalog entry is a **role, not a mind**:
//! what it is for, which attached minds run it (a preference list, first attached wins), its
//! standing instructions (`brief`), what it may touch (`reach`: surfaces and a grade ceiling
//! narrower than the machine's), what it hands back (`returns`) and how long it has (`budget`).
//! `shell.hand_off` starts one on a task (`control_agents::hand_off`); the Agents screen's New agent
//! does the same "from the catalog".
//!
//! # Where roles come from
//!
//! Three layers, each replacing a role of the same `id` in the one before:
//!
//! 1. the shipped roles, compiled in from `config/agents/*.toml` — the floor, so a machine whose
//!    image lost a file still has every role;
//! 2. the image's copies, `/opt/yantrik/share/agents/*.toml` (`build-release.sh` puts the same
//!    files there, for a person to read and copy);
//! 3. the person's own, `~/.config/yantrik/agents/*.toml`.
//!
//! A file that does not read as a role is skipped and said (`Catalog::problems`, which `describe
//! shell` shows under `catalog`) — never half-read: an unknown key is an error, because a typo in
//! `reach` must not quietly become a wider reach.
//!
//! # A role's reach is enforced
//!
//! Not here: this file only reads it. `agents::reaches` publishes it for each agent started as a
//! role, and every door that carries the agent's token holds the call to it
//! (`yantrik_ipc_transport::reach`).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
use yantrik_ipc_transport::{gate, reach};

use super::model::RoleMeta;

/// Where the image puts its copies of the shipped roles.
pub const SHARE: &str = "/opt/yantrik/share/agents";

/// The shipped roles, compiled in: `(file name, text)`.
pub const SHIPPED: [(&str, &str); 8] = [
    ("researcher.toml", include_str!("../../../../config/agents/researcher.toml")),
    ("planner.toml", include_str!("../../../../config/agents/planner.toml")),
    ("coder.toml", include_str!("../../../../config/agents/coder.toml")),
    ("reviewer.toml", include_str!("../../../../config/agents/reviewer.toml")),
    ("red-team.toml", include_str!("../../../../config/agents/red-team.toml")),
    ("writer.toml", include_str!("../../../../config/agents/writer.toml")),
    ("chair.toml", include_str!("../../../../config/agents/chair.toml")),
    ("scribe.toml", include_str!("../../../../config/agents/scribe.toml")),
];

/// The most a role may ask for: turns in one agent's life, and minutes from its start.
pub const MOST_TURNS: u32 = 50;
pub const MOST_MINUTES: u32 = 240;

/// The longest brief a role may carry. It is the head of every first prompt the role is given.
pub const BRIEF_MOST_BYTES: usize = 8 * 1024;

/// A role file larger than this is not read.
const FILE_MOST_BYTES: u64 = 64 * 1024;

/// The person's own roles: `$XDG_CONFIG_HOME/yantrik/agents`, or `~/.config/yantrik/agents`.
pub fn person_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(".config")
        })
        .join("yantrik")
        .join("agents")
}

/// Which layer a role came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Shipped,
    Image,
    Person,
}

impl Source {
    pub fn key(self) -> &'static str {
        match self {
            Source::Shipped => "shipped",
            Source::Image => "image",
            Source::Person => "yours",
        }
    }
}

/// What a role may touch: surfaces (`app`, `app.action`, `app.prefix*`) and the highest grade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reach {
    pub surfaces: Vec<String>,
    pub ceiling: String,
}

impl Reach {
    /// "editor, documents and notes · at most safe".
    pub fn text(&self) -> String {
        format!("{} · at most {}", reach::surfaces_text(&self.surfaces), self.ceiling)
    }
}

/// How long a role's agent has: turns, and minutes from its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub turns: u32,
    pub minutes: u32,
}

/// One role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: String,
    pub name: String,
    /// What it is for, in one line.
    pub purpose: String,
    /// Attached minds that run it, best first.
    pub mind: Vec<String>,
    /// Its standing instructions: the head of every first prompt it is given.
    pub brief: String,
    pub reach: Reach,
    /// What it hands back, in words.
    pub returns: String,
    pub budget: Budget,
    pub source: Source,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleFile {
    id: String,
    name: String,
    purpose: String,
    mind: Vec<String>,
    brief: String,
    returns: String,
    reach: ReachFile,
    budget: BudgetFile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReachFile {
    #[serde(default)]
    surfaces: Vec<String>,
    ceiling: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetFile {
    turns: u32,
    minutes: u32,
}

fn one_line(field: &str, value: &str, most: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("`{field}` is empty"));
    }
    if value.contains('\n') {
        return Err(format!("`{field}` is more than one line"));
    }
    if value.chars().count() > most {
        return Err(format!("`{field}` is longer than {most} characters"));
    }
    Ok(value.to_string())
}

/// One surface as the catalog keeps it: the app part named the way the app publishes itself
/// (`text-editor` is `editor`), the action part as written.
fn surface(given: &str) -> Result<String, String> {
    let given = given.trim();
    let bad = || {
        format!(
            "reach surface `{given}` is not `app`, `app.action` or `app.prefix*` — e.g. `notes`, \
             `shell.agent_run`, `shell.agent_*`"
        )
    };
    if given.is_empty() || given.contains(char::is_whitespace) || given.matches('*').count() > 1 {
        return Err(bad());
    }
    let (app, action) = match given.split_once('.') {
        Some((app, action)) => (app, Some(action)),
        None => (given, None),
    };
    if app.is_empty() || app.contains('*') || action.is_some_and(|a| a.is_empty() || a.contains('.') || (a.contains('*') && !a.ends_with('*'))) {
        return Err(bad());
    }
    // An app's names are its `.desktop` file's (`crate::surfaces`); ours from the build first.
    let app = crate::surfaces::surface_id(app).unwrap_or_else(|| app.to_lowercase());
    Ok(match action {
        Some(action) => format!("{app}.{action}"),
        None => app,
    })
}

/// Read one role file. Every field is required; a value that could only be a mistake — an empty
/// mind list, a grade off the ladder, a budget of nothing — is refused with the field named.
pub fn parse(text: &str, source: Source) -> Result<Role, String> {
    let file: RoleFile = toml::from_str(text).map_err(|e| format!("not a role: {}", e.message()))?;
    let id = file.id.trim().to_string();
    if id.is_empty() || id.len() > 40 || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err(format!("`id` is `{id}`; an id is up to 40 of a-z, 0-9, - and _"));
    }
    let name = one_line("name", &file.name, 40)?;
    let purpose = one_line("purpose", &file.purpose, 160)?;
    let returns = file.returns.trim().to_string();
    if returns.is_empty() {
        return Err("`returns` is empty: say what it hands back".into());
    }
    let mind: Vec<String> = file.mind.iter().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
    if mind.is_empty() || mind.iter().any(|m| m.contains(char::is_whitespace)) {
        return Err("`mind` lists no mind: the ids of the minds that run it, best first (pi, deepseek)".into());
    }
    let brief = file.brief.trim().to_string();
    if brief.is_empty() {
        return Err("`brief` is empty: its standing instructions".into());
    }
    if brief.len() > BRIEF_MOST_BYTES {
        return Err(format!("`brief` is {} bytes; at most {BRIEF_MOST_BYTES}", brief.len()));
    }
    let ceiling = file.reach.ceiling.trim().to_string();
    if gate::grade(&ceiling).is_none() {
        return Err(format!("`reach.ceiling` is `{ceiling}`, which is not one of {}", gate::LADDER.join(", ")));
    }
    let surfaces = file.reach.surfaces.iter().map(|s| surface(s)).collect::<Result<Vec<_>, _>>()?;
    let budget = Budget { turns: file.budget.turns, minutes: file.budget.minutes };
    if !(1..=MOST_TURNS).contains(&budget.turns) || !(1..=MOST_MINUTES).contains(&budget.minutes) {
        return Err(format!(
            "`budget` is {} turns and {} minutes; a role has 1 to {MOST_TURNS} turns and 1 to {MOST_MINUTES} minutes",
            budget.turns, budget.minutes
        ));
    }
    Ok(Role { id, name, purpose, mind, brief, reach: Reach { surfaces, ceiling }, returns, budget, source })
}

/// An attached mind, as far as a role is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mind {
    pub id: String,
    /// Whether it holds a conversation per agent. One that does not has only the person's own
    /// conversation, which a role is never started in.
    pub conversations: bool,
}

/// The minds attached right now, the built-in companion aside.
pub fn minds_now(host: &yantrik_harness::Host) -> Vec<Mind> {
    host.list()
        .into_iter()
        .filter(|e| !e.builtin)
        .map(|e| Mind { conversations: host.holds_conversations(&e.id).unwrap_or(false), id: e.id })
        .collect()
}

impl Role {
    /// The first of its minds attached now that can give it a conversation of its own — or why
    /// there is none, naming what it runs on and what is attached.
    pub fn pick_mind(&self, attached: &[Mind]) -> Result<String, String> {
        if let Some(mind) = self.mind.iter().find(|m| attached.iter().any(|a| &a.id == *m && a.conversations)) {
            return Ok(mind.clone());
        }
        let prefers = self.mind.join(", ");
        let one: Vec<&str> = attached.iter().filter(|a| !a.conversations).map(|a| a.id.as_str()).collect();
        let many: Vec<&str> = attached.iter().filter(|a| a.conversations).map(|a| a.id.as_str()).collect();
        let mut said = format!("No mind the {} runs on is attached: it runs on {prefers}", self.name);
        match (many.is_empty(), one.is_empty()) {
            (true, true) => said.push_str(", and no mind is attached at all"),
            (false, _) => said.push_str(&format!("; attached are {}", many.join(", "))),
            (true, false) => {}
        }
        if !one.is_empty() {
            said.push_str(&format!(
                "; {} {} one conversation at a time — the person's own — so it cannot take a role",
                one.join(", "),
                if one.len() == 1 { "holds" } else { "each hold" }
            ));
        }
        said.push_str(". Start one of its minds in Settings → Harnesses, or pick another role.");
        Err(said)
    }

    /// What its agent is started with: who it is, its standing instructions, what to hand back,
    /// its reach and budget in words — and the apps its reach names, which it may open (#195) —
    /// then the task and anything given to read first.
    pub fn first_turn(&self, task: &str, context: &str) -> String {
        let apps = reach::apps_named(&self.reach.surfaces);
        let opens = if apps.is_empty() {
            String::new()
        } else {
            format!(
                " You may open {apps} when closed (`shell.open_app name=<app>`), whatever your \
                 ceiling; inside them you are still held to `{ceiling}`.",
                apps = reach::surfaces_text(&apps),
                ceiling = self.reach.ceiling,
            )
        };
        let mut text = format!(
            "You are the {name} on this desktop, started to do one piece of work and hand it back. \
             {purpose}\n\n{brief}\n\nWhat you hand back: {returns}\n\nYour reach: {surfaces}, at \
             most `{ceiling}`.{opens} The desktop refuses anything else you try, so do not try it; \
             say in your answer what else needs doing.\nYour budget: {turns} turns and {minutes} \
             minutes. After that you are stopped, so hand back what you have before then.\n\nThe \
             task:\n{task}",
            name = self.name,
            purpose = self.purpose,
            brief = self.brief,
            returns = self.returns,
            surfaces = reach::surfaces_text(&self.reach.surfaces),
            ceiling = self.reach.ceiling,
            turns = self.budget.turns,
            minutes = self.budget.minutes,
            task = task.trim(),
        );
        if !context.trim().is_empty() {
            text.push_str(&format!("\n\nRead this first:\n{}", context.trim()));
        }
        text
    }

    /// What an agent started as this role keeps of it.
    pub fn meta(&self) -> RoleMeta {
        RoleMeta {
            id: self.id.clone(),
            name: self.name.clone(),
            reach: self.reach.text(),
            turns: self.budget.turns,
            minutes: self.budget.minutes,
        }
    }

    /// The definition's digest (SHA-256, hex) over everything that decides what the role does:
    /// its name, the minds it runs on, its brief, its reach, what it returns and its budget. A
    /// recipe's run records it for each role the person agreed to, and starts nothing whose
    /// definition has changed since: a file in ~/.config/yantrik/agents with the same id replaces
    /// a role whole — its mind and its reach with it.
    pub fn digest(&self) -> String {
        let definition = json!({
            "id": self.id,
            "name": self.name,
            "mind": self.mind,
            "brief": self.brief,
            "surfaces": self.reach.surfaces,
            "ceiling": self.reach.ceiling,
            "returns": self.returns,
            "turns": self.budget.turns,
            "minutes": self.budget.minutes,
        });
        reach::token_digest(&definition.to_string())
    }

    /// The reach as a door holds it, for `agent`.
    pub fn reach_for(&self, agent: &str) -> reach::Reach {
        reach::Reach {
            agent: agent.to_string(),
            role: self.id.clone(),
            name: self.name.clone(),
            surfaces: self.reach.surfaces.clone(),
            ceiling: self.reach.ceiling.clone(),
        }
    }
}

/// Every role, and what could not be read.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub roles: Vec<Role>,
    /// One sentence per file that was skipped, naming the file and why.
    pub problems: Vec<String>,
}

impl Catalog {
    /// The catalog as the machine has it now: shipped, the image's, the person's.
    pub fn load() -> Catalog {
        Catalog::from_layers(&SHIPPED, &[(PathBuf::from(SHARE), Source::Image), (person_dir(), Source::Person)])
    }

    /// The catalog from compiled-in roles and directories, each layer replacing a role of the same
    /// id in the one before and adding the rest after.
    pub fn from_layers(shipped: &[(&str, &str)], dirs: &[(PathBuf, Source)]) -> Catalog {
        let mut catalog = Catalog::default();
        for (name, text) in shipped {
            match parse(text, Source::Shipped) {
                Ok(role) => catalog.put(role),
                Err(why) => catalog.problems.push(format!("shipped role {name}: {why}")),
            }
        }
        for (dir, source) in dirs {
            catalog.read_dir(dir, *source);
        }
        catalog
    }

    fn put(&mut self, role: Role) {
        match self.roles.iter_mut().find(|r| r.id == role.id) {
            Some(known) => *known = role,
            None => self.roles.push(role),
        }
    }

    fn read_dir(&mut self, dir: &Path, source: Source) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.is_file())
            .collect();
        files.sort();
        for path in files {
            let shown = display(&path);
            let read = match std::fs::metadata(&path) {
                Ok(m) if m.len() > FILE_MOST_BYTES => Err(format!("larger than {} KiB", FILE_MOST_BYTES / 1024)),
                _ => std::fs::read_to_string(&path).map_err(|e| e.to_string()),
            };
            match read.and_then(|text| parse(&text, source)) {
                Ok(role) => self.put(role),
                Err(why) => self.problems.push(format!("{shown}: {why}; it was skipped")),
            }
        }
    }

    /// A role by its id, or by the name a person reads (any case).
    pub fn find(&self, named: &str) -> Option<&Role> {
        let named = named.trim();
        self.roles
            .iter()
            .find(|r| r.id == named)
            .or_else(|| self.roles.iter().find(|r| r.name.eq_ignore_ascii_case(named) || r.id.eq_ignore_ascii_case(named)))
    }

    /// "researcher (Researcher), planner (Planner), …" — for a refusal that has to say what there is.
    pub fn listing(&self) -> String {
        self.roles.iter().map(|r| format!("{} ({})", r.id, r.name)).collect::<Vec<_>>().join(", ")
    }
}

/// A path as a person would type it: under `~` when it is in their home.
fn display(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if path.starts_with(&home) => format!("~/{}", path.strip_prefix(&home).unwrap_or(path).display()),
        _ => path.display().to_string(),
    }
}

/// One role as `describe shell` lists it under `catalog`, with whether a mind it runs on is
/// attached now.
pub fn role_json(role: &Role, attached: &[Mind]) -> Value {
    let runs_on = role.pick_mind(attached).ok();
    json!({
        "id": role.id,
        "name": role.name,
        "purpose": role.purpose,
        "reach": { "surfaces": role.reach.surfaces, "ceiling": role.reach.ceiling, "text": role.reach.text() },
        "mind": role.mind,
        "runs_on": runs_on,
        "available": runs_on.is_some(),
        "returns": role.returns,
        "budget": { "turns": role.budget.turns, "minutes": role.budget.minutes },
        "from": role.source.key(),
    })
}

/// What `describe shell` says under `catalog`: every role, whether it can be started now, and any
/// file of the person's that could not be read. `shell.hand_off` starts one.
pub fn for_describe() -> Value {
    let catalog = Catalog::load();
    let attached = crate::wire::harness::host().map(minds_now).unwrap_or_default();
    json!({
        "roles": catalog.roles.iter().map(|r| role_json(r, &attached)).collect::<Vec<_>>(),
        "problems": catalog.problems,
        "yours": display(&person_dir()),
        "how": "hand_off {role, task, context?, wait_seconds?} starts one; a file in `yours` with the same id replaces a role",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> Catalog {
        Catalog::from_layers(&SHIPPED, &[])
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-catalog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MINE: &str = r#"
id = "reviewer"
name = "Strict reviewer"
purpose = "Reviews my changes, harder."
mind = ["pi"]
brief = "Be strict."
returns = "Findings."
[reach]
surfaces = ["text-editor", "shell.agent_run"]
ceiling = "standard"
[budget]
turns = 2
minutes = 5
"#;

    #[test]
    fn the_eight_shipped_roles_read_whole_and_their_briefs_are_short_and_shaped() {
        let catalog = shipped();
        assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
        let ids: Vec<&str> = catalog.roles.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["researcher", "planner", "coder", "reviewer", "red-team", "writer", "chair", "scribe"]);
        for role in &catalog.roles {
            let words = role.brief.split_whitespace().count();
            assert!(words <= 150, "{}'s brief is {words} words", role.id);
            assert!(role.brief.contains("Answer in this shape:"), "{}'s brief says what shape to answer in", role.id);
            assert!(!role.purpose.is_empty() && !role.returns.is_empty() && !role.mind.is_empty(), "{}", role.id);
            // No role is shipped at the machine's own top: a role's reach is narrower.
            assert!(gate::grade(&role.reach.ceiling) < gate::grade("dangerous"), "{}", role.id);
        }
        let ceiling = |id: &str| catalog.find(id).unwrap().reach.ceiling.clone();
        assert_eq!(ceiling("reviewer"), "safe", "the design's example: a reviewer is safe");
        assert_eq!(ceiling("coder"), "sensitive", "a coder may ask for sensitive");
        assert!(catalog.find("red-team").unwrap().reach.surfaces.is_empty());
        // Line-end backslashes join a paragraph into one line; the shape keeps its lines.
        let brief = &catalog.find("reviewer").unwrap().brief;
        assert!(brief.contains("before the person ships it. Read the change"), "{brief}");
        assert!(brief.contains("Answer in this shape:\nVerdict"), "{brief}");
    }

    #[test]
    fn every_role_file_is_compiled_in_and_put_in_the_bundle() {
        // build-release.sh copies config/agents into the bundle's share/agents; the compiled-in
        // floor is those same files.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/agents");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".toml"))
            .collect();
        on_disk.sort();
        let mut shipped: Vec<String> = SHIPPED.iter().map(|(n, _)| n.to_string()).collect();
        shipped.sort();
        assert_eq!(on_disk, shipped, "every file in config/agents is compiled in, and nothing else");
        let script = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/build-release.sh")).unwrap();
        assert!(script.contains("config/agents/") && script.contains("share/agents"), "build-release.sh puts them in the bundle");
    }

    #[test]
    fn the_persons_role_replaces_the_shipped_one_of_the_same_id_and_adds_its_own() {
        let dir = scratch("override");
        std::fs::write(dir.join("reviewer.toml"), MINE).unwrap();
        std::fs::write(dir.join("tutor.toml"), MINE.replace("\"reviewer\"", "\"tutor\"").replace("Strict reviewer", "Tutor")).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a role").unwrap();
        let catalog = Catalog::from_layers(&SHIPPED, &[(dir.clone(), Source::Person)]);
        assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
        assert_eq!(catalog.roles.len(), 9, "eight shipped, one replaced, one added");
        let mine = catalog.find("reviewer").unwrap();
        assert_eq!((mine.name.as_str(), mine.source), ("Strict reviewer", Source::Person));
        assert_eq!(catalog.roles[3].id, "reviewer", "it keeps the shipped one's place");
        assert_eq!(mine.reach.surfaces, ["editor", "shell.agent_run"], "an app's other name is the name it publishes");
        assert_eq!(catalog.roles.last().unwrap().id, "tutor");
        assert_eq!(catalog.find("TUTOR").map(|r| r.id.as_str()), Some("tutor"), "by id or name, any case");
        assert_eq!(catalog.find("Red team").map(|r| r.id.as_str()), Some("red-team"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_role_is_skipped_and_said_never_half_read() {
        let dir = scratch("broken");
        // A typo in `reach` must not become a wider reach: an unknown key is refused.
        std::fs::write(dir.join("a.toml"), MINE.replace("ceiling =", "celing =")).unwrap();
        std::fs::write(dir.join("b.toml"), MINE.replace("\"standard\"", "\"root\"")).unwrap();
        std::fs::write(dir.join("c.toml"), MINE.replace("turns = 2", "turns = 0")).unwrap();
        std::fs::write(dir.join("d.toml"), MINE.replace("[\"pi\"]", "[]")).unwrap();
        std::fs::write(dir.join("e.toml"), MINE.replace("\"shell.agent_run\"", "\"shell.*.x\"")).unwrap();
        std::fs::write(dir.join("f.toml"), "id = [").unwrap();
        let catalog = Catalog::from_layers(&SHIPPED, &[(dir.clone(), Source::Person)]);
        assert_eq!(catalog.problems.len(), 6, "{:#?}", catalog.problems);
        assert!(catalog.problems.iter().all(|p| p.contains("it was skipped")), "{:#?}", catalog.problems);
        for (file, says) in [("a.toml", "ceiling"), ("b.toml", "`reach.ceiling` is `root`"), ("c.toml", "`budget`"), ("d.toml", "`mind`"), ("e.toml", "reach surface")] {
            assert!(catalog.problems.iter().any(|p| p.contains(file) && p.contains(says)), "{file}: {:#?}", catalog.problems);
        }
        assert_eq!(catalog.find("reviewer").unwrap().source, Source::Shipped, "the shipped role stands");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_role_runs_on_its_first_attached_mind_that_can_give_it_a_conversation() {
        let reviewer = shipped().find("reviewer").unwrap().clone();
        let mind = |id: &str, conversations| Mind { id: id.into(), conversations };
        assert_eq!(reviewer.pick_mind(&[mind("pi", true), mind("deepseek", true)]), Ok("deepseek".into()), "its preference, not the host's order");
        assert_eq!(reviewer.pick_mind(&[mind("openclaw", true)]), Ok("openclaw".into()), "falling down its list");
        let err = reviewer.pick_mind(&[mind("hermes", false)]).unwrap_err();
        assert!(err.starts_with("No mind the Reviewer runs on is attached: it runs on deepseek, pi, openclaw"), "{err}");
        assert!(err.contains("hermes holds one conversation at a time"), "{err}");
        let err = reviewer.pick_mind(&[mind("deepseek", false)]).unwrap_err();
        assert!(err.contains("deepseek holds one conversation"), "a preferred mind that cannot give it a conversation is passed over: {err}");
        let err = reviewer.pick_mind(&[]).unwrap_err();
        assert!(err.contains("no mind is attached at all"), "{err}");
    }

    #[test]
    fn a_first_turn_carries_the_brief_the_shape_the_reach_the_budget_and_the_task() {
        let reviewer = shipped().find("reviewer").unwrap().clone();
        let turn = reviewer.first_turn("  review the change in ~/src/app  ", "diff --git a/x b/x");
        for says in [
            "You are the Reviewer on this desktop",
            "Reviews a change for bugs and risks; reads only.",
            "Find what is wrong with a change",
            "What you hand back: A verdict",
            "Your reach: editor, documents and notes, at most `safe`. You may open editor, documents \
             and notes when closed (`shell.open_app name=<app>`), whatever your ceiling; inside them \
             you are still held to `safe`. The desktop refuses anything else you try",
            "Your budget: 4 turns and 15 minutes",
            "The task:\nreview the change in ~/src/app",
            "Read this first:\ndiff --git a/x b/x",
        ] {
            assert!(turn.contains(says), "{says:?} missing:\n{turn}");
        }
        assert!(!reviewer.first_turn("x", "  ").contains("Read this first"));
        assert_eq!(reviewer.meta().reach, "editor, documents and notes · at most safe");

        // The Planner is told it may open what it reads; the Coder, the one app among the shell's
        // actions it may use; the Red team, which names nothing, is offered nothing to open.
        let catalog = shipped();
        let turn = |role: &str| catalog.find(role).unwrap().first_turn("do it", "");
        assert!(turn("planner").contains("You may open calendar and notes when closed"), "{}", turn("planner"));
        assert!(turn("coder").contains("You may open editor when closed"), "{}", turn("coder"));
        assert!(!turn("red-team").contains("You may open"), "{}", turn("red-team"));
    }
}
