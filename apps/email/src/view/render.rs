//! The only code that writes the window, by the part of the model that changed.

use slint::{Color, ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::for_mind::shown_if_any;

use crate::model::{part, Clear, Model, Rgb, OPEN_MESSAGE};
use crate::state::{self, Counted};
use crate::{EmailApp, EmailAttachmentData, EmailDetailData, EmailFolderData, EmailListItem, EmailThreadMessage};

fn color((r, g, b): Rgb) -> Color {
    Color::from_rgb_u8(r, g, b)
}

pub fn render(ui: &EmailApp, m: &Model, changed: &Changed) {
    if changed.has(part::STATE) {
        state(ui, m);
    }
    if changed.has(part::FOLDERS) {
        folders(ui, m);
    }
    if changed.has(part::LIST) || changed.has(part::FOLDERS) {
        list(ui, m);
    }
    if changed.has(part::OPEN) {
        open(ui, m);
    }
    if changed.has(part::STATUS) {
        status(ui, m);
    }
    if changed.has(part::COMPOSE_TEXT) {
        let text = &m.composer.text;
        let hide = |field: &str| SharedString::from(shown_if_any(m.for_mind, field).as_ref());
        ui.set_compose_to(hide(&text.to));
        ui.set_compose_cc(hide(&text.cc));
        ui.set_compose_bcc(hide(&text.bcc));
        ui.set_compose_subject(hide(&text.subject));
        ui.set_compose_body(hide(&text.body));
    }
    if changed.has(part::SEARCH_TEXT) {
        ui.set_email_search_query(m.search.query.clone().into());
    }
    if changed.has(part::SETUP) || changed.has(part::STATE) {
        setup(ui, m);
    }
    if changed.has(part::SETUP_FIELDS) {
        setup_fields(ui, m.setup.clear);
    }
}

/// Which of the three states the service is in, and what the setup screen says about it.
fn state(ui: &EmailApp, m: &Model) {
    // Until the first look lands the window shows the mailbox, loading: it has not been told yet
    // whether there is an account, and the setup form would be a claim that there is none.
    ui.set_has_account(if m.looked { m.state.has_account().unwrap_or(false) } else { true });
    ui.set_service_state(m.state.service_word().into());
    ui.set_account_name(m.account_name.clone().into());
    ui.set_setup_note(state::password_storage_note(m.state.config_path(), m.state.secrets_are_plaintext()).into());
    // Whether the setup screen can offer a Google sign-in at all, in the service's own words.
    // The button is not drawn unless this is true, because a "Sign in with Google" that cannot
    // work is the dead control this flow replaced — and the note is set either way, so a build
    // with no OAuth client says so instead of showing an unexplained gap.
    let google = m.state.google_sign_in();
    ui.set_google_available(google.available);
    ui.set_google_note(google.note.into());
}

fn folders(ui: &EmailApp, m: &Model) {
    let rows: Vec<EmailFolderData> = m
        .folders
        .iter()
        .map(|f| {
            let counts = f.counts.unwrap_or_default();
            EmailFolderData {
                name: f.name.clone().into(),
                label: f.label.clone().into(),
                icon: f.icon.clone().into(),
                unread_count: counts.unread,
                total_count: counts.total,
                counts_known: f.counts.is_some(),
                counts_reason: f.reason.clone().into(),
                is_selected: f.selected,
                folder_type: f.kind.clone().into(),
            }
        })
        .collect();
    ui.set_folders(ModelRc::new(VecModel::from(rows)));
    ui.set_selected_folder_index(m.selected_folder as i32);
}

/// The rows the triage tab keeps, and the open folder's counts on the header. The counts are of
/// the folder, not of the tab: "3 unread of 128" is about the mailbox, and it would be a strange
/// thing for pressing Unread to change.
fn list(ui: &EmailApp, m: &Model) {
    let hide = |text: &str| SharedString::from(shown_if_any(m.for_mind, text).as_ref());
    let items: Vec<EmailListItem> = m
        .shown()
        .into_iter()
        .map(|(key, r)| {
            let it = &r.item;
            EmailListItem {
                id: key,
                from_name: hide(&it.from_name),
                from_addr: hide(&it.from_addr),
                subject: hide(&it.subject),
                preview: hide(&it.preview),
                date_text: it.date_text.clone().into(),
                is_read: it.is_read,
                is_flagged: it.is_flagged,
                is_selected: it.is_selected,
                has_attachment: it.has_attachment,
                thread_count: it.thread_count,
                thread_id: it.thread_id.clone().into(),
                avatar_initial: hide(&it.avatar_initial),
                avatar_color: color(it.avatar),
            }
        })
        .collect();
    ui.set_email_triage_view(m.triage.index());
    ui.set_email_list(ModelRc::new(VecModel::from(items)));
    match m.header() {
        Counted::Known(counts) => {
            ui.set_email_folder_unread(counts.unread);
            ui.set_email_folder_total(counts.total);
            ui.set_email_folder_counts_known(true);
            ui.set_email_folder_counts_reason(SharedString::default());
        }
        Counted::Unavailable { reason } => {
            // The two numbers mean nothing with `known` false; zero is what they hold, not what
            // they say, and nothing reads them without the flag.
            ui.set_email_folder_unread(0);
            ui.set_email_folder_total(0);
            ui.set_email_folder_counts_known(false);
            ui.set_email_folder_counts_reason(reason.into());
        }
    }
}

/// The reading pane. Its id is `OPEN_MESSAGE` while a message is open, so the toolbar's buttons
/// act on that message; 0 draws the empty pane.
fn open(ui: &EmailApp, m: &Model) {
    let Some(open) = &m.open else {
        ui.set_email_detail(EmailDetailData::default());
        ui.set_email_attachments(ModelRc::new(VecModel::<EmailAttachmentData>::from(Vec::new())));
        ui.set_email_thread_messages(ModelRc::new(VecModel::<EmailThreadMessage>::from(Vec::new())));
        return;
    };
    let d = &open.detail;
    let hide = |text: &str| SharedString::from(shown_if_any(m.for_mind, text).as_ref());
    ui.set_email_detail(EmailDetailData {
        id: OPEN_MESSAGE,
        from_name: hide(&d.from_name),
        from_addr: hide(&d.from_addr),
        from_initial: hide(&d.from_initial),
        from_avatar_color: color(d.avatar),
        to_addr: hide(&d.to_addr),
        cc_addr: hide(&d.cc_addr),
        subject: hide(&d.subject),
        date_text: d.date_text.clone().into(),
        body: hide(&d.body),
        ai_summary: hide(&d.ai_summary),
        is_flagged: d.is_flagged,
        is_read: d.is_read,
        has_attachment: d.has_attachment,
        attachment_names: hide(&d.attachment_names),
        thread_count: d.thread_count,
        has_html: d.has_html,
    });
    let attachments: Vec<EmailAttachmentData> = open
        .attachments
        .iter()
        .map(|a| EmailAttachmentData {
            name: hide(&a.name),
            size_text: a.size_text.clone().into(),
            mime_type: a.mime_type.clone().into(),
            is_downloaded: a.is_downloaded,
        })
        .collect();
    ui.set_email_attachments(ModelRc::new(VecModel::from(attachments)));
    // Each entry's id is its place in the thread, which is what collapsing one names.
    let thread: Vec<EmailThreadMessage> = open
        .thread
        .iter()
        .enumerate()
        .map(|(i, t)| EmailThreadMessage {
            id: i as i32,
            from_name: hide(&t.from_name),
            from_addr: hide(&t.from_addr),
            date_text: t.date_text.clone().into(),
            body: hide(&t.body),
            is_collapsed: t.is_collapsed,
        })
        .collect();
    ui.set_email_thread_messages(ModelRc::new(VecModel::from(thread)));
}

fn status(ui: &EmailApp, m: &Model) {
    ui.set_notice(m.notice.clone().into());
    ui.set_email_sync_status(m.sync_status.clone().into());
    ui.set_is_loading(m.loading);
    ui.set_ai_is_working(m.ai_working);
    ui.set_email_ai_intent(m.ai_intent.clone().into());
    ui.set_is_composing(m.composer.open);
    ui.set_is_sending(m.composer.sending.is_some());
    ui.set_email_draft_status(m.composer.status.clone().into());
    ui.set_email_search_active(m.search.active);
    ui.set_email_search_count(m.search.count);
}

fn setup(ui: &EmailApp, m: &Model) {
    ui.set_setup_open(m.setup.open);
    ui.set_setup_status(m.setup.status.clone().into());
    ui.set_setup_ok(m.setup.ok);
    ui.set_setup_testing(m.setup.testing);
    ui.set_google_waiting(m.google.waiting);
    ui.set_google_url(m.google.url.clone().into());
}

/// The form's own fields, cleared when the model says: the password the moment it is not needed,
/// every identity field when the form opens for another account.
fn setup_fields(ui: &EmailApp, clear: Clear) {
    match clear {
        Clear::Nothing => {}
        Clear::Password => ui.set_setup_password(SharedString::default()),
        Clear::Fresh => {
            ui.set_setup_email(SharedString::default());
            ui.set_setup_password(SharedString::default());
            ui.set_setup_display_name(SharedString::default());
            ui.set_setup_provider(SharedString::default());
            ui.set_setup_advanced_mode(false);
        }
    }
}
