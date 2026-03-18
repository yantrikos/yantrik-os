//! Yantrik Email — standalone app binary.
//!
//! Communicates with `email-service` via JSON-RPC IPC.
//! Falls back to stub/error UI when service is unavailable.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_transport::SyncRpcClient;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-email");

    let app = EmailApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

// ── Service wrappers ─────────────────────────────────────────────────

fn list_messages_via_service(folder: &str, page: u32) -> Option<Vec<yantrik_ipc_contracts::email::EmailSummary>> {
    let client = SyncRpcClient::for_service("email");
    let result = client
        .call("email.list_messages", serde_json::json!({ "folder": folder, "page": page }))
        .ok()?;
    serde_json::from_value(result).ok()
}

fn get_message_via_service(message_id: &str) -> Option<yantrik_ipc_contracts::email::EmailDetail> {
    let client = SyncRpcClient::for_service("email");
    let result = client
        .call("email.get_message", serde_json::json!({ "message_id": message_id }))
        .ok()?;
    serde_json::from_value(result).ok()
}

fn send_message_via_service(
    to: &str, cc: &str, bcc: &str, subject: &str, body: &str, reply_to: Option<&str>,
) -> Option<()> {
    let client = SyncRpcClient::for_service("email");
    let mut params = serde_json::json!({
        "to": to, "cc": cc, "bcc": bcc, "subject": subject, "body": body,
    });
    if let Some(rt) = reply_to {
        params["reply_to_id"] = serde_json::Value::String(rt.to_string());
    }
    client.call("email.send_message", params).ok()?;
    Some(())
}

fn search_via_service(query: &str) -> Option<Vec<yantrik_ipc_contracts::email::EmailSummary>> {
    let client = SyncRpcClient::for_service("email");
    let result = client
        .call("email.search", serde_json::json!({ "query": query }))
        .ok()?;
    serde_json::from_value(result).ok()
}

fn mark_read_via_service(message_id: &str, read: bool) -> Option<()> {
    let client = SyncRpcClient::for_service("email");
    client
        .call("email.mark_read", serde_json::json!({ "message_id": message_id, "read": read }))
        .ok()?;
    Some(())
}

fn mark_starred_via_service(message_id: &str, starred: bool) -> Option<()> {
    let client = SyncRpcClient::for_service("email");
    client
        .call("email.mark_starred", serde_json::json!({ "message_id": message_id, "starred": starred }))
        .ok()?;
    Some(())
}

fn delete_message_via_service(message_id: &str) -> Option<()> {
    let client = SyncRpcClient::for_service("email");
    client
        .call("email.delete_message", serde_json::json!({ "message_id": message_id }))
        .ok()?;
    Some(())
}

fn list_folders_via_service() -> Option<Vec<yantrik_ipc_contracts::email::EmailFolder>> {
    let client = SyncRpcClient::for_service("email");
    let result = client.call("email.list_folders", serde_json::json!({})).ok()?;
    serde_json::from_value(result).ok()
}

fn move_message_via_service(message_id: &str, target_folder: &str) -> Option<()> {
    let client = SyncRpcClient::for_service("email");
    client
        .call("email.move_message", serde_json::json!({ "message_id": message_id, "target_folder": target_folder }))
        .ok()?;
    Some(())
}

// ── Conversion helpers ───────────────────────────────────────────────

fn summary_to_list_item(s: &yantrik_ipc_contracts::email::EmailSummary, idx: usize) -> EmailListItem {
    let from_name = s.from.split('<').next().unwrap_or(&s.from).trim().to_string();
    let initial = from_name.chars().next().unwrap_or('?').to_uppercase().to_string();
    let colors = [
        slint::Color::from_rgb_u8(0x4E, 0x79, 0xA7),
        slint::Color::from_rgb_u8(0xF2, 0x8E, 0x2C),
        slint::Color::from_rgb_u8(0xE1, 0x57, 0x59),
        slint::Color::from_rgb_u8(0x76, 0xB7, 0xB2),
        slint::Color::from_rgb_u8(0x59, 0xA1, 0x4F),
    ];
    EmailListItem {
        id: idx as i32,
        from_name: from_name.into(),
        from_addr: s.from.clone().into(),
        subject: s.subject.clone().into(),
        preview: s.snippet.clone().into(),
        date_text: s.date.clone().into(),
        is_read: s.is_read,
        is_flagged: s.is_starred,
        is_selected: false,
        has_attachment: s.has_attachments,
        thread_count: 0,
        thread_id: s.thread_id.clone().unwrap_or_default().into(),
        avatar_initial: initial.into(),
        avatar_color: colors[idx % colors.len()],
    }
}

fn detail_to_ui(d: &yantrik_ipc_contracts::email::EmailDetail) -> EmailDetailData {
    let from_name = d.from.split('<').next().unwrap_or(&d.from).trim().to_string();
    let initial = from_name.chars().next().unwrap_or('?').to_uppercase().to_string();
    let body = if d.body_text.is_empty() { &d.body_html } else { &d.body_text };
    EmailDetailData {
        id: 0,
        from_name: from_name.into(),
        from_addr: d.from.clone().into(),
        from_initial: initial.into(),
        from_avatar_color: slint::Color::from_rgb_u8(0x4E, 0x79, 0xA7),
        to_addr: d.to.join(", ").into(),
        cc_addr: d.cc.join(", ").into(),
        subject: d.subject.clone().into(),
        date_text: d.date.clone().into(),
        body: body.clone().into(),
        ai_summary: SharedString::default(),
        is_flagged: false,
        is_read: true,
        has_attachment: !d.attachments.is_empty(),
        attachment_names: d.attachments.iter().map(|a| a.filename.clone()).collect::<Vec<_>>().join(", ").into(),
        thread_count: d.thread_messages.len() as i32,
    }
}

fn folder_to_ui(f: &yantrik_ipc_contracts::email::EmailFolder, idx: usize) -> EmailFolderData {
    let icon = match f.name.to_lowercase().as_str() {
        "inbox" => "\u{1F4E5}",
        "sent" | "sent mail" | "[gmail]/sent mail" => "\u{1F4E4}",
        "drafts" | "[gmail]/drafts" => "\u{1F4DD}",
        "trash" | "[gmail]/trash" => "\u{1F5D1}\u{FE0F}",
        "spam" | "[gmail]/spam" | "junk" => "\u{26A0}\u{FE0F}",
        "starred" | "[gmail]/starred" => "\u{2B50}",
        "archive" | "[gmail]/all mail" => "\u{1F4E6}",
        _ => "\u{1F4C1}",
    };
    let folder_type = match f.name.to_lowercase().as_str() {
        "inbox" => "inbox",
        s if s.contains("sent") => "sent",
        s if s.contains("draft") => "drafts",
        s if s.contains("trash") => "trash",
        s if s.contains("spam") || s.contains("junk") => "spam",
        s if s.contains("starred") => "starred",
        s if s.contains("archive") || s.contains("all mail") => "archive",
        _ => "custom",
    };
    EmailFolderData {
        name: f.name.clone().into(),
        icon: icon.into(),
        unread_count: f.unread_count,
        total_count: f.total_count,
        is_selected: idx == 0,
        folder_type: folder_type.into(),
    }
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &EmailApp) {
    let email_ids: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

    // Initial load — try to populate folders and inbox
    if let Some(folders) = list_folders_via_service() {
        let folder_models: Vec<EmailFolderData> = folders.iter().enumerate()
            .map(|(i, f)| folder_to_ui(f, i)).collect();
        app.set_folders(ModelRc::new(VecModel::from(folder_models)));
        app.set_has_account(true);
        // Load inbox
        load_folder(app, &email_ids, "INBOX", 0);
    } else {
        app.set_has_account(false);
    }

    // ── Folder clicked ──
    {
        let weak = app.as_weak();
        let ids = email_ids.clone();
        app.on_folder_clicked(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_folders();
            if idx < 0 || idx as usize >= model.row_count() { return; }
            let folder = model.row_data(idx as usize).unwrap();
            let folder_name = folder.name.to_string();
            ui.set_selected_folder_index(idx);
            load_folder(&ui, &ids, &folder_name, 0);
        });
    }

    // ── Email selected ──
    {
        let weak = app.as_weak();
        let ids = email_ids.clone();
        app.on_email_selected(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let id_list = ids.borrow();
            let idx = idx as usize;
            if idx >= id_list.len() { return; }
            let msg_id = &id_list[idx];

            if let Some(detail) = get_message_via_service(msg_id) {
                ui.set_email_detail(detail_to_ui(&detail));
                // Mark as read
                let _ = mark_read_via_service(msg_id, true);
                // Update thread messages
                let thread: Vec<EmailThreadMessage> = detail.thread_messages.iter()
                    .map(|t| EmailThreadMessage {
                        id: 0,
                        from_name: t.from.clone().into(),
                        from_addr: t.from.clone().into(),
                        date_text: t.date.clone().into(),
                        body: t.snippet.clone().into(),
                        is_collapsed: true,
                    }).collect();
                ui.set_email_thread_messages(ModelRc::new(VecModel::from(thread)));
            }
        });
    }

    // ── Compose new ──
    {
        let weak = app.as_weak();
        app.on_compose_new(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_is_composing(true);
                ui.set_compose_to(SharedString::default());
                ui.set_compose_cc(SharedString::default());
                ui.set_compose_bcc(SharedString::default());
                ui.set_compose_subject(SharedString::default());
                ui.set_compose_body(SharedString::default());
            }
        });
    }

    // ── Send email ──
    {
        let weak = app.as_weak();
        let ids = email_ids.clone();
        app.on_send_email(move |to, cc, bcc, subject, body| {
            let Some(ui) = weak.upgrade() else { return };
            if send_message_via_service(&to, &cc, &bcc, &subject, &body, None).is_some() {
                ui.set_is_composing(false);
                ui.set_email_sync_status("Message sent".into());
                load_folder(&ui, &ids, "INBOX", 0);
            } else {
                ui.set_email_sync_status("Send failed — service unavailable".into());
            }
        });
    }

    // ── Cancel compose ──
    {
        let weak = app.as_weak();
        app.on_cancel_compose(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_is_composing(false);
            }
        });
    }

    // ── Delete email ──
    {
        let weak = app.as_weak();
        let ids = email_ids.clone();
        app.on_delete_email(move |idx| {
            let id_list = ids.borrow();
            let idx = idx as usize;
            if idx >= id_list.len() { return; }
            let msg_id = id_list[idx].clone();
            drop(id_list);
            if delete_message_via_service(&msg_id).is_some() {
                if let Some(ui) = weak.upgrade() {
                    load_folder(&ui, &ids, "INBOX", 0);
                }
            }
        });
    }

    // ── Archive email ──
    {
        let weak = app.as_weak();
        let ids = email_ids.clone();
        app.on_archive_email(move |idx| {
            let id_list = ids.borrow();
            let idx = idx as usize;
            if idx >= id_list.len() { return; }
            let msg_id = id_list[idx].clone();
            drop(id_list);
            if move_message_via_service(&msg_id, "[Gmail]/All Mail").is_some() {
                if let Some(ui) = weak.upgrade() {
                    load_folder(&ui, &ids, "INBOX", 0);
                }
            }
        });
    }

    // ── Mark read ──
    {
        let ids = email_ids.clone();
        app.on_mark_read(move |idx| {
            let id_list = ids.borrow();
            let idx = idx as usize;
            if idx >= id_list.len() { return; }
            let _ = mark_read_via_service(&id_list[idx], true);
        });
    }

    // ── Mark flagged ──
    {
        let ids = email_ids.clone();
        app.on_mark_flagged(move |idx| {
            let id_list = ids.borrow();
            let idx = idx as usize;
            if idx >= id_list.len() { return; }
            let _ = mark_starred_via_service(&id_list[idx], true);
        });
    }

    // ── Search ──
    {
        let weak = app.as_weak();
        let ids = email_ids.clone();
        app.on_search_emails(move |query| {
            let Some(ui) = weak.upgrade() else { return };
            let q = query.to_string();
            if q.is_empty() {
                ui.set_email_search_active(false);
                load_folder(&ui, &ids, "INBOX", 0);
                return;
            }
            ui.set_email_search_active(true);
            if let Some(results) = search_via_service(&q) {
                let mut new_ids = ids.borrow_mut();
                new_ids.clear();
                let items: Vec<EmailListItem> = results.iter().enumerate()
                    .map(|(i, s)| {
                        new_ids.push(s.id.clone());
                        summary_to_list_item(s, i)
                    }).collect();
                ui.set_email_search_count(items.len() as i32);
                ui.set_email_list(ModelRc::new(VecModel::from(items)));
            }
        });
    }

    // ── Sync ──
    {
        let weak = app.as_weak();
        let ids = email_ids.clone();
        app.on_sync_emails(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_email_sync_status("Syncing...".into());
                load_folder(&ui, &ids, "INBOX", 0);
                ui.set_email_sync_status("Synced".into());
            }
        });
    }

    // ── Toggle thread message ──
    {
        let weak = app.as_weak();
        app.on_toggle_thread_message(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_email_thread_messages();
            let idx = idx as usize;
            if idx >= model.row_count() { return; }
            if let Some(mut msg) = model.row_data(idx) {
                msg.is_collapsed = !msg.is_collapsed;
                if let Some(vec_model) = model.as_any().downcast_ref::<VecModel<EmailThreadMessage>>() {
                    vec_model.set_row_data(idx, msg);
                }
            }
        });
    }

    // ── Stubs for features requiring companion bridge or complex setup ──
    app.on_reply_email(|| { tracing::info!("Reply (standalone mode)"); });
    app.on_reply_all_email(|| { tracing::info!("Reply-all (standalone mode)"); });
    app.on_forward_email(|| { tracing::info!("Forward (standalone mode)"); });
    app.on_summarize_email(|| { tracing::info!("AI summarize (standalone mode)"); });
    app.on_enhance_text(|_| { tracing::info!("AI enhance (standalone mode)"); });
    app.on_ai_draft(|_| { tracing::info!("AI draft (standalone mode)"); });
    app.on_ai_reply_suggest(|| { tracing::info!("AI reply suggest (standalone mode)"); });
    app.on_back_pressed(|| {});
    app.on_add_account(|| { tracing::info!("Add account (standalone mode)"); });
    app.on_download_attachment(|_| { tracing::info!("Download attachment (standalone mode)"); });
    app.on_preview_attachment(|_| { tracing::info!("Preview attachment (standalone mode)"); });
    app.on_set_signature(|_| { tracing::info!("Set signature (standalone mode)"); });
    app.on_triage_filter(|_| { tracing::info!("Triage filter (standalone mode)"); });
    app.on_save_draft(|| { tracing::info!("Save draft (standalone mode)"); });
    app.on_ai_classify(|| { tracing::info!("AI classify (standalone mode)"); });
    app.on_save_account(|_,_,_,_,_,_,_,_| { tracing::info!("Save account (standalone mode)"); });
    app.on_test_connection(|_,_,_,_,_| { tracing::info!("Test connection (standalone mode)"); });
    app.on_oauth_google(|| { tracing::info!("OAuth Google (standalone mode)"); });
}

fn load_folder(
    ui: &EmailApp,
    ids: &std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    folder: &str,
    page: u32,
) {
    ui.set_is_loading(true);
    if let Some(summaries) = list_messages_via_service(folder, page) {
        let mut new_ids = ids.borrow_mut();
        new_ids.clear();
        let items: Vec<EmailListItem> = summaries.iter().enumerate()
            .map(|(i, s)| {
                new_ids.push(s.id.clone());
                summary_to_list_item(s, i)
            }).collect();
        let total = items.len() as i32;
        let unread = items.iter().filter(|e| !e.is_read).count() as i32;
        ui.set_email_list(ModelRc::new(VecModel::from(items)));
        ui.set_email_folder_total(total);
        ui.set_email_folder_unread(unread);
        ui.set_email_sync_status("Synced".into());
    } else {
        ui.set_email_sync_status("Service unavailable".into());
    }
    ui.set_is_loading(false);
}
