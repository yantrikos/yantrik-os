//! The window: its callbacks bound to commands (`wire`), and the only code that writes it
//! (`render`).

use slint::SharedString;
use yantrik_app::cx::Cx;

use crate::command::{Ask, Command, Flag, Pick, SetupForm};
use crate::state::Draft;
use crate::{Email, EmailApp};

pub mod render;

/// What is in the composer right now.
fn draft_on_screen(ui: &EmailApp) -> Draft {
    Draft {
        to: ui.get_compose_to().to_string(),
        cc: ui.get_compose_cc().to_string(),
        bcc: ui.get_compose_bcc().to_string(),
        subject: ui.get_compose_subject().to_string(),
        body: ui.get_compose_body().to_string(),
    }
}

pub fn wire(cx: &Cx<Email>, ui: &EmailApp) {
    // The mailbox.
    ui.on_sync_emails(cx.cmd(|| Command::Look(crate::command::Look::Sync)));
    ui.on_folder_clicked(cx.cmd1(Command::FolderClicked));
    ui.on_search_emails(cx.cmd1(|query: SharedString| Command::Search(query.to_string())));
    ui.on_triage_filter(cx.cmd1(Command::Triage));

    // A message. A row sends its place in the listing; the reading pane's toolbar sends
    // `OPEN_MESSAGE`, the message that is open.
    ui.on_email_selected(cx.cmd1(|key: i32| Command::Open(Pick::Key(key))));
    ui.on_mark_read(cx.cmd1(|key: i32| Command::Mark { pick: Pick::Key(key), flag: Flag::Read, toggle: false }));
    ui.on_mark_flagged(cx.cmd1(|key: i32| Command::Mark { pick: Pick::Key(key), flag: Flag::Starred, toggle: true }));
    ui.on_delete_email(cx.cmd1(Command::Delete));
    ui.on_archive_email(cx.cmd1(Command::Archive));
    ui.on_toggle_thread_message(cx.cmd1(Command::ToggleThread));
    ui.on_open_original(cx.cmd(|| Command::OpenOriginal));

    // Writing. The composer's fields are read off the window when a button needs them, because
    // they are the window's while the person types; `edited` keeps the model's copy current for
    // `describe`.
    ui.on_compose_new(cx.cmd(|| Command::ComposeNew));
    ui.on_reply_email(cx.cmd(|| Command::Reply { all: false }));
    ui.on_reply_all_email(cx.cmd(|| Command::Reply { all: true }));
    ui.on_forward_email(cx.cmd(|| Command::Forward));
    let send = cx.clone();
    ui.on_send_email(move |to, cc, bcc, subject, body| {
        send.run(Command::Send(Draft {
            to: to.to_string(),
            cc: cc.to_string(),
            bcc: bcc.to_string(),
            subject: subject.to_string(),
            body: body.to_string(),
        }));
    });
    let cancel = cx.clone();
    ui.on_cancel_compose(move || {
        let draft = draft_on_screen(&cancel.ui());
        cancel.run(Command::CancelCompose(draft));
    });
    let save = cx.clone();
    ui.on_save_draft(move || {
        let draft = draft_on_screen(&save.ui());
        save.run(Command::SaveDraft(draft));
    });
    let edited = cx.clone();
    ui.on_edited(move || {
        let (text, query) = {
            let ui = edited.ui();
            (draft_on_screen(&ui), ui.get_email_search_query().to_string())
        };
        edited.run(Command::Edited { text, query });
    });

    // The companion.
    ui.on_summarize_email(cx.cmd(|| Command::Ask(Ask::Summarize)));
    ui.on_ai_reply_suggest(cx.cmd(|| Command::Ask(Ask::SuggestReply)));
    ui.on_ai_classify(cx.cmd(|| Command::Ask(Ask::Classify)));
    ui.on_ai_draft(cx.cmd1(|instruction: SharedString| Command::Ask(Ask::Draft(instruction.to_string()))));
    // The button names a tone; the body is the composer's, read by the command.
    ui.on_enhance_text(cx.cmd1(|style: SharedString| Command::Ask(Ask::Enhance(style.to_string()))));

    // Account setup. Test Connection is given five of the eight fields by the screen; the rest
    // are read off the window, so both halves of the account are tried by the one button.
    ui.on_add_account(cx.cmd(|| Command::AddAccount));
    ui.on_cancel_setup(cx.cmd(|| Command::CancelSetup));
    ui.on_sign_in_with_google(cx.cmd(|| Command::SignInWithGoogle));
    ui.on_cancel_google_sign_in(cx.cmd(|| Command::CancelGoogle));
    let test = cx.clone();
    ui.on_test_connection(move |email, password, provider, imap_server, imap_port| {
        let form = {
            let ui = test.ui();
            SetupForm {
                email: email.to_string(),
                password: password.to_string(),
                display_name: ui.get_setup_display_name().to_string(),
                provider: provider.to_string(),
                imap_server: imap_server.to_string(),
                imap_port: imap_port.to_string(),
                smtp_server: ui.get_setup_smtp_server().to_string(),
                smtp_port: ui.get_setup_smtp_port().to_string(),
            }
        };
        test.run(Command::TestConnection(form));
    });
    let save_account = cx.clone();
    ui.on_save_account(move |email, password, display_name, provider, imap_server, imap_port, smtp_server, smtp_port| {
        save_account.run(Command::SaveAccount(SetupForm {
            email: email.to_string(),
            password: password.to_string(),
            display_name: display_name.to_string(),
            provider: provider.to_string(),
            imap_server: imap_server.to_string(),
            imap_port: imap_port.to_string(),
            smtp_server: smtp_server.to_string(),
            smtp_port: smtp_port.to_string(),
        }));
    });
}
