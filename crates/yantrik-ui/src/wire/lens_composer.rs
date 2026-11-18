//! What the Lens's composer shows, published to the window every tick: what Enter will do (send,
//! queue for the busy mind, or answer its question), the question above the box, and the Stop
//! control (`lens_stop`).

use slint::ComponentHandle;

use super::agents::clip;
use super::lens_queue::Composer;
use crate::{AgentsState, App, LensStop};

const QUESTION_CHARS: usize = yantrik_harness::redact::QUESTION_CHARS;

/// What the Lens's composer says Enter will do (`lens_queue`): send, queue for the busy mind, or
/// answer its question, which is shown above the box. Set only where it changed.
pub fn publish(ui: &App) {
    if let Some(host) = super::harness::host() {
        crate::agents::stopped::settle(host);
    }
    let composer = Composer::active();
    let g = ui.global::<AgentsState>();
    let mode = composer.mode().word();
    if g.get_lens_composer_mode() != mode {
        g.set_lens_composer_mode(mode.into());
    }
    let placeholder = composer.placeholder();
    if g.get_lens_composer_placeholder() != placeholder.as_str() {
        g.set_lens_composer_placeholder(placeholder.into());
    }
    // The Stop control, and the turn a press of it is for.
    let turn = composer.turn_word();
    let stop = ui.global::<LensStop>();
    if stop.get_turn() != turn {
        stop.set_turn(turn.into());
    }
    super::lens_stop::seen(composer.turn_id);
    let asks = composer.asking.as_ref().map(|a| format!("{} asks: {}", composer.mind, clip(&a.prompt, QUESTION_CHARS))).unwrap_or_default();
    if g.get_lens_composer_asks() != asks.as_str() {
        g.set_lens_composer_asks(asks.into());
    }
}
