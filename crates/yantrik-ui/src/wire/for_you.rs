//! The Lens's For you inbox on screen: its cards, their one button, the person's feedback, and the
//! toast a granted interrupt gets (design/initiative-inbox-2026-10-16.md).
//!
//! The rules are `notice_inbox`'s; this file applies them once a second, on the notification
//! poll's tick, from the mirror the poll keeps. Talking to the store goes through the same
//! fire-and-forget calls the notification centre uses, off the UI thread.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_ipc_contracts::notifications::notice::{self, FeedbackKind};
use yantrik_ipc_contracts::notifications::{Notification, NOTICE_FEEDBACK, NOTICE_SHOWN};

use super::notifications::{act, forward_to_app, resync, with_mirror, SILENT_SCREENS};
use crate::notice_inbox::{self, Opening, Screen, Toast};
use crate::{App, InboxState, NoticeCardData, ToastData};

thread_local! {
    /// Cards toasted by this shell, so a card is never toasted twice while the store's record of
    /// it is on its way back through the poll.
    static TOASTED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Cards recorded as seen in the inbox, for the same reason.
    static SEEN: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// A line under a card: what its button or a feedback press did.
    static NOTES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// What the section last drew, so an unchanged second does not rebuild it.
    static DRAWN: RefCell<Vec<NoticeCardData>> = const { RefCell::new(Vec::new()) };
}

pub fn wire(ui: &App) {
    let state = ui.global::<InboxState>();

    let weak = ui.as_weak();
    state.on_open_action(move |id| {
        if let Some(ui) = weak.upgrade() {
            open_action(&ui, id.as_str());
        }
    });

    let weak = ui.as_weak();
    state.on_feedback(move |id, kind| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(kind) = FeedbackKind::parse(kind.as_str()).filter(|k| FeedbackKind::BUTTONS.contains(k)) else {
            return;
        };
        let id = id.to_string();
        // Pressed on the desktop's own screen and nowhere else: the store takes feedback only
        // from the desktop, so a mind cannot mark its own card useful.
        act(NOTICE_FEEDBACK, serde_json::json!({ "id": id, "kind": kind.as_str() }));
        with_mirror(|m| {
            if kind.takes_the_card_away() {
                m.dismiss_locally(&id);
            } else {
                m.mark_read_locally(&id);
            }
        });
        let said = match kind {
            FeedbackKind::NeverFamily => "This kind is off now. Settings › Harnesses › Initiative turns it back on.",
            FeedbackKind::InboxOnly => "This kind waits here from now on. Settings › Harnesses › Initiative changes it.",
            _ => "Thanks. The mind that sent it will hear this.",
        };
        NOTES.with(|n| n.borrow_mut().insert(id.clone(), said.to_string()));
        crate::wire::toast::remove(&ui, &id);
        resync(&ui);
        tick(&ui);
    });

    let weak = ui.as_weak();
    state.on_open_centre(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_current_screen(9);
            ui.invoke_navigate(9);
        }
    });
}

/// Once a second, after the poll: draw the cards, toast what may be toasted, and record what the
/// person has seen.
pub fn tick(ui: &App) {
    let now = chrono::Utc::now();
    let screen = Screen {
        silent: SILENT_SCREENS.contains(&ui.get_current_screen()),
        do_not_disturb: ui.get_dnd_mode(),
        focus: ui.get_focus_mode(),
        // No signal on this desktop says either yet (see `notice_inbox::Screen`).
        fullscreen: false,
        call: false,
    };
    let (cards, more, due) = with_mirror(|m| {
        let live = notice_inbox::cards(m.all(), now);
        let more = live.len().saturating_sub(notice_inbox::SHOWN);
        let shown: Vec<NoticeCardData> = live.iter().take(notice_inbox::SHOWN).map(|n| card(n)).collect();
        let due: Vec<Notification> = m
            .all()
            .filter(|n| notice_inbox::toast(n, screen, now) == Toast::Now)
            .filter(|n| !TOASTED.with(|t| t.borrow().contains(&n.id)))
            .cloned()
            .collect();
        (shown, more, due)
    })
    .unwrap_or_default();

    for n in &due {
        toast(ui, n);
    }

    // Seen in the inbox: the Lens is open and the cards are on screen (its own page, or the
    // conversation with the section unfolded). A card seen there is not toasted afterwards.
    let state = ui.global::<InboxState>();
    let visible = ui.get_lens_open() && (!ui.get_lens_chat_mode() || state.get_expanded());
    if visible {
        for c in &cards {
            if SEEN.with(|s| s.borrow_mut().insert(c.id.to_string())) && !TOASTED.with(|t| t.borrow().contains(c.id.as_str())) {
                act(NOTICE_SHOWN, serde_json::json!({ "id": c.id.as_str(), "shown_as": "inbox" }));
            }
        }
    }

    let changed = DRAWN.with(|d| {
        let mut d = d.borrow_mut();
        if *d == cards {
            return false;
        }
        *d = cards.clone();
        true
    });
    if changed {
        state.set_cards(ModelRc::new(VecModel::from(cards)));
    }
    if state.get_more() != more as i32 {
        state.set_more(more as i32);
    }
    crate::wire::initiative::refresh(ui);
}

/// One card as the section draws it. Plain text all the way: the words are a mind's.
fn card(n: &Notification) -> NoticeCardData {
    let Some(notice) = &n.notice else { return NoticeCardData::default() };
    NoticeCardData {
        id: n.id.clone().into(),
        title: crate::notification_sender::one_line(&n.title).into(),
        body: notice::plain(&n.body, notice::MAX_BODY_NEWLINES).into(),
        why: notice_inbox::why_line(n).into(),
        sender: crate::notifications::sender_line(n).into(),
        family: notice.family.label().to_string().into(),
        action_label: notice.action.as_ref().map(notice_inbox::button_label).unwrap_or_default().into(),
        interrupt: notice_inbox::granted_interrupt(n),
        unread: !n.read,
        note: NOTES.with(|notes| notes.borrow().get(&n.id).cloned()).unwrap_or_default().into(),
    }
}

/// Toast a granted interrupt, and tell the store it was shown that way.
fn toast(ui: &App, n: &Notification) {
    TOASTED.with(|t| t.borrow_mut().insert(n.id.clone()));
    act(NOTICE_SHOWN, serde_json::json!({ "id": n.id, "shown_as": "toast" }));
    let name = crate::notifications::sender_name(n);
    crate::wire::toast::push(
        ui,
        ToastData {
            id: n.id.clone().into(),
            app_name: name.clone().into(),
            summary: crate::notification_sender::one_line(&n.title).into(),
            body: super::toast::brief(&notice::plain(&n.body, 0)).into(),
            urgency: 1,
            icon_char: name.chars().next().unwrap_or('N').to_uppercase().to_string().into(),
            // The card's button is pressed from the card, in the Lens, where "Why you got this"
            // is beside it; a toast's buttons forward to an app's control surface, which a
            // mind's card never may.
            actions: ModelRc::new(VecModel::from(Vec::new())),
            // A picture is only the desktop's own capture; a mind's card never shows one.
            thumbnail: Default::default(),
            has_thumbnail: false,
        },
        1,
    );
}

/// Press a card's one button: open what it names, as the person, if its check still holds.
fn open_action(ui: &App, id: &str) {
    if crate::control::locked_screen(ui.get_current_screen()) {
        return;
    }
    let Some(action) = with_mirror(|m| m.get(id).and_then(|n| n.notice.as_ref()).and_then(|c| c.action.clone())).flatten() else {
        return;
    };
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let note = |text: &str| NOTES.with(|n| n.borrow_mut().insert(id.to_string(), text.to_string()));
    match notice_inbox::plan(&action, &home) {
        Err(why) => {
            note(&format!("Not opened: {why}."));
        }
        Ok(Opening::File(path)) => {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            use crate::mime_dispatch::FileAction;
            let how = match crate::mime_dispatch::classify(&name) {
                // Never the browser: it is started with the DevTools flags that let `yos web` drive
                // the page, and a mind that can read the screen would be reading the file. A page
                // or a drawing a card names is opened as the text it is.
                FileAction::Browser => FileAction::TextEditor,
                other => other,
            };
            // Every kind opens a read-only copy (`notice_inbox::read_only_copy`): the spec opens a
            // card's file read-only, and an app handed the original would resolve the path again
            // later, after the check, when a component could have become a symlink.
            let opened = card_copies()
                .ok_or_else(|| "there is no private place to copy it to (XDG_RUNTIME_DIR is not set)".to_string())
                .and_then(|into| notice_inbox::read_only_copy(&path, &into, &format!("{id}-{name}")));
            match opened {
                Ok(target) => super::open_with::launch(&how, &name, &target, Some(ui)),
                Err(why) => {
                    note(&format!("Not opened: {why}."));
                }
            }
        }
        Ok(Opening::Url { url, host }) => {
            let host2 = host.clone();
            let _ = std::thread::Builder::new().name("yos-notice-url".into()).spawn(move || {
                if let Err(e) = crate::open_url::open(&url) {
                    tracing::warn!(host = %host2, error = %e, "a card's link could not be opened");
                }
            });
            note(&format!("Opened {host} in the browser."));
        }
        Ok(Opening::Note(reference)) => {
            forward_to_app(ui, "notes", "open_note", Some(serde_json::json!({ "title": reference })));
        }
        Ok(Opening::Event(reference)) => open_event(ui, id, reference),
    }
    act(NOTICE_SHOWN, serde_json::json!({ "id": id, "opened": true }));
    with_mirror(|m| m.mark_read_locally(id));
    resync(ui);
    tick(ui);
}

/// Where a card's read-only copies go: this account's runtime directory, which is in memory, only
/// this account's, and gone at logout. Never the shared temp directory: `None` refuses instead.
fn card_copies() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(|d| std::path::PathBuf::from(d).join("yantrik-card-files"))
}

/// Show an event in Calendar, once Calendar says it knows the id. Asked off the UI thread.
fn open_event(ui: &App, card: &str, event: String) {
    let weak = ui.as_weak();
    let card = card.to_string();
    let _ = std::thread::Builder::new().name("yos-notice-event".into()).spawn(move || {
        use yantrik_ipc_contracts::calendar::method::GET_EVENT;
        let found = yantrik_ipc_transport::SyncRpcClient::for_service("calendar")
            .with_timeout(std::time::Duration::from_secs(3))
            .call(GET_EVENT, serde_json::json!({ "id": event }))
            .ok()
            .and_then(|e| e["start"].as_str().map(|s| s.chars().take(10).collect::<String>()))
            .filter(|d| d.len() == 10);
        let _ = weak.upgrade_in_event_loop(move |ui| match found {
            Some(date) => forward_to_app(&ui, "calendar", "show_date", Some(serde_json::json!({ "date": date }))),
            None => {
                NOTES.with(|n| n.borrow_mut().insert(card, "Not opened: Calendar does not know this event.".into()));
                DRAWN.with(|d| d.borrow_mut().clear());
                tick(&ui);
            }
        });
    });
}

/// What `describe shell` says about the inbox.
pub fn describe() -> serde_json::Value {
    with_mirror(|m| notice_inbox::describe(m.all(), chrono::Utc::now())).unwrap_or_else(|| serde_json::json!({ "count": 0, "unread": 0, "newest": [] }))
}
