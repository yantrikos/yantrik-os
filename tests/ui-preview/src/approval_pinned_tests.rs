//! The approval card an agent's pane answers, pinned above its reply box (agents_pinned.slint),
//! drawn by the production components at 1280×800 and answered with real pointer events.
//!
//! In the transcript a waiting card could have its head scrolled off the top while its Allow
//! stayed live: the pane follows the bottom, and whatever was appended under the card pushed who
//! was asking, the grade and the action's sentence out of view. Now the pane draws the oldest
//! waiting card outside its scroller, and the transcript keeps one line where it was. So:
//!
//! - items appended after the card leave it whole and where it was, and its Allow live;
//! - two waiting requests draw one card and "and 1 more", and there is one live Allow in the pane;
//! - answering the first brings up the second;
//! - after the card changes or moves, a click on Allow does nothing for a moment (the re-arm), and
//!   the countdown ticking does not count as a change.
use super::agents_tests::{call, fill};
use super::approval_tests::{button_top, card, save, scan, settle, RUN_RECIPE_SUMMARY};
use super::*;
use slint::Model;
use std::cell::{Cell, RefCell};

/// Past the pinned card's re-arm (400ms, checked every 100ms), so a click after it lands on a
/// settled card. Drawn first: the card takes its place, and so starts its moment, when it is laid
/// out, and only a frame lays it out here.
pub(crate) fn rest(w: &MinimalSoftwareWindow, width: u32, height: u32) {
    settle(w, width, height);
    std::thread::sleep(std::time::Duration::from_millis(550));
    settle(w, width, height);
}

/// The pointer off the card, over the details column, so no button is drawn hovered.
fn away(w: &MinimalSoftwareWindow, width: u32, height: u32) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(width as f32 - 100.0, 420.0) });
    settle(w, width, height)
}

fn waiting(id: &str) -> ApprovalRequest {
    ApprovalRequest { id: id.into(), ..card(RUN_RECIPE_SUMMARY) }
}

fn item(kind: &str, key: &str, text: &str) -> AgentItemData {
    AgentItemData { kind: kind.into(), key: key.into(), text: text.into(), ..Default::default() }
}

fn approval_item(key: &str, request: ApprovalRequest) -> AgentItemData {
    AgentItemData { approval: request, ..item("approval", key, "shell.run_recipe") }
}

/// A tool card still running, its output growing under it: what pushed the card's head off the
/// top when the card was in the transcript.
fn growing(key: &str, lines: usize) -> AgentItemData {
    AgentItemData {
        live: true,
        output_kind: "text".into(),
        output: (0..lines).map(|i| format!("building step {i} of {lines}")).collect::<Vec<_>>().join("\n").into(),
        badge: "verified".into(),
        call: call("agent_run", "", r#"agent_run command="make -j8""#, "", "running", ""),
        ..item("card", key, "")
    }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let (fw, fh) = (width as f32, height as f32);
    let window = AgentWindow::new()?;
    window.set_agent_title("Agent · pi · ship the release".into());
    let g = window.global::<AgentsState>();
    fill(&g, true);

    // What the shell does when Allow is pressed on the first card and `advance` is set: the store
    // settles it, the pane's next draw pins the second (wire/agents_pinned.rs).
    let (allowed, denied, sessioned) = (Rc::new(RefCell::new(Vec::<String>::new())), Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let advance = Rc::new(Cell::new(false));
    {
        let (a, adv, weak) = (allowed.clone(), advance.clone(), window.as_weak());
        g.on_approval_allow(move |id| {
            a.borrow_mut().push(id.to_string());
            if adv.get() && id == "appr-1" {
                let Some(window) = weak.upgrade() else { return };
                let g = window.global::<AgentsState>();
                g.set_pinned(waiting("appr-2"));
                g.set_pinned_more(0);
                let items = g.get_items();
                for i in 0..items.row_count() {
                    let mut row = items.row_data(i).unwrap();
                    if row.approval.id == "appr-1" {
                        row.approval = ApprovalRequest {
                            decision: "allowed".into(),
                            decided_at: "21:06".into(),
                            ..waiting("appr-1")
                        };
                        items.set_row_data(i, row);
                    }
                }
            }
        });
        let (d, s) = (denied.clone(), sessioned.clone());
        g.on_approval_deny(move |_| d.set(d.get() + 1));
        g.on_approval_allow_session(move |_| s.set(s.get() + 1));
    }
    let grants = || allowed.borrow().len();
    let last = || allowed.borrow().last().cloned().unwrap_or_default();

    let items = Rc::new(slint::VecModel::from(vec![
        item("prompt", "t1", "ship the release"),
        item("text", "t1.0", "Asking before it starts the council."),
        approval_item("t1.1", waiting("appr-1")),
    ]));
    g.set_items(slint::ModelRc::from(items.clone()));
    g.set_pinned(waiting("appr-1"));
    g.set_pinned_more(0);
    window.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    std::thread::sleep(std::time::Duration::from_millis(300));
    rest(w, width, height);

    // The session's box, as approval_pane_tests measures it: the scroller starts about 60px down,
    // the reply box is the last 60px; the card spans the session, Decline on the left half and
    // Allow on the right.
    let (pane_top, pane_bottom) = (60.0f32, fh - 60.0);
    let (left, right) = (36.0f32, fw - 304.0);
    let quarter = (right - left) / 4.0;
    let (deny_x, allow_x) = (left + quarter, right - quarter);

    // Where the pinned card's buttons are, from Decline (never held) up to its top.
    let find = |w: &MinimalSoftwareWindow| -> (f32, f32) {
        let before = denied.get();
        let deny_y = scan(w, deny_x, pane_top, pane_bottom, || denied.get() > before)
            .unwrap_or_else(|| panic!("Decline answers on the pinned card, inside the pane"));
        let top = button_top(w, deny_x, deny_y, pane_top, || denied.get());
        (deny_y, top + 14.0)
    };
    // The card's top and foot edges: a sensitive card's amber border, drawn as a line across
    // the card, above its buttons and below them.
    let edges = |shown: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, deny_y: f32| -> (Option<u32>, Option<u32>) {
        let edge_row = |y: u32| {
            let xs = (left as u32 + 24)..(right as u32 - 24);
            let n = xs.len();
            let amber = xs
                .filter(|x| {
                    let p = shown.as_slice()[(y * width + x) as usize];
                    p.r >= 150 && p.r as i32 > p.b as i32 + 60
                })
                .count();
            amber * 10 >= n * 9
        };
        ((pane_top as u32..deny_y as u32).find(|y| edge_row(*y)), (deny_y as u32..pane_bottom as u32).find(|y| edge_row(*y)))
    };

    // ── One waiting card: pinned, whole, and the transcript draws no buttons ──
    let (deny_y, allow_y) = find(w);
    click(w, allow_x, allow_y);
    assert_eq!(last(), "appr-1", "the pinned card's Allow answers its own request");
    let first = away(w, width, height);
    save(&first, &output.replace(".png", "-one.png"), width, height)?;
    let (top_edge, foot_edge) = edges(&first, deny_y);
    let (top_edge, _) = (
        top_edge.unwrap_or_else(|| panic!("the pinned card's top edge is inside the pane")),
        foot_edge.unwrap_or_else(|| panic!("the pinned card's foot is inside the pane")),
    );
    let before = denied.get();
    assert!(
        scan(w, deny_x, pane_top, top_edge as f32 - 2.0, || denied.get() > before).is_none(),
        "the transcript above the pinned card has no Decline: the waiting card there is a line"
    );

    // ── Items appended after the card: the card does not move, and Allow stays live ──
    for (i, n) in [(2, 30), (3, 60), (4, 90)] {
        items.push(growing(&format!("t1.{i}"), n));
    }
    items.push(item("text", "t1.5", "Still building; the council waits on your answer below."));
    let appended = away(w, width, height);
    save(&appended, &output.replace(".png", "-appended.png"), width, height)?;
    let rows = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>| -> Vec<slint::Rgb8Pixel> {
        p.as_slice()[(top_edge as usize - 2) * width as usize..pane_bottom as usize * width as usize].to_vec()
    };
    assert!(rows(&first) == rows(&appended), "the pinned card is drawn exactly where it was, whole, after items were appended");
    let n = grants();
    click(w, allow_x, allow_y);
    assert_eq!(grants(), n + 1, "nothing moved the card, so nothing holds its Allow");

    // ── A second waiting request: one card, "and 1 more", and one live Allow ──
    items.push(approval_item("t1.6", waiting("appr-2")));
    g.set_pinned_more(1);
    settle(w, width, height);
    // The count's line under the card lifts the card by its height and the gap: 16px + 4px.
    let moved = allow_y - 20.0;
    let n = grants();
    click(w, allow_x, moved);
    assert_eq!(grants(), n, "a click on Allow just after it moved grants nothing");
    rest(w, width, height);
    let (deny_y2, allow_y2) = find(w);
    assert!((allow_y2 - moved).abs() <= 1.0, "the card moved up by the count's line ({allow_y} → {allow_y2})");
    let two = settle(w, width, height);
    save(&two, &output.replace(".png", "-two.png"), width, height)?;
    let (top2, foot2) = edges(&two, deny_y2);
    assert!(top2.is_some() && foot2.is_some(), "the pinned card is whole with a second one waiting");
    // Every point of Allow's column in the transcript and in the card's button rows, pressed: the
    // grants come from one band, the pinned card's button, and nowhere in the transcript. (Not
    // the card's middle: "Details and source" there would open, move the buttons, and re-arm.)
    let top2 = top2.unwrap();
    let rows_pressed = (pane_top as u32..top2 - 2).chain(deny_y2 as u32 - 30..pane_bottom as u32).step_by(2);
    let mut bands: Vec<(f32, f32)> = Vec::new();
    for y in rows_pressed.map(|y| y as f32) {
        let n = grants();
        click(w, allow_x, y);
        if grants() > n {
            match bands.last_mut() {
                Some((_, end)) if y - *end <= 3.0 => *end = y,
                _ => bands.push((y, y)),
            }
        }
    }
    assert_eq!(bands.len(), 1, "one live Allow in the pane, not one per waiting card: {bands:?}");
    assert!(bands[0].0 <= allow_y2 && allow_y2 <= bands[0].1 + 2.0, "and it is the pinned card's");
    assert!(allowed.borrow().iter().all(|id| id == "appr-1"), "every grant so far was for the oldest request");

    // ── Answering the first brings up the second, and a resting pointer cannot answer it ──
    advance.set(true);
    click(w, allow_x, allow_y2);
    assert_eq!(last(), "appr-1");
    settle(w, width, height);
    let n = grants();
    click(w, allow_x, allow_y2);
    click(w, allow_x, allow_y);
    assert_eq!(grants(), n, "the next card's Allow takes no click in the moment after it comes up");
    rest(w, width, height);
    let (_, allow_y3) = find(w);
    assert!((allow_y3 - allow_y).abs() <= 1.0, "with nothing behind it the card is back where the first was");
    click(w, allow_x, allow_y3);
    assert_eq!(last(), "appr-2", "answering the first brought up the second");
    save(&settle(w, width, height), &output.replace(".png", "-next.png"), width, height)?;

    // ── The countdown ticking is no change; the window moving the card is ──
    g.set_pinned(ApprovalRequest { age_text: "Expires in 1 min, then declined".into(), ..waiting("appr-2") });
    settle(w, width, height);
    let n = grants();
    click(w, allow_x, allow_y3);
    assert_eq!(grants(), n + 1, "the countdown moves nothing, and holds nothing");
    w.set_size(slint::PhysicalSize::new(width, height - 20));
    settle(w, width, height - 20);
    let n = grants();
    click(w, allow_x, allow_y3 - 20.0);
    assert_eq!(grants(), n, "a click on Allow just after the window moved it grants nothing");
    rest(w, width, height - 20);
    click(w, allow_x, allow_y3 - 20.0);
    assert_eq!(grants(), n + 1, "after the moment, the same click answers");

    window.hide()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    println!(
        "PASS: the pane's approval card is pinned above its reply box: appended items leave it whole and live, \
         two waiting draw one card and one live Allow, answering the first brings up the second, and Allow \
         takes no click for a moment after the card moves or changes"
    );
    Ok(())
}
