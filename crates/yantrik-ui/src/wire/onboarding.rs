//! The first run, wired to what is true (design/experience-final-2026-10-10.md, "First run,
//! minute by minute"; the screens are `onboarding_first_run.slint`).
//!
//! What this file decides, and nothing else draws:
//! - **The cards.** A few named cards, each with one plain line and its honest requirement, and
//!   behind them every provider the catalogue knows (`yantrik_ml::KNOWN_PROVIDERS`), never a
//!   shortened list. "On this computer" is a card only when a model is actually here (or the
//!   private-lane backend says one can be downloaded), and it says its size.
//! - **The check rows.** Not checked yet → Checking (started h:mm) → Passed / Needs attention:
//!   <reason>, from what each check returned. No percentage, no step counter pretending to know.
//! - **The model.** Connect account lists the account's models and the person chooses one;
//!   nothing is chosen for them (no catalogue default, no first-in-the-list). That model is
//!   proven with a real answer, saved on the account, and made the one pick for every mind
//!   (`ai_accounts::onboarding::choose_model`, after the catalogue is listed again); a pick that
//!   did not take is said on screen, with Try again.
//! - **The sharing choice.** Share / Private lane / Off, recorded through #675's onboarding calls
//!   (`ai_accounts::onboarding`: `agree_to_share`, `keep_personal_things_on`).
//! - **"Can it answer?"** is asked of the mind itself, through the companion's own chat path.
//!
//! The provider is chosen once: a choice made through these screens in the installer (carried to
//! the new home in `first-run.json`) skips "Who should do the thinking?" and "What your minds
//! share" at first boot. A provider merely present (the `--with-llm` image's own account) does not.
//!
//! Functions not built yet sit behind a small trait with a stub marked `STUB(#…)` that never
//! reports a pass it did not see:
//! - [`Proofs`], the memory and desktop proofs and the list of minds (#677,
//!   feat/677-memory-and-pairing);
//! - [`Lane`], browser sign-in and a downloadable local model (no branch owns them yet).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_ml::{ProviderDescriptor, ProviderKind, KNOWN_PROVIDERS};

use crate::app_context::AppContext;
use crate::wire::provider_catalogue::auth_type_for;
use crate::wire::settings::{ProviderStore, ProviderStoreEntry};
use crate::{App, FirstRunCheck, FirstRunMind, FirstRunModel, FirstRunProvider, TextScale};

// ── The cards ──────────────────────────────────────────────────────────────────────────────

/// The named cards, in the order shown. Everything else is one press away, under "Show every
/// provider".
const FEATURED: [&str; 6] = ["anthropic", "openai", "gemini", "deepseek", "openrouter", "nvidia-nim"];

/// The bundled model the ISO puts on the machine (deploy/yantrik-os/build-debian-iso.sh), served
/// by llama-server on the loopback port below.
const LOCAL_MODEL_DIR: &str = "/opt/yantrik/models/llm";
const LOCAL_ENDPOINT: &str = "http://127.0.0.1:8341/v1";
const THIS_COMPUTER: &str = "this-computer";
/// Settings, and its Harnesses section (wire/harness.rs), where Add another mind goes.
const SETTINGS_SCREEN: i32 = 7;
const HARNESSES_SECTION: i32 = 8;

/// A model on this machine, or one the private-lane backend can download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalModel {
    pub bytes: u64,
    /// On the disk now; otherwise downloadable.
    pub present: bool,
}

/// What a person sees on a card. `Card` → `FirstRunProvider` is a field-for-field copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub line: String,
    pub needs: String,
    pub group: String,
    pub key_placeholder: String,
    pub browser_sign_in: bool,
    pub key_help: bool,
}

/// The name on the card. Anthropic's models are known by their own name.
fn card_name(p: &ProviderDescriptor) -> String {
    match p.id {
        "anthropic" => "Claude".into(),
        _ => p.display_name.into(),
    }
}

/// The one plain line for the named cards; the catalogue's own description for the rest.
fn plain_line(p: &ProviderDescriptor) -> String {
    match p.id {
        "anthropic" => "online, careful with words".into(),
        "openai" => "online, a good all-rounder".into(),
        "gemini" => "online, reads long documents".into(),
        "deepseek" => "online, strong at reasoning, low cost".into(),
        "openrouter" => "online, one account for many models".into(),
        "nvidia-nim" => "online, many open models".into(),
        _ if p.kind == ProviderKind::Local => "on a computer you run it on".into(),
        _ => p.description.trim_end_matches('.').into(),
    }
}

/// The honest requirement. A cloud needs an account with its maker; a runtime the person hosts
/// needs to be running; neither is disguised as effortless.
fn needs(p: &ProviderDescriptor) -> String {
    match p.kind {
        ProviderKind::Local => format!("needs {} running on this computer or your network", p.short_name),
        _ if p.key_placeholder.is_empty() => "needs no key".into(),
        _ => {
            let who = match p.id {
                "anthropic" => "Anthropic",
                "gemini" => "Google",
                "nvidia-nim" => "NVIDIA",
                _ => p.display_name,
            };
            let a = if who.starts_with(['A', 'E', 'I', 'O', 'U']) { "an" } else { "a" };
            format!("needs {a} {who} account")
        }
    }
}

/// Where the provider shows a person their key. Only pages known to exist; a provider without
/// one gets no "Show me where to find it" rather than a guess.
pub fn key_page(id: &str) -> Option<&'static str> {
    Some(match id {
        "anthropic" => "https://console.anthropic.com/settings/keys",
        "openai" => "https://platform.openai.com/api-keys",
        "gemini" => "https://aistudio.google.com/apikey",
        "deepseek" => "https://platform.deepseek.com/api_keys",
        "openrouter" => "https://openrouter.ai/settings/keys",
        "nvidia-nim" => "https://build.nvidia.com/settings/api-keys",
        "groq" => "https://console.groq.com/keys",
        "mistral" => "https://console.mistral.ai/api-keys",
        "huggingface" => "https://huggingface.co/settings/tokens",
        "together" => "https://api.together.ai/settings/api-keys",
        "cohere" => "https://dashboard.cohere.com/api-keys",
        "ollama-cloud" => "https://ollama.com/settings/keys",
        _ => return None,
    })
}

/// Gigabytes, one decimal, the way a person reads a download: "2.5 GB".
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn card(p: &ProviderDescriptor, lane: &dyn Lane) -> Card {
    Card {
        id: p.id.into(),
        name: card_name(p),
        line: plain_line(p),
        needs: needs(p),
        group: match p.kind {
            ProviderKind::Cloud => "cloud",
            ProviderKind::Aggregator => "aggregator",
            ProviderKind::Local => "local",
        }
        .into(),
        key_placeholder: p.key_placeholder.into(),
        browser_sign_in: lane.browser_sign_in(p.id),
        key_help: key_page(p.id).is_some(),
    }
}

/// "On this computer", only for a model that is here or can be fetched, with its size.
pub fn this_computer_card(local: &LocalModel) -> Card {
    Card {
        id: THIS_COMPUTER.into(),
        name: "On this computer".into(),
        line: "private, slower".into(),
        needs: if local.present {
            format!("already here ({})", gb(local.bytes))
        } else {
            format!("downloads {}", gb(local.bytes))
        },
        group: THIS_COMPUTER.into(),
        key_placeholder: String::new(),
        browser_sign_in: false,
        key_help: false,
    }
}

pub fn featured_cards(local: Option<&LocalModel>, lane: &dyn Lane) -> Vec<Card> {
    let mut cards: Vec<Card> = FEATURED
        .iter()
        .filter_map(|id| ProviderDescriptor::by_id(id))
        .map(|p| card(p, lane))
        .collect();
    if let Some(l) = local {
        cards.push(this_computer_card(l));
    }
    cards
}

/// Every provider the catalogue knows, in its order: the full selection, never a simplified one.
pub fn every_card(lane: &dyn Lane) -> Vec<Card> {
    KNOWN_PROVIDERS.iter().filter(|p| !p.id.is_empty()).map(|p| card(p, lane)).collect()
}

fn find_card(id: &str, local: Option<&LocalModel>, lane: &dyn Lane) -> Option<Card> {
    if id == THIS_COMPUTER {
        return local.map(this_computer_card);
    }
    ProviderDescriptor::by_id(id).map(|p| card(p, lane))
}

/// A GGUF in the bundled model directory, by its size on disk.
pub fn local_model_in(dir: &Path) -> Option<LocalModel> {
    let bytes = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "gguf"))
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .max()?;
    Some(LocalModel { bytes, present: true })
}

// ── The check rows ─────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckKind {
    Answer,
    Remember,
    Desktop,
}

impl CheckKind {
    pub const ALL: [CheckKind; 3] = [CheckKind::Answer, CheckKind::Remember, CheckKind::Desktop];
    pub fn id(self) -> &'static str {
        match self {
            CheckKind::Answer => "answer",
            CheckKind::Remember => "remember",
            CheckKind::Desktop => "desktop",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            CheckKind::Answer => "Can it answer?",
            CheckKind::Remember => "Can it remember?",
            CheckKind::Desktop => "Can it reach the desktop?",
        }
    }
}

/// What a check found: the proof for Details, or the plain reason it needs attention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proof {
    Passed { details: String },
    Attention { reason: String, details: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckState {
    NotChecked,
    /// Started at this wall-clock time, "2:16".
    Checking(String),
    Done(Proof),
}

impl CheckState {
    pub fn state(&self) -> &'static str {
        match self {
            CheckState::NotChecked => "not-checked",
            CheckState::Checking(_) => "checking",
            CheckState::Done(Proof::Passed { .. }) => "passed",
            CheckState::Done(Proof::Attention { .. }) => "attention",
        }
    }
    pub fn words(&self) -> String {
        match self {
            CheckState::NotChecked => "Not checked yet".into(),
            CheckState::Checking(at) => format!("Checking (started {at})"),
            CheckState::Done(Proof::Passed { .. }) => "Passed".into(),
            CheckState::Done(Proof::Attention { reason, .. }) => format!("Needs attention: {reason}"),
        }
    }
    fn details(&self) -> String {
        match self {
            CheckState::Done(Proof::Passed { details } | Proof::Attention { details, .. }) => details.clone(),
            _ => String::new(),
        }
    }
}

/// "2:16", the clock a person reads: hour without a leading zero, no seconds.
pub fn clock(t: chrono::DateTime<chrono::Local>) -> String {
    t.format("%-I:%M").to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MindRows {
    pub id: String,
    pub name: String,
    pub checks: Vec<(CheckKind, CheckState)>,
}

impl MindRows {
    pub fn new(id: &str, name: &str) -> Self {
        Self { id: id.into(), name: name.into(), checks: CheckKind::ALL.iter().map(|k| (*k, CheckState::NotChecked)).collect() }
    }
    fn done(&self) -> bool {
        self.checks.iter().all(|(_, s)| matches!(s, CheckState::Done(_)))
    }
    fn ok(&self) -> bool {
        self.checks.iter().all(|(_, s)| matches!(s, CheckState::Done(Proof::Passed { .. })))
    }
    /// The one line on "Your minds": what was seen, never what was assumed.
    pub fn words(&self) -> String {
        if self.ok() {
            return "Ready".into();
        }
        if let Some(r) = self.checks.iter().find_map(|(_, s)| match s {
            CheckState::Done(Proof::Attention { reason, .. }) => Some(reason.clone()),
            _ => None,
        }) {
            return format!("Needs attention: {r}");
        }
        if self.checks.iter().any(|(_, s)| matches!(s, CheckState::Checking(_))) {
            return "Checking".into();
        }
        "Not checked yet".into()
    }
    fn set(&mut self, kind: CheckKind, state: CheckState) {
        if let Some(slot) = self.checks.iter_mut().find(|(k, _)| *k == kind) {
            slot.1 = state;
        }
    }
    fn to_ui(&self) -> FirstRunMind {
        let checks: Vec<FirstRunCheck> = self
            .checks
            .iter()
            .map(|(k, s)| FirstRunCheck {
                id: k.id().into(),
                label: k.label().into(),
                state: s.state().into(),
                words: s.words().into(),
                details: s.details().into(),
            })
            .collect();
        FirstRunMind {
            id: self.id.clone().into(),
            name: self.name.clone().into(),
            words: self.words().into(),
            ok: self.ok(),
            checks: ModelRc::new(VecModel::from(checks)),
        }
    }
}

// ── What the minds share ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sharing {
    /// The person's context goes to the chosen provider with their consent.
    Share,
    /// Calls that carry it go to a model on this machine instead.
    PrivateLane,
    /// Neither: such calls are refused, with the fix named. Nothing is sent quietly.
    Off,
}

impl Sharing {
    pub fn parse(s: &str) -> Option<Sharing> {
        match s {
            "share" => Some(Sharing::Share),
            "private-lane" => Some(Sharing::PrivateLane),
            "off" => Some(Sharing::Off),
            _ => None,
        }
    }
}

/// "What's shared?", in plain words.
pub const SHARED_ITEMS: [&str; 4] = [
    "Your name",
    "Your town",
    "The app you're in",
    "What your minds remember about you",
];

/// The private-lane offer under the sharing sentence: its size, and that answers may be slower.
/// Empty when there is no model here and none can be fetched; the screen then says so.
pub fn private_lane_words(local: Option<&LocalModel>) -> String {
    match local {
        Some(l) if l.present => format!("Uses the {} model already on this computer. Answers about you may be slower.", gb(l.bytes)),
        Some(l) => format!("Downloads a {} model to this computer. Answers about you may be slower.", gb(l.bytes)),
        None => String::new(),
    }
}

/// The AI-accounts id of a saved provider: its name as a slug, as `ai_accounts::account` makes it.
pub fn account_id(entry: &ProviderStoreEntry) -> String {
    let s = crate::ai_accounts::account::slug(&entry.name);
    if s.is_empty() { crate::ai_accounts::account::slug(&entry.provider_type) } else { s }
}

/// The local model that keeps personal things here: the bundled one when it is listed (it is on
/// this machine, not only the network), else the first local model.
pub fn lane_model(choices: &[(String, String)]) -> Option<String> {
    choices
        .iter()
        .find(|(_, url)| url.contains("127.0.0.1") || url.contains("localhost"))
        .or_else(|| choices.first())
        .map(|(id, _)| id.clone())
}

/// Share / Private lane / Off, through #675's onboarding calls. A private lane with no local
/// model to keep it is refused with the reason, and sharing stays off: nothing is sent quietly.
fn apply_sharing(sharing: Sharing) -> Result<(), String> {
    use crate::ai_accounts::onboarding as ob;
    match sharing {
        Sharing::Share => ob::agree_to_share(true),
        Sharing::Off => ob::agree_to_share(false),
        Sharing::PrivateLane => {
            ob::agree_to_share(false)?;
            let accounts = crate::ai_accounts::accounts();
            let choices: Vec<(String, String)> = ob::private_lane_choices()
                .into_iter()
                .map(|m| {
                    let url = accounts.iter().find(|a| a.id == m.account).map(|a| a.base_url.clone()).unwrap_or_default();
                    (m.id, url)
                })
                .collect();
            let lane = lane_model(&choices).ok_or("no model on this computer is ready to keep personal things yet")?;
            ob::keep_personal_things_on(&lane)
        }
    }
}

// ── What other branches provide ───────────────────────────────────────────────────────────

/// The minds and their three proofs (#677: memory write-then-recall, an attach seen on the
/// harness socket). The answer proof is ours: the mind is asked, through its own chat path.
pub trait Proofs: Send + Sync {
    /// Every mind to check, `(id, name)`.
    fn minds(&self) -> Vec<(String, String)>;
    /// Write a temporary note to the mind's memory, read it back, delete it.
    fn remember(&self, mind: &str) -> Proof;
    /// See the mind attached to the desktop's harness socket.
    fn desktop(&self, mind: &str) -> Proof;
}

/// STUB(#677): replace with the three-proofs function from feat/677-memory-and-pairing. It never
/// reports a pass: until the proof exists, the row says it could not be checked, and why.
pub struct PendingProofs;

impl Proofs for PendingProofs {
    fn minds(&self) -> Vec<(String, String)> {
        // The built-in companion is the one mind every install has; the harness minds come with
        // #677's list.
        vec![("companion".into(), "Yantrik Mind".into())]
    }
    fn remember(&self, _mind: &str) -> Proof {
        Proof::Attention {
            reason: "this build cannot check memory yet".into(),
            details: "memory proof not built in (waiting on #677: write a note, recall it, delete it)".into(),
        }
    }
    fn desktop(&self, _mind: &str) -> Proof {
        Proof::Attention {
            reason: "this build cannot check the desktop link yet".into(),
            details: "pairing proof not built in (waiting on #677: an attach seen on the harness socket)".into(),
        }
    }
}

/// Browser sign-in and a downloadable local model: AI accounts' backend, not built yet.
pub trait Lane: Send + Sync {
    /// The provider lets a person sign in with the browser instead of pasting a key.
    fn browser_sign_in(&self, provider: &str) -> bool;
    /// Sign in with the browser; the key arrives in AI accounts. Returns the plain outcome.
    fn sign_in(&self, provider: &str) -> Result<(), String>;
    /// A model the private lane can download, when none is on the machine.
    fn downloadable(&self) -> Option<LocalModel>;
}

/// STUB(#675 follow-up): no browser sign-in and nothing to download. Every answer here is the
/// honest "not available", so no card offers what cannot happen.
pub struct PendingLane;

impl Lane for PendingLane {
    fn browser_sign_in(&self, _provider: &str) -> bool {
        false
    }
    fn sign_in(&self, _provider: &str) -> Result<(), String> {
        Err("browser sign-in is not available in this build; paste a key instead".into())
    }
    fn downloadable(&self) -> Option<LocalModel> {
        None
    }
}

// ── What the first run remembers ──────────────────────────────────────────────────────────

/// `~/.config/yantrik/first-run.json`: the text size and the sharing choice. No key.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Saved {
    pub text_scale: f32,
    /// "share", "private-lane", "off" or "local", for #675 to read when it routes.
    pub sharing: String,
    // Not kept yet: whether the one-time "Hold What really happened" hint was shown. The 3:45
    // "Recorded … Show me" footer and that hint wait on a receipt mechanism that does not exist
    // yet; nothing here pretends to record one.
    /// A provider was chosen and answered through these screens (in the installer or at first
    /// boot): the first boot does not ask again.
    pub provider_chosen: bool,
}

pub fn saved_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config/yantrik/first-run.json")
}

pub fn load_saved(path: &Path) -> Saved {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn update_saved(path: &Path, f: impl FnOnce(&mut Saved)) {
    let mut s = load_saved(path);
    f(&mut s);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(&s) {
        if let Err(e) = std::fs::write(path, text + "\n") {
            tracing::warn!(error = %e, "first run: could not save the choice");
        }
    }
}

// ── The wiring ─────────────────────────────────────────────────────────────────────────────

fn to_ui(c: &Card) -> FirstRunProvider {
    FirstRunProvider {
        id: c.id.clone().into(),
        name: c.name.clone().into(),
        line: c.line.clone().into(),
        needs: c.needs.clone().into(),
        group: c.group.clone().into(),
        key_placeholder: c.key_placeholder.clone().into(),
        browser_sign_in: c.browser_sign_in,
        key_help: c.key_help,
    }
}

fn model_of<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

/// Save `entry` as the one primary provider. An earlier primary steps aside but is kept: merely
/// looking at a card (a runtime the person hosts connects at once) or a refused key must not
/// delete an account that worked, key and all.
fn save_primary(entry: ProviderStoreEntry) -> Result<(), String> {
    let mut store = ProviderStore::load();
    store.entries.retain(|e| e.id != entry.id);
    for e in &mut store.entries {
        e.is_primary = false;
    }
    store.entries.push(entry);
    store.save()
}

/// The companion's bridge, for putting it on the account just proven.
static BRIDGE: std::sync::OnceLock<Arc<crate::bridge::CompanionBridge>> = std::sync::OnceLock::new();

/// The account being connected, key and all, between "Connect account" and the person choosing
/// its model. In memory only: nothing is saved until the chosen model has answered.
static PENDING: Mutex<Option<ProviderStoreEntry>> = Mutex::new(None);

/// One model the account lists, as the model step shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    /// What the provider calls it, sent upstream as `model`.
    pub id: String,
    pub name: String,
}

/// The models an account offers, from the same source the catalogue lists them from: a hand-kept
/// list for a provider with none of its own, else the provider's own answer. A failed listing is
/// the plain reason. Nothing here chooses one: no catalogue default, no first-in-the-list.
pub(crate) fn model_choices(
    account: &crate::ai_accounts::account::Account,
    listed: Option<Result<Vec<crate::wire::provider_models::ListedModel>, crate::wire::provider_models::ListError>>,
) -> Result<Vec<ModelChoice>, String> {
    use crate::ai_accounts::models::{build, Listing};
    let free = yantrik_ml::provider::pool::tiers::FREE_TIERS;
    let built = build(account, free, listed);
    if let Listing::Failed(why) = &built.listing {
        return Err(why.trim_end_matches('.').to_string());
    }
    if built.models.is_empty() {
        return Err("it answered, but lists no model to talk to".into());
    }
    Ok(built.models.into_iter().map(|m| ModelChoice { id: m.model, name: m.name }).collect())
}

/// Ask the account for its models (off the UI thread).
fn list_choices(entry: &ProviderStoreEntry) -> Result<Vec<ModelChoice>, String> {
    let account = crate::ai_accounts::account::accounts(std::slice::from_ref(entry), &[], &Default::default(), &Default::default())
        .into_iter()
        .next()
        .ok_or("there is no address for it")?;
    let free = yantrik_ml::provider::pool::tiers::FREE_TIERS;
    let listed = if crate::ai_accounts::models::needs_no_listing(&account, free) {
        None
    } else {
        Some(crate::wire::provider_models::list_models(&entry.base_url, entry.api_key.as_deref(), &entry.auth_type))
    };
    model_choices(&account, listed)
}

/// Proven with the person's model: keep it on the saved account (so the next start runs on the
/// model they chose) and put the companion on it now, so the first success in the chat is
/// answered by what was just checked rather than "No mind can think yet" until a restart. A
/// companion on Yantrik models (companion-models.json) stays there.
fn adopt(entry: &ProviderStoreEntry) -> Result<(), String> {
    save_primary(entry.clone())?;
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    if !home.join(".config/yantrik/companion-models.json").exists() {
        if let Some(bridge) = BRIDGE.get() {
            crate::wire::provider_panel::reload_primary(bridge, entry);
        }
    }
    Ok(())
}

/// The smallest real answer from the account, with the model the person chose: a one-token call.
fn prove_answer(entry: &ProviderStoreEntry) -> Result<i32, String> {
    use crate::wire::provider_models::{check_key, KeyCheck};
    if entry.model.is_empty() {
        return Err("no model is chosen yet".into());
    }
    let started = std::time::Instant::now();
    match check_key(&entry.base_url, entry.api_key.as_deref().unwrap_or(""), &entry.model) {
        KeyCheck::Works => Ok(started.elapsed().as_millis() as i32),
        KeyCheck::Refused => Err("the key was refused".into()),
        KeyCheck::Unconfirmed(why) => Err(why.trim_end_matches('.').to_string()),
    }
}

/// The AI-accounts id the catalogue gave `entry` once saved: its own key entry, else its address
/// and model; the slug of its name when the catalogue has not listed it yet.
fn catalogue_account(entry: &ProviderStoreEntry) -> String {
    use crate::ai_accounts::account::KeyRef;
    let base = crate::wire::provider_models::openai_base(&entry.base_url);
    crate::ai_accounts::accounts()
        .into_iter()
        .find(|a| matches!(&a.key, KeyRef::Saved { entry: id } if *id == entry.id))
        .or_else(|| crate::ai_accounts::accounts().into_iter().find(|a| a.base_url == base && a.saved_model == entry.model))
        .map(|a| a.id)
        .unwrap_or_else(|| account_id(entry))
}

/// Make the person's model the one pick for every mind (#675): the catalogue is listed again
/// first, so the account added a moment ago is in it. The plain reason when it did not take.
fn make_the_pick(entry: &ProviderStoreEntry) -> Result<(), String> {
    if let Some(bridge) = BRIDGE.get() {
        crate::ai_accounts::refresh_now(bridge);
    }
    let id = crate::ai_accounts::models::gateway_id(&catalogue_account(entry), &entry.model);
    crate::ai_accounts::onboarding::choose_model(&id)
}

struct State {
    rows: Vec<MindRows>,
    local: Option<LocalModel>,
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let _ = BRIDGE.set(ctx.bridge.clone());
    let proofs: Arc<dyn Proofs> = Arc::new(PendingProofs);
    let lane: Arc<dyn Lane> = Arc::new(PendingLane);
    let saved = saved_path();

    // The text size the person chose, on every later start too.
    let s = load_saved(&saved).text_scale;
    if s >= 1.0 {
        ui.global::<TextScale>().set_factor(s);
    }

    let local = local_model_in(Path::new(LOCAL_MODEL_DIR)).or_else(|| lane.downloadable());
    let state = Arc::new(Mutex::new(State {
        rows: proofs.minds().iter().map(|(id, n)| MindRows::new(id, n)).collect(),
        local: local.clone(),
    }));

    ui.set_fr_featured(model_of(featured_cards(local.as_ref(), lane.as_ref()).iter().map(to_ui).collect()));
    ui.set_fr_every_provider(model_of(every_card(lane.as_ref()).iter().map(to_ui).collect()));
    ui.set_fr_shared_items(model_of(SHARED_ITEMS.iter().map(|s| SharedString::from(*s)).collect()));
    let lane_listed = !crate::ai_accounts::onboarding::private_lane_choices().is_empty();
    ui.set_fr_private_lane_words(match (&local, lane_listed) {
        (Some(_), _) => private_lane_words(local.as_ref()),
        (None, true) => "Uses a model on this computer or your network. Answers about you may be slower.".into(),
        (None, false) => String::new(),
    }.into());
    ui.set_fr_minds(model_of(lock(&state).rows.iter().map(MindRows::to_ui).collect()));
    // A first boot the installer did not set up has no name for the person: ask it, once, so
    // the minds do not call them "User". Nothing else (interests, place, the companion's name).
    ui.set_fr_ask_name(asks_name(
        crate::onboarding::after_install_marker_path().exists(),
        &ui.get_settings_user_name(),
    ));
    // Chosen once: a provider already saved (the installer carries the live session's choice
    // into this account's providers.yaml) skips the two AI screens.
    if let Some(p) = ProviderStore::load().primary().filter(|_| load_saved(&saved).provider_chosen) {
        let name = card_name_for(&p.provider_type, &p.name);
        ui.set_fr_installer_chose(true);
        ui.set_fr_chosen_name(name.clone().into());
        ui.set_fr_installer_choice(name.into());
    }

    // Back from the provider screens the installer drew: its "Thinking" row says what was chosen
    // (and connected), and the install carries it over (wire/installer.rs).
    ui.on_fr_installer_done({
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let chosen = ui.get_fr_connect_status() == "connected";
            let name = match ProviderStore::load().primary() {
                Some(p) if chosen => card_name_for(&p.provider_type, &p.name),
                _ => String::new(),
            };
            ui.set_fr_installer_choice(name.into());
        }
    });

    ui.on_fr_text_scale_changed({
        let saved = saved.clone();
        move |f| update_saved(&saved, |s| s.text_scale = f)
    });

    ui.on_fr_read_aloud(|text| speak(text.to_string()));

    ui.on_fr_choose_provider({
        let weak = ui.as_weak();
        let lane = lane.clone();
        let state = state.clone();
        move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let local = lock(&state).local.clone();
            let Some(c) = find_card(&id, local.as_ref(), lane.as_ref()) else { return };
            ui.set_fr_chosen_id(c.id.clone().into());
            ui.set_fr_chosen_name(c.name.clone().into());
            ui.set_fr_chosen_key_placeholder(c.key_placeholder.clone().into());
            ui.set_fr_chosen_browser_sign_in(c.browser_sign_in);
            ui.set_fr_chosen_key_help(c.key_help);
            ui.set_fr_key_input("".into());
            ui.set_fr_connect_status("".into());
            ui.set_fr_connect_words("".into());
            ui.set_fr_models(model_of(Vec::new()));
            ui.set_fr_chosen_model("".into());
            *lock(&PENDING) = None;
            // An address with a part only the person knows (Cloudflare's {account_id}): Advanced,
            // with the address to fill in, rather than a Connect that can only fail.
            if let Some(template) = needs_filling(&c.id) {
                ui.set_fr_custom_url(template.into());
                ui.set_fr_custom_model("".into());
                ui.set_fr_custom_key("".into());
                ui.set_fr_connect_mode("advanced".into());
                return;
            }
            let mode = if c.id == THIS_COMPUTER {
                "this-computer"
            } else if c.browser_sign_in {
                "browser"
            } else if c.key_placeholder.is_empty() {
                // A runtime the person hosts: no key, so connect straight away.
                connect(&ui, ProviderStoreEntry {
                    id: format!("{}-onboarding", c.id),
                    name: crate::wire::provider_catalogue::provider_preset(&c.id).0.into(),
                    provider_type: c.id.clone(),
                    base_url: crate::wire::provider_catalogue::provider_preset(&c.id).1.into(),
                    api_key: None,
                    auth_type: auth_type_for(&c.id).into(),
                    is_primary: true,
                    is_fallback: false,
                    model: String::new(),
                });
                "local"
            } else {
                "key"
            };
            ui.set_fr_connect_mode(mode.into());
        }
    });

    ui.on_fr_show_key_help(|id| {
        if let Some(url) = key_page(&id) {
            // A page where a key is made: the browser apart from the desktop's own.
            if let Err(e) = crate::open_url::open_apart(url) {
                tracing::warn!(error = %e, "first run: could not open the key page");
            }
        }
    });

    ui.on_fr_sign_in_browser({
        let weak = ui.as_weak();
        let lane = lane.clone();
        move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if let Err(why) = lane.sign_in(&id) {
                ui.set_fr_connect_status("attention".into());
                ui.set_fr_connect_words(format!("Needs attention: {why}").into());
                ui.set_fr_connect_mode("key".into());
            }
        }
    });

    ui.on_fr_submit_key({
        let weak = ui.as_weak();
        move |id, key| {
            let Some(ui) = weak.upgrade() else { return };
            let (name, url) = crate::wire::provider_catalogue::provider_preset(&id);
            if url.is_empty() || key.trim().is_empty() {
                return;
            }
            connect(&ui, ProviderStoreEntry {
                id: format!("{id}-onboarding"),
                name: name.into(),
                provider_type: id.to_string(),
                base_url: url.into(),
                api_key: Some(key.trim().to_string()),
                auth_type: auth_type_for(&id).into(),
                is_primary: true,
                is_fallback: false,
                model: String::new(),
            });
        }
    });

    ui.on_fr_submit_custom({
        let weak = ui.as_weak();
        move |url, model, key| {
            let Some(ui) = weak.upgrade() else { return };
            let key = key.trim().to_string();
            let url = url.trim().to_string();
            if url.contains('{') {
                ui.set_fr_connect_status("attention".into());
                ui.set_fr_connect_words("Needs attention: fill in the part of the address in braces first".into());
                return;
            }
            // Advanced for a known provider (one whose address needed filling in) keeps its name
            // and how it signs requests; otherwise it is the person's own endpoint.
            let chosen = ui.get_fr_chosen_id().to_string();
            let known = ProviderDescriptor::by_id(&chosen).map(|p| p.id.to_string());
            let (id, name, provider_type, auth) = match &known {
                Some(p) => (format!("{p}-onboarding"), crate::wire::provider_catalogue::provider_preset(p).0.to_string(), p.clone(), auth_type_for(p).to_string()),
                None => ("custom-onboarding".into(), "Custom".into(), "custom".into(), "bearer".into()),
            };
            connect(&ui, ProviderStoreEntry {
                id,
                name,
                provider_type,
                base_url: url,
                auth_type: if key.is_empty() { "none".into() } else { auth },
                api_key: if key.is_empty() { None } else { Some(key) },
                is_primary: true,
                is_fallback: false,
                // The person typed it: this one is theirs.
                model: model.trim().to_string(),
            });
        }
    });

    ui.on_fr_pick_model({
        let weak = ui.as_weak();
        move |model| {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_fr_connect_status() == "checking" {
                return;
            }
            let Some(entry) = lock(&PENDING).as_mut().map(|e| {
                e.model = model.to_string();
                e.clone()
            }) else {
                return;
            };
            ui.set_fr_chosen_model(entry.model.into());
            prove_and_pick(&ui);
        }
    });

    ui.on_fr_retry_pick({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                retry_pick(&ui);
            }
        }
    });

    ui.on_fr_use_this_computer({
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            // The image's own account for the bundled model, when it wrote one; else a new one.
            // Its model is the one the person chooses from what the runtime lists, not whatever the
            // image wrote down.
            if let Some(mut e) = ProviderStore::load().entries.into_iter().find(|e| e.base_url.trim_end_matches('/') == LOCAL_ENDPOINT) {
                e.is_primary = true;
                e.model.clear();
                connect(&ui, e);
                return;
            }
            connect(&ui, ProviderStoreEntry {
                id: "llamacpp-onboarding".into(),
                name: "On this computer".into(),
                provider_type: "llamacpp".into(),
                base_url: LOCAL_ENDPOINT.into(),
                api_key: None,
                auth_type: "none".into(),
                is_primary: true,
                is_fallback: false,
                model: String::new(),
            });
        }
    });

    ui.on_fr_sharing_chosen({
        let weak = ui.as_weak();
        let saved = saved.clone();
        move |mode| {
            if let Some(ui) = weak.upgrade() {
                ui.set_fr_sharing_words("".into());
            }
            let Some(sharing) = Sharing::parse(&mode) else {
                update_saved(&saved, |s| s.sharing = mode.to_string());
                return;
            };
            match apply_sharing(sharing) {
                Ok(()) => update_saved(&saved, |s| s.sharing = mode.to_string()),
                Err(e) => {
                    tracing::warn!(error = %e, "first run: the sharing choice was not saved");
                    let words = if sharing == Sharing::PrivateLane {
                        // apply_sharing turned sharing off before the lane refused: record what is true.
                        update_saved(&saved, |s| s.sharing = "off".into());
                        format!("Needs attention: personal things are not shared, and not kept on this computer either: {e}. You can choose again in Settings → AI accounts.")
                    } else {
                        format!("Needs attention: the sharing choice was not saved ({e})")
                    };
                    if let Some(ui) = weak.upgrade() {
                        ui.set_fr_sharing_words(words.into());
                    }
                }
            }
        }
    });

    ui.on_fr_start_checks({
        let weak = ui.as_weak();
        let state = state.clone();
        let proofs = proofs.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_fr_checks_running() {
                return;
            }
            start_checks(&ui, state.clone(), proofs.clone());
        }
    });

    ui.on_fr_add_mind({
        let weak = ui.as_weak();
        move || {
            // Settings → Harnesses, where every mind installs with the model and memory chosen
            // here; the first run is done once a mind can be added there.
            let Some(ui) = weak.upgrade() else { return };
            crate::onboarding::write_marker();
            ui.set_settings_category(HARNESSES_SECTION);
            ui.set_current_screen(SETTINGS_SCREEN);
            ui.invoke_navigate(SETTINGS_SCREEN);
        }
    });

    ui.on_fr_finished({
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            crate::onboarding::write_marker();
            // The first success happens in the chat beside the desktop.
            ui.set_fr_chat(1);
            ui.set_fr_chat_outcome("".into());
            ui.set_fr_chat_failure("".into());
            ui.set_lens_open(true);
            tracing::info!("first run: done, chat opened for the first success");
        }
    });

    ui.on_fr_chat_retry({
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let question = lock(&FIRST_QUESTION).clone();
            if !question.is_empty() {
                ui.invoke_send_message(question.into());
            }
        }
    });

    ui.on_fr_chat_done({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_fr_chat(0);
            }
        }
    });
}

/// Whether Welcome asks the person's name: not when the installer made the account (it asked
/// already), and not when a name is already kept. "User" is the config's placeholder, not a name.
pub fn asks_name(after_install: bool, kept: &str) -> bool {
    let kept = kept.trim();
    !after_install && (kept.is_empty() || kept == "User")
}

/// The address to fill in, for a provider whose address has a part only the person knows.
pub fn needs_filling(id: &str) -> Option<&'static str> {
    ProviderDescriptor::by_id(id).filter(|p| p.url_needs_filling()).map(|p| p.openai_base_url())
}

fn card_name_for(provider_type: &str, saved_name: &str) -> String {
    match ProviderDescriptor::by_id(provider_type) {
        Some(p) => card_name(p),
        None => saved_name.to_string(),
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Connect account: ask the account for its models, off the UI thread, and show them for the
/// person to choose from. Nothing is chosen for them and nothing is saved yet. A model they
/// typed themselves (Advanced) is theirs already, and goes straight to the proof.
fn connect(ui: &App, entry: ProviderStoreEntry) {
    if entry.base_url.is_empty() {
        ui.set_fr_connect_status("attention".into());
        ui.set_fr_connect_words("Needs attention: there is no address for it".into());
        return;
    }
    ui.set_fr_models(model_of(Vec::new()));
    ui.set_fr_chosen_model("".into());
    if !entry.model.is_empty() {
        *lock(&PENDING) = Some(entry);
        prove_and_pick(ui);
        return;
    }
    ui.set_fr_connect_status("checking".into());
    ui.set_fr_connect_words(format!("Checking (started {})", clock(chrono::Local::now())).into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let listed = list_choices(&entry);
        if listed.is_ok() {
            *lock(&PENDING) = Some(entry);
        }
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match listed {
                Ok(models) => {
                    let n = models.len();
                    ui.set_fr_models(model_of(models.into_iter().map(|m| FirstRunModel { id: m.id.into(), name: m.name.into() }).collect()));
                    ui.set_fr_connect_status("choose-model".into());
                    ui.set_fr_connect_words(
                        if n == 1 { "It lists one model. Choose it to go on.".to_string() } else { format!("It lists {n} models. Choose one.") }.into(),
                    );
                }
                Err(why) => {
                    ui.set_fr_connect_status("attention".into());
                    ui.set_fr_connect_words(format!("Needs attention: {why}").into());
                }
            }
        });
    });
}

/// The person chose `model`: prove it answers, save the account with it, put the companion on
/// it, and make it the one pick for every mind. "Connected to Claude." only after all of that;
/// a pick that did not take is said, with Try again.
fn prove_and_pick(ui: &App) {
    let Some(entry) = lock(&PENDING).clone() else { return };
    let name = ui.get_fr_chosen_name().to_string();
    ui.set_fr_connect_status("checking".into());
    ui.set_fr_connect_words(format!("Checking (started {})", clock(chrono::Local::now())).into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        // Saved only once it has answered: a refused key or a runtime that is not running
        // leaves nothing behind, and the account that worked before stays the primary.
        let result = prove_answer(&entry).and_then(|_| adopt(&entry).map_err(|e| format!("it answered, but could not be saved ({e})")));
        let picked = match &result {
            Ok(()) => {
                update_saved(&saved_path(), |s| s.provider_chosen = true);
                make_the_pick(&entry)
            }
            Err(_) => Ok(()),
        };
        if let Err(e) = &picked {
            tracing::warn!(error = %e, model = %entry.model, "first run: the model was not made the pick");
        }
        let model = entry.model.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match (result, picked) {
                (Ok(()), Ok(())) => {
                    ui.set_fr_connect_status("connected".into());
                    ui.set_fr_connect_words(format!("Connected to {name}. Your minds will use {model}.").into());
                }
                (Ok(()), Err(why)) => {
                    ui.set_fr_connect_status("pick-failed".into());
                    ui.set_fr_connect_words(
                        format!("Needs attention: {model} answered, but your minds could not be set to use it ({why})").into(),
                    );
                }
                (Err(why), _) => {
                    ui.set_fr_connect_status("attention".into());
                    ui.set_fr_connect_words(format!("Needs attention: {why}").into());
                }
            }
        });
    });
}

/// Try again after a pick that did not take: the account is saved and answered, so only the
/// pick is made again.
fn retry_pick(ui: &App) {
    let Some(entry) = lock(&PENDING).clone() else { return };
    let name = ui.get_fr_chosen_name().to_string();
    ui.set_fr_connect_status("checking".into());
    ui.set_fr_connect_words(format!("Checking (started {})", clock(chrono::Local::now())).into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let picked = make_the_pick(&entry);
        let model = entry.model.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match picked {
                Ok(()) => {
                    ui.set_fr_connect_status("connected".into());
                    ui.set_fr_connect_words(format!("Connected to {name}. Your minds will use {model}.").into());
                }
                Err(why) => {
                    ui.set_fr_connect_status("pick-failed".into());
                    ui.set_fr_connect_words(
                        format!("Needs attention: {model} answered, but your minds could not be set to use it ({why})").into(),
                    );
                }
            }
        });
    });
}

// ── The first success, in the chat ─────────────────────────────────────────────────────────

/// What the chat says when the first reply came from the desktop, not a model.
pub const NOT_SET_UP_SENTENCE: &str =
    "Yantrik Mind did not answer: it is not set up to think yet. Choose who does the thinking in Settings \u{2192} AI, then try again.";
/// What the chat says when the first reply failed or did not come.
pub const NO_ANSWER_SENTENCE: &str = "Yantrik Mind did not answer this time.";

/// The first-run question, for Try again.
static FIRST_QUESTION: Mutex<String> = Mutex::new(String::new());

/// The person asked something in the chat: while the first run's chat is open, it is the question
/// whose answer decides "You're ready.", and anything said about an earlier one is cleared.
/// On the UI thread.
pub fn first_chat_asked(weak: &slint::Weak<App>, text: &str) {
    let Some(ui) = weak.upgrade() else { return };
    if ui.get_fr_chat() == 0 {
        return;
    }
    *lock(&FIRST_QUESTION) = text.to_string();
    ui.set_fr_chat_outcome("".into());
    ui.set_fr_chat_failure("".into());
}

/// A chat turn ended: `Ok` only for a model's words. While the first run's chat is open, this
/// is what lets "You're ready." show, or keeps the person there with the plain sentence and
/// Try again. Any thread.
pub fn first_chat_ended(weak: &slint::Weak<App>, outcome: Result<(), String>) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_fr_chat() == 0 {
            return;
        }
        match outcome {
            Ok(()) => {
                ui.set_fr_chat_outcome("answered".into());
                ui.set_fr_chat_failure("".into());
            }
            Err(why) => {
                tracing::info!(why = %why, "first run: the first reply was not an answer");
                ui.set_fr_chat_outcome("failed".into());
                ui.set_fr_chat_failure(why.into());
            }
        }
    });
}

fn push_rows(ui: &App, state: &Mutex<State>) {
    let st = lock(state);
    ui.set_fr_minds(model_of(st.rows.iter().map(MindRows::to_ui).collect()));
    let done = st.rows.iter().all(MindRows::done);
    ui.set_fr_checks_done(done);
    ui.set_fr_checks_running(!done);
}

/// Run the three checks for every mind, one row at a time, each row saying when it started.
fn start_checks(ui: &App, state: Arc<Mutex<State>>, proofs: Arc<dyn Proofs>) {
    {
        let mut st = lock(&state);
        st.rows = proofs.minds().iter().map(|(id, n)| MindRows::new(id, n)).collect();
    }
    ui.set_fr_checks_running(true);
    ui.set_fr_checks_done(false);
    push_rows(ui, &state);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let minds: Vec<String> = lock(&state).rows.iter().map(|r| r.id.clone()).collect();
        for mind in minds {
            for kind in CheckKind::ALL {
                set_row(&weak, &state, &mind, kind, CheckState::Checking(clock(chrono::Local::now())));
                let proof = match kind {
                    CheckKind::Answer => ask_the_mind(&mind),
                    CheckKind::Remember => proofs.remember(&mind),
                    CheckKind::Desktop => proofs.desktop(&mind),
                };
                set_row(&weak, &state, &mind, kind, CheckState::Done(proof));
            }
        }
    });
}

/// "Can it answer?", asked of the mind itself: the companion's own chat path, on whatever it now
/// runs on (the account just connected, or Yantrik models), not the account behind it. Only a
/// model's words pass; the not-set-up reply and a failed turn are the plain reason.
fn ask_the_mind(mind: &str) -> Proof {
    if mind != "companion" {
        return Proof::Attention {
            reason: "this build cannot ask this mind yet".into(),
            details: "only Yantrik Mind can be asked from the first run (the other minds come with #677)".into(),
        };
    }
    let Some(bridge) = BRIDGE.get() else {
        return Proof::Attention { reason: "Yantrik Mind is not running".into(), details: "no companion in this process".into() };
    };
    let started = std::time::Instant::now();
    let said = bridge.handle().ask("Reply with the single word OK.".into(), std::time::Duration::from_secs(90));
    answer_proof(said, started.elapsed().as_millis())
}

/// What the mind's reply proves.
pub fn answer_proof(said: Result<String, crate::bridge::AskError>, ms: u128) -> Proof {
    use crate::bridge::AskError;
    match said {
        Ok(words) if !words.trim().is_empty() => {
            let words: String = words.trim().chars().take(60).collect();
            Proof::Passed { details: format!("Yantrik Mind answered in {ms} ms: \u{201c}{words}\u{201d}") }
        }
        Ok(_) => Proof::Attention { reason: "it answered with nothing".into(), details: format!("an empty reply after {ms} ms") },
        Err(AskError::NoModel) => Proof::Attention {
            reason: "it is not set up to think yet".into(),
            details: "the reply came from the desktop, not from a model".into(),
        },
        Err(AskError::Failed(why)) => {
            let why = why.trim().trim_end_matches('.').to_string();
            Proof::Attention { reason: if why.is_empty() { "it did not answer".into() } else { why.clone() }, details: why }
        }
    }
}

fn set_row(weak: &slint::Weak<App>, state: &Arc<Mutex<State>>, mind: &str, kind: CheckKind, s: CheckState) {
    if let Some(r) = lock(state).rows.iter_mut().find(|r| r.id == mind) {
        r.set(kind, s);
    }
    let weak = weak.clone();
    let state = state.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            push_rows(&ui, &state);
        }
    });
}

/// Read this to me: the screen's words through the same voices the voice runtime uses (Piper,
/// else the system's), on a thread of its own so the screen never waits on it.
fn speak(text: String) {
    std::thread::spawn(move || {
        let piper = yantrik_ml::PiperTTS::from_dir(Path::new("/opt/yantrik/models/tts"));
        let result = match piper {
            Ok(p) => p.speak(&text, None),
            Err(_) => yantrik_ml::TTSEngine::new().and_then(|t| t.speak(&text, None)),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "first run: could not read the screen aloud");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Signs;
    impl Lane for Signs {
        fn browser_sign_in(&self, p: &str) -> bool {
            p == "openrouter"
        }
        fn sign_in(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn downloadable(&self) -> Option<LocalModel> {
            Some(LocalModel { bytes: 2_500_000_000, present: false })
        }
    }

    #[test]
    fn the_full_provider_list_stays_behind_the_cards() {
        let every = every_card(&PendingLane);
        let known = KNOWN_PROVIDERS.iter().filter(|p| !p.id.is_empty()).count();
        assert_eq!(every.len(), known, "every provider the catalogue knows, expert tier included");
        for id in ["anthropic", "openai", "ollama", "llamacpp", "nvidia-nim", "cloudflare", "litellm"] {
            assert!(every.iter().any(|c| c.id == id), "{id}");
        }
    }

    #[test]
    fn named_cards_say_one_plain_line_and_an_honest_need() {
        let cards = featured_cards(None, &PendingLane);
        let claude = &cards[0];
        assert_eq!(claude.name, "Claude");
        assert_eq!(claude.line, "online, careful with words");
        assert_eq!(claude.needs, "needs an Anthropic account");
        assert!(cards.iter().all(|c| !c.needs.is_empty() && !c.line.is_empty()));
        assert_eq!(cards.iter().find(|c| c.id == "openai").unwrap().needs, "needs an OpenAI account");
        assert!(every_card(&PendingLane).iter().find(|c| c.id == "ollama").unwrap().needs.contains("running"));
    }

    #[test]
    fn this_computer_is_offered_only_when_a_model_is_here_or_downloadable_with_its_size() {
        assert!(!featured_cards(None, &PendingLane).iter().any(|c| c.id == THIS_COMPUTER));
        let here = LocalModel { bytes: 2_740_000_000, present: true };
        let c = featured_cards(Some(&here), &PendingLane).pop().unwrap();
        assert_eq!((c.name.as_str(), c.line.as_str(), c.needs.as_str()), ("On this computer", "private, slower", "already here (2.7 GB)"));
        let fetch = Signs.downloadable().unwrap();
        assert_eq!(this_computer_card(&fetch).needs, "downloads 2.5 GB");
    }

    #[test]
    fn a_model_on_disk_is_found_by_its_size_and_an_empty_dir_offers_none() {
        let dir = std::env::temp_dir().join(format!("fr-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(local_model_in(&dir), None);
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert_eq!(local_model_in(&dir), None);
        std::fs::write(dir.join("yantrik-4b.gguf"), vec![0u8; 4096]).unwrap();
        assert_eq!(local_model_in(&dir), Some(LocalModel { bytes: 4096, present: true }));
        assert_eq!(local_model_in(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn browser_sign_in_is_offered_only_where_the_backend_has_it() {
        assert!(every_card(&PendingLane).iter().all(|c| !c.browser_sign_in), "the stub offers none");
        let with = every_card(&Signs);
        assert!(with.iter().find(|c| c.id == "openrouter").unwrap().browser_sign_in);
        assert!(!with.iter().find(|c| c.id == "anthropic").unwrap().browser_sign_in);
        assert!(PendingLane.sign_in("openrouter").is_err());
    }

    #[test]
    fn show_me_where_is_offered_only_for_a_known_key_page() {
        assert!(key_page("anthropic").unwrap().starts_with("https://"));
        assert_eq!(key_page("custom"), None);
        let every = every_card(&PendingLane);
        assert!(every.iter().find(|c| c.id == "anthropic").unwrap().key_help);
        assert!(!every.iter().find(|c| c.id == "ollama").unwrap().key_help);
    }

    #[test]
    fn a_row_goes_not_checked_then_checking_with_its_start_then_passed_or_the_reason() {
        let mut m = MindRows::new("companion", "Yantrik Mind");
        assert_eq!(m.checks[0].1.words(), "Not checked yet");
        assert_eq!(m.words(), "Not checked yet");
        m.set(CheckKind::Answer, CheckState::Checking("2:16".into()));
        assert_eq!(m.checks[0].1.words(), "Checking (started 2:16)");
        assert_eq!(m.checks[0].1.state(), "checking");
        m.set(CheckKind::Answer, CheckState::Done(Proof::Passed { details: "x".into() }));
        assert_eq!(m.checks[0].1.words(), "Passed");
        m.set(CheckKind::Remember, CheckState::Done(Proof::Attention { reason: "the note did not come back".into(), details: "".into() }));
        assert_eq!(m.checks[1].1.words(), "Needs attention: the note did not come back");
        assert!(!m.done());
        m.set(CheckKind::Desktop, CheckState::Done(Proof::Passed { details: "".into() }));
        assert!(m.done() && !m.ok());
        assert_eq!(m.words(), "Needs attention: the note did not come back");
        m.set(CheckKind::Remember, CheckState::Done(Proof::Passed { details: "".into() }));
        assert_eq!(m.words(), "Ready");
    }

    #[test]
    fn no_row_words_are_a_percentage_or_a_spinner() {
        let states = [
            CheckState::NotChecked,
            CheckState::Checking("9:05".into()),
            CheckState::Done(Proof::Passed { details: String::new() }),
            CheckState::Done(Proof::Attention { reason: "r".into(), details: String::new() }),
        ];
        for s in states {
            let w = s.words();
            assert!(!w.contains('%') && !w.contains("...") && !w.contains('…'), "{w}");
        }
    }

    #[test]
    fn the_clock_reads_like_a_person_says_it() {
        use chrono::TimeZone;
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 11, 14, 16, 42).unwrap();
        assert_eq!(clock(t), "2:16");
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 11, 9, 5, 0).unwrap();
        assert_eq!(clock(t), "9:05");
    }

    #[test]
    fn the_pending_proofs_never_claim_a_pass() {
        let p = PendingProofs;
        assert!(!p.minds().is_empty());
        for (id, _) in p.minds() {
            assert!(matches!(p.remember(&id), Proof::Attention { .. }));
            assert!(matches!(p.desktop(&id), Proof::Attention { .. }));
        }
    }

    #[test]
    fn the_private_lane_keeps_personal_things_on_this_computer_first() {
        let lan = ("lan-ollama/llama3".to_string(), "http://192.168.1.5:11434/v1".to_string());
        let here = ("on-this-computer-qwen-3-5-4b/qwen3.5-4b".to_string(), "http://127.0.0.1:8341/v1".to_string());
        assert_eq!(lane_model(&[lan.clone(), here.clone()]).as_deref(), Some(here.0.as_str()));
        assert_eq!(lane_model(&[lan.clone()]).as_deref(), Some(lan.0.as_str()));
        assert_eq!(lane_model(&[]), None, "no lane: refused with the reason, nothing shared");
        assert_eq!(Sharing::parse("share"), Some(Sharing::Share));
        assert_eq!(Sharing::parse("private-lane"), Some(Sharing::PrivateLane));
        assert_eq!(Sharing::parse("off"), Some(Sharing::Off));
        assert_eq!(Sharing::parse("local"), None, "a model on this computer has nothing to share");
    }

    #[test]
    fn the_account_id_is_the_one_ai_accounts_gives_the_saved_provider() {
        let e = ProviderStoreEntry {
            id: "anthropic-onboarding".into(),
            name: "Anthropic".into(),
            provider_type: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key: Some("k".into()),
            auth_type: "x-api-key".into(),
            is_primary: true,
            is_fallback: false,
            model: String::new(),
        };
        assert_eq!(account_id(&e), "anthropic");
        let ids: Vec<String> = crate::ai_accounts::account::accounts(&[e], &[], &Default::default(), &Default::default())
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, ["anthropic"]);
    }

    fn an_account(provider: &str, model: &str) -> crate::ai_accounts::account::Account {
        let e = ProviderStoreEntry {
            id: format!("{provider}-onboarding"),
            name: provider.into(),
            provider_type: provider.into(),
            base_url: "https://example.com/v1".into(),
            api_key: Some("k".into()),
            auth_type: "bearer".into(),
            is_primary: true,
            is_fallback: false,
            model: model.into(),
        };
        crate::ai_accounts::account::accounts(&[e], &[], &Default::default(), &Default::default()).remove(0)
    }

    fn listed(ids: &[&str]) -> Vec<crate::wire::provider_models::ListedModel> {
        ids.iter().map(|id| crate::wire::provider_models::ListedModel { id: id.to_string(), name: id.to_string(), size_bytes: 0 }).collect()
    }

    #[test]
    fn the_model_step_offers_what_the_account_lists_and_picks_none() {
        let got = model_choices(&an_account("openai", ""), Some(Ok(listed(&["gpt-x", "gpt-y", "gpt-z"])))).unwrap();
        let ids: Vec<&str> = got.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gpt-x", "gpt-y", "gpt-z"], "every listed model, in the account's order, none chosen");
        // A provider with no list of its own offers its hand-kept one, not a guess.
        let curated = model_choices(&an_account("perplexity", ""), None).unwrap();
        assert!(curated.iter().any(|m| m.id == "sonar"));
    }

    #[test]
    fn a_failed_or_empty_listing_is_the_plain_reason_not_a_default() {
        use crate::wire::provider_models::ListError;
        let refused = model_choices(&an_account("openai", ""), Some(Err(ListError::KeyRefused))).unwrap_err();
        assert!(refused.contains("refused"), "{refused}");
        let empty = model_choices(&an_account("openai", ""), Some(Ok(Vec::new()))).unwrap_err();
        assert!(empty.contains("no model"), "{empty}");
    }

    /// The owner's release gate: nothing on screen shows, or silently picks, a model the person
    /// did not configure. No catalogue default and no first-in-the-list anywhere in the first run.
    #[test]
    fn the_first_run_never_picks_a_model_for_the_person() {
        let src = include_str!("onboarding.rs");
        let body = src.split("#[cfg(test)]").next().unwrap();
        for banned in ["default_model_for", "pick_model(&", "models.first()", ".first().map(|m| m.id"] {
            assert!(!body.contains(banned), "the first run uses {banned}");
        }
        let slint = include_str!("../../../yantrik-ui-slint/ui/onboarding_first_run.slint");
        assert!(slint.contains("in-out property <string> chosen-model: \"\";"), "no model is chosen before the person chooses");
        assert!(!slint.contains("yantrik-4b") && !body.contains("yantrik-4b"));
    }

    #[test]
    fn can_it_answer_passes_only_on_the_minds_own_words() {
        use crate::bridge::AskError;
        assert!(matches!(answer_proof(Ok("OK".into()), 420), Proof::Passed { .. }));
        match answer_proof(Err(AskError::NoModel), 5) {
            Proof::Attention { reason, .. } => assert_eq!(reason, "it is not set up to think yet"),
            p => panic!("{p:?}"),
        }
        match answer_proof(Err(AskError::Failed("companion timed out".into())), 90_000) {
            Proof::Attention { reason, .. } => assert_eq!(reason, "companion timed out"),
            p => panic!("{p:?}"),
        }
        assert!(matches!(answer_proof(Ok("  ".into()), 1), Proof::Attention { .. }));
        assert!(matches!(ask_the_mind("hermes"), Proof::Attention { .. }), "a mind it cannot ask is not passed");
    }

    #[test]
    fn a_provider_whose_address_needs_filling_goes_to_advanced() {
        let cf = needs_filling("cloudflare").expect("Cloudflare's address has {account_id}");
        assert!(cf.contains('{'), "{cf}");
        assert_eq!(needs_filling("anthropic"), None);
        for p in KNOWN_PROVIDERS.iter().filter(|p| !p.id.is_empty()) {
            assert_eq!(needs_filling(p.id).is_some(), p.url_needs_filling(), "{}", p.id);
        }
    }

    #[test]
    fn a_first_boot_without_the_installer_asks_the_name_and_nothing_else() {
        assert!(asks_name(false, ""), "no installer, no name: ask");
        assert!(asks_name(false, "User"), "the config's placeholder is not a name");
        assert!(!asks_name(true, ""), "the installer asked already");
        assert!(!asks_name(false, "Asha"), "a name already kept is not asked again");
        let slint = include_str!("../../../yantrik-ui-slint/ui/onboarding_first_run.slint");
        assert!(slint.contains("What should your minds call you?") && slint.contains("You can leave this empty."));
        for hidden in ["interests", "Interests", "Where do you live", "companion's name", "Name your companion"] {
            assert!(!slint.contains(hidden), "the first run asks \"{hidden}\"");
        }
    }

    #[test]
    fn the_sharing_sentence_is_the_one_ai_accounts_records() {
        let slint = include_str!("../../../yantrik-ui-slint/ui/onboarding_first_run.slint");
        let line = crate::ai_accounts::onboarding::share_line("Claude");
        let (head, tail) = line.split_once("Claude").unwrap();
        assert!(slint.contains(&format!("\"{head}\"")), "{head}");
        assert!(slint.contains(&format!("\"{tail}\"")), "{tail}");
    }

    #[test]
    fn the_private_lane_offer_states_its_size_or_is_absent() {
        assert_eq!(private_lane_words(None), "");
        let w = private_lane_words(Some(&LocalModel { bytes: 2_500_000_000, present: false }));
        assert!(w.contains("2.5 GB") && w.contains("slower"), "{w}");
    }

    #[test]
    fn what_is_shared_is_listed_plainly() {
        assert!(SHARED_ITEMS.contains(&"Your name") && SHARED_ITEMS.contains(&"Your town"));
        assert!(SHARED_ITEMS.contains(&"The app you're in"));
    }

    #[test]
    fn the_text_size_and_sharing_choice_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("fr-saved-{}", std::process::id()));
        let path = dir.join("first-run.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_saved(&path), Saved::default());
        update_saved(&path, |s| s.text_scale = 1.25);
        update_saved(&path, |s| s.sharing = "private-lane".into());
        let s = load_saved(&path);
        assert_eq!((s.text_scale, s.sharing.as_str()), (1.25, "private-lane"));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("key"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only the experience doc's plain words, on every screen of the first run.
    #[test]
    fn the_screens_use_the_plain_vocabulary() {
        let slint = include_str!("../../../yantrik-ui-slint/ui/onboarding_first_run.slint");
        let shown: String = slint
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .filter_map(|l| l.split_once("text: \"").or_else(|| l.split_once("label: \"")).map(|(_, r)| r))
            .collect::<Vec<_>>()
            .join("\n");
        for word in ["Lens", "harness", "Witness", "thread", "private lane", "attach", "Decline", "Error", "LLM", "API"] {
            assert!(!shown.contains(word), "a screen says \"{word}\"");
        }
        for want in ["Welcome to Yantrik.", "Make text bigger", "Read this to me", "Get started", "Who should do the thinking?",
                     "Connect account", "Paste a key", "Show me where to find it", "Advanced", "What's shared?",
                     "Keep personal things on this computer instead", "Getting your minds ready", "Your minds", "Add another mind"] {
            assert!(slint.contains(want), "{want}");
        }
    }
}
