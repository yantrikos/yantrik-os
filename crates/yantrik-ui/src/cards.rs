//! Whisper Card Manager — manages floating notification cards.
//!
//! Cards are scored urges that appear as floating overlays.
//! Three tiers: Queue (badge only), Whisper (floating, auto-dismiss), Interrupt (persistent).

use slint::{Model, ModelRc, VecModel};

use super::{App, WhisperCardItem};
use crate::features;

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

/// Manages floating Whisper Cards (max 3 visible) + hint counter for Queue tier.
pub struct CardManager {
    cards: Vec<ActiveCard>,
    max_visible: usize,
    hint_count: i32,
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
        }
    }

    /// Add a scored urge. Queue tier → hint badge only. Whisper/Interrupt → floating card.
    pub fn add(&mut self, scored: &features::ScoredUrge) {
        if scored.tier == features::UrgeTier::Queue {
            self.hint_count += 1;
            return;
        }

        // Dedup by source
        if self.cards.iter().any(|c| c.source == scored.urge.source && c.title == scored.urge.title) {
            return;
        }

        let auto_dismiss = match scored.tier {
            features::UrgeTier::Whisper => Some(std::time::Instant::now() + std::time::Duration::from_secs(8)),
            features::UrgeTier::Interrupt => None, // Must be manually dismissed
            _ => Some(std::time::Instant::now() + std::time::Duration::from_secs(5)),
        };

        self.cards.push(ActiveCard {
            id: scored.urge.id.clone(),
            source: scored.urge.source.clone(),
            title: scored.urge.title.clone(),
            context: scored.urge.body.clone(),
            tier: scored.tier.clone(),
            pressure: scored.pressure,
            accent: category_rgb(&scored.urge.category),
            created_at: std::time::Instant::now(),
            auto_dismiss_at: auto_dismiss,
            shown: false,
        });

        // Trim to max
        while self.cards.len() > self.max_visible + 2 {
            self.cards.remove(0);
        }
    }

    /// Dismiss a card by ID. Returns the source name if found.
    pub fn dismiss(&mut self, id: &str) -> Option<String> {
        if let Some(pos) = self.cards.iter().position(|c| c.id == id) {
            let card = self.cards.remove(pos);
            Some(card.source)
        } else {
            None
        }
    }

    /// Tick: auto-dismiss expired cards. Returns true if anything changed.
    pub fn tick(&mut self) -> bool {
        let now = std::time::Instant::now();
        let before = self.cards.len();

        self.cards.retain(|c| {
            if let Some(deadline) = c.auto_dismiss_at {
                now < deadline
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
