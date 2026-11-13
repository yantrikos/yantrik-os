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
use yantrik_companion::commitment_cards::Card;

use super::commitment_prep_open as open;
use yantrik_companion::commitment_consent::{Status, TURN_OFF, TURN_ON};
use yantrik_companion::commitment_reply::{self, Shown};

use crate::app_context::AppContext;
use crate::commitment_brief::Reply;
use crate::{App, PrepCardData, PrepState};

/// Replies are looked for this often, and the state is asked again every [`ASK_EVERY`] looks.
const TICK: Duration = Duration::from_millis(200);
const ASK_EVERY: u32 = 25;

pub fn wire(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let pending: Rc<RefCell<Vec<(Receiver<Reply>, Option<String>)>>> = Rc::new(RefCell::new(Vec::new()));

    let g = ui.global::<PrepState>();
    g.set_off_id(TURN_OFF.into());
    g.set_on_id(TURN_ON.into());
    // The cards last shown, with their paths and commitment ids, which never go into Slint.
    let shown: Rc<RefCell<Vec<Card>>> = Rc::new(RefCell::new(Vec::new()));
    wire_opens(ui, shown.clone());
    let queue = pending.clone();
    let asker = bridge.clone();
    ui.global::<PrepState>().on_answer(move |action_id| {
        queue.borrow_mut().push((asker.answer_commitment_prep(action_id.to_string()), Some(action_id.to_string())));
    });

    let weak = ui.as_weak();
    let ticks = RefCell::new(0u32);
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, TICK, move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut n = ticks.borrow_mut();
        if *n % ASK_EVERY == 0 && bridge.is_online() {
            pending.borrow_mut().push((bridge.answer_commitment_prep(String::new()), None));
        }
        *n += 1;
        let mut waiting = pending.borrow_mut();
        waiting.retain(|(rx, pressed)| match rx.try_recv() {
            Ok(reply) => {
                show(&ui, &reply.1, &reply.2, &shown);
                if let Some(pressed) = pressed {
                    pressed_reply(&ui, &commitment_reply::shown(pressed, &reply));
                }
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
    });
    std::mem::forget(timer);
}

/// What Settings → Companion and the cards show. The cards are set only when they changed, so an
/// item has one card however often it is asked, and a card is not redrawn under the pointer.
fn show(ui: &App, status: &Status, cards: &[Card], shown: &RefCell<Vec<Card>>) {
    *shown.borrow_mut() = cards.to_vec();
    let g = ui.global::<PrepState>();
    let now: Vec<PrepCardData> = cards.iter().map(card_data).collect();
    let drawn: Vec<PrepCardData> = g.get_cards().iter().collect();
    if now != drawn {
        g.set_cards(ModelRc::new(VecModel::from(now)));
    }
    g.set_on(status.on);
    let (text, yes, no) = status.question.as_ref().map(|q| (q.text.clone(), q.yes.clone(), q.no.clone())).unwrap_or_default();
    g.set_question(text.into());
    g.set_yes_id(yes.into());
    g.set_no_id(no.into());
}

/// The person's press: say what it did (or why it did nothing), and take the answered rows off the
/// brief card, giving it the question that is outstanding now when the press answered nothing.
fn pressed_reply(ui: &App, shown: &Shown) {
    super::toast::local(&ui.as_weak(), "Yantrik", &shown.toast, "", 1);
    let mut rows: Vec<_> = ui.get_morning_brief_sections().iter().filter(|s| s.yes_id.is_empty()).collect();
    if let Some(q) = &shown.brief_question {
        rows.push(super::morning_brief::section(&crate::commitment_brief::question_row(q)));
    }
    ui.set_morning_brief_sections(ModelRc::new(VecModel::from(rows)));
}

/// The person's own openings. Both callbacks are pressed only by pointer-only buttons, and both
/// open as the person: the file through the launcher a click in Files uses, the draft by a launch
/// of Email naming the promise, which Email turns into its person-only `OpenPreparedFor`. Neither is the Mind's `open_app`.
fn wire_opens(ui: &App, shown: Rc<RefCell<Vec<Card>>>) {
    let g = ui.global::<PrepState>();
    let (cards, weak) = (shown.clone(), ui.as_weak());
    g.on_open_file(move |item, index| {
        let Some(ui) = weak.upgrade() else { return };
        // A bad index is refused like an unknown card, not turned into place 0.
        let ok = usize::try_from(index).ok().is_some_and(|i| open::open_file(&cards.borrow(), item.as_str(), i, |path, name| open::person_open(path, name, &ui)));
        if !ok {
            super::toast::local(&ui.as_weak(), "Yantrik", "That file isn't on this card any more.", "", 1);
        }
    });
    let weak = ui.as_weak();
    g.on_open_draft(move |item| {
        let Some(ui) = weak.upgrade() else { return };
        let opened = open::draft_for(&shown.borrow(), item.as_str()).is_some_and(|c| open::open_draft(&c));
        if !opened {
            super::toast::local(&ui.as_weak(), "Yantrik", "That draft isn't on this card any more.", "", 1);
        }
    });
}

fn card_data(c: &Card) -> PrepCardData {
    let o = open::openings(c);
    PrepCardData {
        file1_label: o.file1.into(),
        file2_label: o.file2.into(),
        more_files: o.more.into(),
        draft_label: o.draft.into(),
        item: c.item.clone().into(),
        title: c.title.clone().into(),
        text: c.text.clone().into(),
        yes_label: c.yes_label.into(),
        yes_id: c.yes_id.clone().into(),
        no_label: c.no_label.into(),
        no_id: c.no_id.clone().into(),
    }
}
