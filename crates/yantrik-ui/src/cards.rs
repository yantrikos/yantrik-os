//! Whisper Card Manager — manages floating notification cards.
//!
//! Cards are scored urges that appear as floating overlays.
//! Three tiers: Queue (badge only), Whisper (floating, auto-dismiss), Interrupt (persistent).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use slint::{Model, ModelRc, VecModel};
use yantrik_companion::proactive_backoff::{why_text, BackoffLevel, Outcome, ProactiveKind};

use super::{App, WhisperCardItem};
use crate::features;

/// The cooldown a backed-off kind of card waits between cards, before its level's multiplier
/// (3x at "less often", 10x at "rarely"). A kind at "normal" has no cooldown of its own.
const CARD_BASE_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// How long a card stays after "Less of this", so its line can say what that did.
const ACK_LINGER: Duration = Duration::from_secs(4);

/// The kind a card's source belongs to, or `None` for a card that is not the shell speaking
/// first: a relayed app notification is the app's, and is not backed off here (#754).
fn card_kind(source: &str) -> Option<ProactiveKind> {
    if source == "NotificationRelay" {
        return None;
    }
    Some(ProactiveKind::from_source(source))
}

/// Why a security card appeared. A safety signal is never backed off, so it has no kind to back
/// off and offers no "Less of this" (design/companion-who-cares: safety signals override the
/// backoff).
const SECURITY_WHY: &str = "It may put your computer or your data at risk.";

/// Category → RGB accent color.
fn category_rgb(cat: &features::UrgeCategory) -> (f32, f32, f32) {
    match cat {
        features::UrgeCategory::Resource => (0.83, 0.65, 0.45),       // amber
        features::UrgeCategory::Security => (0.91, 0.42, 0.42),       // red
        features::UrgeCategory::FileManagement => (0.35, 0.78, 0.83), // cyan
        features::UrgeCategory::Focus => (0.77, 0.55, 0.83),          // purple
        features::UrgeCategory::Celebration => (0.55, 0.91, 0.42),    // green
        features::UrgeCategory::Shell => (0.91, 0.48, 0.29),          // orange-red
        features::UrgeCategory::Notification => (0.40, 0.70, 0.92),   // blue
        features::UrgeCategory::Vision => (0.35, 0.85, 0.95),         // bright cyan
        features::UrgeCategory::Project => (0.95, 0.75, 0.30),        // gold
    }
}

/// Each kind's backoff level, as the mind's database holds it: published by the companion worker
/// whenever a level changes, and raised here at once when "Less of this" is pressed.
static LEVELS: std::sync::LazyLock<std::sync::Mutex<HashMap<ProactiveKind, BackoffLevel>>> =
    std::sync::LazyLock::new(Default::default);

/// Publish the levels the mind's database holds (any thread).
pub fn publish_levels(levels: impl IntoIterator<Item = (ProactiveKind, BackoffLevel)>) {
    if let Ok(mut map) = LEVELS.lock() {
        *map = levels.into_iter().collect();
    }
}

/// Manages floating Whisper Cards (max 3 visible) + hint counter for Queue tier.
pub struct CardManager {
    cards: Vec<ActiveCard>,
    max_visible: usize,
    hint_count: i32,
    /// When a card of each kind was last shown, for a backed-off kind's cooldown.
    last_shown: HashMap<ProactiveKind, Instant>,
    /// Outcomes not yet handed to the mind's throttle: cards that faded unanswered.
    outcomes: Vec<(ProactiveKind, Outcome)>,
}

struct ActiveCard {
    id: String,
    source: String,
    title: String,
    context: String,
    tier: features::UrgeTier,
    #[allow(dead_code)]
    pressure: f32,
    accent: (f32, f32, f32),
    kind: Option<ProactiveKind>,
    why: String,
    #[allow(dead_code)]
    created_at: std::time::Instant,
    auto_dismiss_at: Option<std::time::Instant>,
    shown: bool,
}

impl CardManager {
    pub fn new() -> Self {
        Self {
            cards: Vec::new(),
            max_visible: 3,
            hint_count: 0,
            last_shown: HashMap::new(),
            outcomes: Vec::new(),
        }
    }

    /// Add a scored urge. Queue tier → hint badge only. Whisper/Interrupt → floating card.
    pub fn add(&mut self, scored: &features::ScoredUrge) {
        self.add_at(scored, crate::wire::settings::dnd_mode(), Instant::now());
    }

    /// `add`, with Do Not Disturb and the clock given.
    ///
    /// Do Not Disturb holds the shell's own cards too (#754): only a security card still floats;
    /// the rest wait behind the hint badge. A kind that is off until the person asks shows
    /// nothing; a backed-off kind waits its longer cooldown and never interrupts.
    pub fn add_at(&mut self, scored: &features::ScoredUrge, dnd: bool, now: Instant) {
        // A safety signal (a security warning, a battery about to die) is decided before any
        // backoff, and never throttled (#754).
        let security = scored.urge.safety || scored.urge.category == features::UrgeCategory::Security;
        if scored.tier == features::UrgeTier::Queue && !security {
            self.hint_count += 1;
            return;
        }
        if dnd && !security {
            self.hint_count += 1;
            return;
        }
        let kind = if security { None } else { card_kind(&scored.urge.source) };
        let mut tier = scored.tier.clone();
        if security && tier == features::UrgeTier::Queue {
            tier = features::UrgeTier::Whisper;
        }
        if let Some(kind) = kind {
            let level = self.level(kind);
            let Some(mult) = level.cooldown_multiplier() else {
                return;
            };
            if level != BackoffLevel::Normal {
                let wait = CARD_BASE_COOLDOWN.mul_f64(mult);
                if self.last_shown.get(&kind).is_some_and(|t| now.duration_since(*t) < wait) {
                    return;
                }
                if tier == features::UrgeTier::Interrupt {
                    tier = features::UrgeTier::Whisper;
                }
            }
        }

        // Dedup by source
        if self.cards.iter().any(|c| c.source == scored.urge.source && c.title == scored.urge.title) {
            return;
        }

        let auto_dismiss = match tier {
            features::UrgeTier::Whisper => Some(std::time::Instant::now() + std::time::Duration::from_secs(8)),
            features::UrgeTier::Interrupt => None, // Must be manually dismissed
            _ => Some(std::time::Instant::now() + std::time::Duration::from_secs(5)),
        };

        self.cards.push(ActiveCard {
            id: scored.urge.id.clone(),
            source: scored.urge.source.clone(),
            title: scored.urge.title.clone(),
            context: scored.urge.body.clone(),
            tier,
            pressure: scored.pressure,
            accent: category_rgb(&scored.urge.category),
            kind,
            why: if security {
                SECURITY_WHY.to_string()
            } else {
                kind.map(|k| why_text(k, &serde_json::Value::Null)).unwrap_or_default()
            },
            created_at: std::time::Instant::now(),
            auto_dismiss_at: auto_dismiss,
            shown: false,
        });

        if let Some(kind) = kind {
            self.last_shown.insert(kind, now);
        }

        // Trim to max: the oldest card that is not a safety signal goes first.
        while self.cards.len() > self.max_visible + 2 {
            let oldest = self.cards.iter().position(|c| c.why != SECURITY_WHY).unwrap_or(0);
            self.cards.remove(oldest);
        }
    }

    /// Dismiss a card by ID. Returns the source name and kind if found.
    pub fn dismiss(&mut self, id: &str) -> Option<(String, Option<ProactiveKind>)> {
        if let Some(pos) = self.cards.iter().position(|c| c.id == id) {
            let card = self.cards.remove(pos);
            Some((card.source, card.kind))
        } else {
            None
        }
    }

    /// "Stop this kind" on a card: its kind is off at once, and the card stays a moment so its
    /// line can say what that did. Returns the kind, for the mind to keep.
    pub fn stop_kind(&mut self, id: &str) -> Option<ProactiveKind> {
        let now = Instant::now();
        let card = self.cards.iter_mut().find(|c| c.id == id)?;
        let kind = card.kind?;
        card.auto_dismiss_at = Some(now + ACK_LINGER);
        // Already answered: its fading out afterwards is not "ignored".
        card.kind = None;
        if let Ok(mut levels) = LEVELS.lock() {
            levels.insert(kind, BackoffLevel::OffUntilAsked);
        }
        Some(kind)
    }

    fn level(&self, kind: ProactiveKind) -> BackoffLevel {
        LEVELS
            .lock()
            .ok()
            .and_then(|levels| levels.get(&kind).copied())
            .unwrap_or(BackoffLevel::Normal)
    }

    /// Outcomes for the mind's throttle since the last call: cards that faded unanswered.
    pub fn take_outcomes(&mut self) -> Vec<(ProactiveKind, Outcome)> {
        std::mem::take(&mut self.outcomes)
    }

    /// Tick: auto-dismiss expired cards. Returns true if anything changed.
    pub fn tick(&mut self) -> bool {
        let now = std::time::Instant::now();
        let before = self.cards.len();

        let outcomes = &mut self.outcomes;
        self.cards.retain(|c| {
            if let Some(deadline) = c.auto_dismiss_at {
                let keep = now < deadline;
                if !keep {
                    if let Some(kind) = c.kind {
                        outcomes.push((kind, Outcome::Ignored));
                    }
                }
                keep
            } else {
                true
            }
        });

        // Mark new cards as shown
        let mut changed = self.cards.len() != before;
        for card in &mut self.cards {
            if !card.shown {
                card.shown = true;
                changed = true;
            }
        }

        changed
    }
}

/// Push scored urges into the CardManager and sync to UI.
pub fn push_whisper_cards(
    card_mgr: &std::cell::RefCell<CardManager>,
    ui_weak: &slint::Weak<App>,
    scored: &[features::ScoredUrge],
) {
    let mut mgr = card_mgr.borrow_mut();
    for s in scored {
        mgr.add(s);
    }
    sync_whisper_ui(&mgr, ui_weak);
}

/// Sync CardManager state to Slint WhisperCardItem model.
pub fn sync_whisper_ui(mgr: &CardManager, ui_weak: &slint::Weak<App>) {
    let items: Vec<WhisperCardItem> = mgr
        .cards
        .iter()
        .take(mgr.max_visible)
        .map(|c| WhisperCardItem {
            id: c.id.clone().into(),
            title: c.title.clone().into(),
            context: c.context.clone().into(),
            accent_r: c.accent.0,
            accent_g: c.accent.1,
            accent_b: c.accent.2,
            target_opacity: if c.tier == features::UrgeTier::Interrupt { 1.0 } else { 0.6 },
            show_actions: c.tier == features::UrgeTier::Interrupt,
            action_label: "Open".into(),
            shown: c.shown,
            source: c.source.clone().into(),
            why: c.why.clone().into(),
            can_back_off: c.kind.is_some(),
        })
        .collect();

    let hint = mgr.hint_count;
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_whisper_cards(ModelRc::new(VecModel::from(items)));
        ui.set_whisper_hint_count(hint);
    }
}

/// Push scored urges as Quiet Queue cards (legacy GlanceCard system).
#[allow(dead_code)]
pub fn push_queue_cards(ui_weak: &slint::Weak<App>, scored: &[features::ScoredUrge]) {
    let cards: Vec<super::UrgeCardData> = scored
        .iter()
        .filter(|s| s.tier == features::UrgeTier::Queue)
        .map(|s| {
            let color = match s.urge.category {
                features::UrgeCategory::Resource => {
                    slint::Color::from_rgb_u8(0xD4, 0xA5, 0x74)
                }
                features::UrgeCategory::Security => {
                    slint::Color::from_rgb_u8(0xE8, 0x6B, 0x6B)
                }
                features::UrgeCategory::FileManagement => {
                    slint::Color::from_rgb_u8(0x5A, 0xC8, 0xD4)
                }
                features::UrgeCategory::Focus => {
                    slint::Color::from_rgb_u8(0xC4, 0x8B, 0xD4)
                }
                features::UrgeCategory::Celebration => {
                    slint::Color::from_rgb_u8(0x8B, 0xE8, 0x6B)
                }
                features::UrgeCategory::Shell => {
                    slint::Color::from_rgb_u8(0xE8, 0x7B, 0x4B)
                }
                features::UrgeCategory::Notification => {
                    slint::Color::from_rgb_u8(0x66, 0xB3, 0xEB)
                }
                features::UrgeCategory::Vision => {
                    slint::Color::from_rgb_u8(0x5A, 0xD9, 0xF2)
                }
                features::UrgeCategory::Project => {
                    slint::Color::from_rgb_u8(0xF2, 0xBF, 0x4D)
                }
            };

            super::UrgeCardData {
                urge_id: s.urge.id.clone().into(),
                instinct_name: s.urge.source.clone().into(),
                reason: s.urge.body.clone().into(),
                urgency: s.pressure,
                suggested_message: s.urge.title.clone().into(),
                time_ago: "just now".into(),
                border_color: color,
            }
        })
        .collect();

    if cards.is_empty() {
        return;
    }

    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            let existing = ui.get_urges();
            let model = existing
                .as_any()
                .downcast_ref::<VecModel<super::UrgeCardData>>();

            if let Some(model) = model {
                for card in cards {
                    model.push(card);
                }
                ui.set_pending_count(model.row_count() as i32);
            } else {
                let count = cards.len();
                ui.set_urges(ModelRc::new(VecModel::from(cards)));
                ui.set_pending_count(count as i32);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urge(id: &str, source: &str, category: features::UrgeCategory, tier: features::UrgeTier) -> features::ScoredUrge {
        features::ScoredUrge {
            urge: features::Urge {
                id: id.into(),
                source: source.into(),
                title: format!("{id} title"),
                body: "body".into(),
                urgency: 0.9,
                confidence: 0.9,
                safety: category == features::UrgeCategory::Security,
                category,
            },
            pressure: 0.9,
            tier,
        }
    }

    /// Every card the shell speaks first carries its why; a relayed app notification is the
    /// app's own and carries neither a why nor "Less of this" (#754).
    #[test]
    fn a_card_says_why_it_appeared() {
        let mut mgr = CardManager::new();
        let now = Instant::now();
        mgr.add_at(&urge("a", "resource_guardian", features::UrgeCategory::Resource, features::UrgeTier::Whisper), false, now);
        mgr.add_at(&urge("b", "NotificationRelay", features::UrgeCategory::Notification, features::UrgeTier::Whisper), false, now);
        assert_eq!(mgr.cards[0].why, "Your computer needs a look.");
        assert_eq!(mgr.cards[0].kind, Some(ProactiveKind::System));
        assert_eq!(mgr.cards[1].why, "");
        assert_eq!(mgr.cards[1].kind, None);
    }

    /// Do Not Disturb holds the shell's cards behind the hint badge; a security card still shows.
    #[test]
    fn do_not_disturb_holds_cards_but_not_security() {
        let mut mgr = CardManager::new();
        let now = Instant::now();
        mgr.add_at(&urge("a", "network_watcher", features::UrgeCategory::Resource, features::UrgeTier::Whisper), true, now);
        assert!(mgr.cards.is_empty());
        assert_eq!(mgr.hint_count, 1);
        mgr.add_at(&urge("b", "process_sentinel", features::UrgeCategory::Security, features::UrgeTier::Interrupt), true, now);
        assert_eq!(mgr.cards.len(), 1);
    }

    /// "Stop this kind" turns the kind off at once: no more cards of it, and the pressed card is
    /// not counted as ignored when it fades.
    #[test]
    fn stop_this_kind_turns_the_kind_off() {
        let mut mgr = CardManager::new();
        let now = Instant::now();
        mgr.add_at(&urge("a", "tool_suggester", features::UrgeCategory::Focus, features::UrgeTier::Interrupt), false, now);
        assert_eq!(mgr.stop_kind("a"), Some(ProactiveKind::Suggestions));
        assert_eq!(mgr.level(ProactiveKind::Suggestions), BackoffLevel::OffUntilAsked);

        mgr.add_at(&urge("b", "clipboard_intelligence", features::UrgeCategory::Focus, features::UrgeTier::Interrupt), false, now + Duration::from_secs(86_400));
        assert!(mgr.cards.iter().all(|c| c.id != "b"), "the kind is off");

        mgr.cards.iter_mut().for_each(|c| c.auto_dismiss_at = Some(now));
        mgr.tick();
        assert!(mgr.take_outcomes().is_empty(), "the pressed card faded answered, not ignored");
        if let Ok(mut levels) = LEVELS.lock() {
            levels.remove(&ProactiveKind::Suggestions);
        }
    }

    /// A kind the throttle backed off waits its longer cooldown and no longer interrupts.
    #[test]
    fn a_backed_off_kind_waits_longer_and_never_interrupts() {
        let mut mgr = CardManager::new();
        let now = Instant::now();
        // Patterns is used by no other test here; LEVELS is shared across the test threads.
        if let Ok(mut levels) = LEVELS.lock() {
            levels.insert(ProactiveKind::Patterns, BackoffLevel::LessOften);
        }
        mgr.add_at(&urge("a", "screen_watcher", features::UrgeCategory::Focus, features::UrgeTier::Interrupt), false, now);
        mgr.add_at(&urge("b", "project_consciousness", features::UrgeCategory::Project, features::UrgeTier::Interrupt), false, now + Duration::from_secs(60));
        assert!(mgr.cards.iter().all(|c| c.id != "b"), "inside the backed-off cooldown");
        let later = now + CARD_BASE_COOLDOWN.mul_f64(COOLDOWN_FACTOR_FOR_TEST) + Duration::from_secs(1);
        mgr.add_at(&urge("c", "project_consciousness", features::UrgeCategory::Project, features::UrgeTier::Interrupt), false, later);
        if let Ok(mut levels) = LEVELS.lock() {
            levels.remove(&ProactiveKind::Patterns);
        }
        let c = mgr.cards.iter().find(|c| c.id == "c").expect("after the cooldown it shows");
        assert_eq!(c.tier, features::UrgeTier::Whisper, "a backed-off kind never interrupts");
        assert_eq!(mgr.cards.iter().find(|c| c.id == "a").map(|c| c.tier.clone()), Some(features::UrgeTier::Whisper));
    }

    /// A security card is never throttled: not by its kind being off, nor by a cooldown, and it
    /// still interrupts. Its fading out teaches the throttle nothing, and it offers no
    /// "Less of this" (#754: safety signals override the backoff).
    #[test]
    fn a_security_card_is_never_backed_off() {
        let mut mgr = CardManager::new();
        let now = Instant::now();
        // Weather is used by no other test here; LEVELS is shared across the test threads.
        if let Ok(mut levels) = LEVELS.lock() {
            levels.insert(ProactiveKind::Weather, BackoffLevel::OffUntilAsked);
        }
        mgr.add_at(&urge("s1", "weather_watch", features::UrgeCategory::Security, features::UrgeTier::Interrupt), false, now);
        mgr.add_at(&urge("n1", "weather", features::UrgeCategory::Resource, features::UrgeTier::Whisper), false, now);
        if let Ok(mut levels) = LEVELS.lock() {
            levels.remove(&ProactiveKind::Weather);
        }
        let ids: Vec<&str> = mgr.cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["s1"], "the kind that is off is held; the security card shows");
        let c = &mgr.cards[0];
        assert_eq!(c.tier, features::UrgeTier::Interrupt, "a security card still interrupts");
        assert_eq!(c.kind, None, "nothing to back off");
        assert_eq!(c.why, SECURITY_WHY);
        assert_eq!(mgr.stop_kind("s1"), None);
        mgr.cards.iter_mut().for_each(|c| c.auto_dismiss_at = Some(now));
        mgr.tick();
        assert!(mgr.take_outcomes().is_empty());
    }

    /// A battery about to die is a safety signal: it shows during Do Not Disturb, interrupts,
    /// and offers no "Less of this" (#754).
    #[test]
    fn a_battery_about_to_die_is_never_held() {
        let mut mgr = CardManager::new();
        let now = Instant::now();
        let mut critical = urge("bc", "resource_guardian", features::UrgeCategory::Resource, features::UrgeTier::Interrupt);
        critical.urge.safety = true;
        mgr.add_at(&critical, true, now);
        let c = mgr.cards.iter().find(|c| c.id == "bc").expect("shown through Do Not Disturb");
        assert_eq!(c.kind, None);
        assert_eq!(c.tier, features::UrgeTier::Interrupt);
    }

    /// Focus mode leaves interruptibility at 0.1: a safety signal still becomes a card, never a
    /// drop or a badge, and a full stack of cards never pushes it out (#754).
    #[test]
    fn focus_mode_and_a_full_stack_never_hide_a_safety_card() {
        let mut scorer = features::UrgencyScorer::new();
        scorer.set_interruptibility(0.1);
        let mut warn = urge("sec", "clipboard_intelligence", features::UrgeCategory::Security, features::UrgeTier::Drop).urge;
        warn.urgency = 0.95;
        let mut idea = warn.clone();
        idea.id = "idea".into();
        idea.safety = false;
        idea.category = features::UrgeCategory::Resource;
        let scored = scorer.score(vec![warn, idea]);
        assert_eq!(scored.len(), 1, "the ordinary card is dropped in focus mode; the safety card is not");
        assert_eq!(scored[0].tier, features::UrgeTier::Whisper);

        let mut mgr = CardManager::new();
        let now = Instant::now();
        mgr.add_at(&urge("q", "process_sentinel", features::UrgeCategory::Security, features::UrgeTier::Queue), false, now);
        assert_eq!(mgr.cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["q"], "a queued safety card still floats");
        for i in 0..mgr.max_visible + 4 {
            mgr.add_at(&urge(&format!("n{i}"), "NotificationRelay", features::UrgeCategory::Resource, features::UrgeTier::Interrupt), false, now);
        }
        assert!(mgr.cards.iter().any(|c| c.id == "q"), "the trim never removes the safety card");
    }

    const COOLDOWN_FACTOR_FOR_TEST: f64 = yantrik_companion::proactive_backoff::COOLDOWN_LESS_OFTEN;
}
