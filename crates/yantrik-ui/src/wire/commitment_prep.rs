//! The person's yes, no and turn-on about getting promised things ready (#1184), on screen.
//!
//! The buttons are in `commitment_prep.slint` and call `PrepState.answer`, a callback of their own:
//! a row's expand toggle is a different callback and never reaches this file. Each press goes to the
//! companion as the person's answer; what it says back is shown as a toast, and the answered rows are
//! taken off the brief card. The switch in Settings → Companion is the place that is always there, so
//! its state (on or off, and the question while one waits) is asked again every few seconds.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use yantrik_companion::commitment_consent::Status;

use crate::app_context::AppContext;
use crate::commitment_brief::Reply;
use crate::{App, PrepState};

/// Replies are looked for this often, and the state is asked again every [`ASK_EVERY`] looks.
const TICK: Duration = Duration::from_millis(200);
const ASK_EVERY: u32 = 25;

pub fn wire(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let pending: Rc<RefCell<Vec<(Receiver<Reply>, bool)>>> = Rc::new(RefCell::new(Vec::new()));

    let queue = pending.clone();
    let asker = bridge.clone();
    ui.global::<PrepState>().on_answer(move |action_id| {
        queue.borrow_mut().push((asker.answer_commitment_prep(action_id.to_string()), true));
    });

    let weak = ui.as_weak();
    let ticks = RefCell::new(0u32);
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, TICK, move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut n = ticks.borrow_mut();
        if *n % ASK_EVERY == 0 && bridge.is_online() {
            pending.borrow_mut().push((bridge.answer_commitment_prep(String::new()), false));
        }
        *n += 1;
        let mut waiting = pending.borrow_mut();
        waiting.retain(|(rx, pressed)| match rx.try_recv() {
            Ok((said, status)) => {
                show(&ui, &status);
                if let Some(said) = said.filter(|_| *pressed) {
                    answered(&ui, &said);
                }
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
    });
    std::mem::forget(timer);
}

/// What Settings → Companion shows.
fn show(ui: &App, status: &Status) {
    let g = ui.global::<PrepState>();
    g.set_on(status.on);
    let (text, yes, no) = status.question.as_ref().map(|q| (q.text.clone(), q.yes.clone(), q.no.clone())).unwrap_or_default();
    g.set_question(text.into());
    g.set_yes_id(yes.into());
    g.set_no_id(no.into());
}

/// The person's press answered: say what it did, and take the answered rows off the brief card.
fn answered(ui: &App, said: &str) {
    super::toast::local(&ui.as_weak(), "Yantrik", said, "", 1);
    let rest: Vec<_> = ui.get_morning_brief_sections().iter().filter(|s| s.yes_id.is_empty()).collect();
    ui.set_morning_brief_sections(ModelRc::new(VecModel::from(rest)));
}
