//! Chat bridge — wires yantrik-chat into the companion.
//!
//! Starts configured providers, connects the ChatRouter to the companion via
//! callback closures, and feeds all events into the brain.
//!
//! The caller (UI crate) provides the AI and brain callbacks since CompanionService
//! is owned by the worker thread and accessed via command channels.

use std::sync::{Arc, Mutex};
use std::thread;

use rusqlite::Connection;

use yantrik_chat::manager::ProviderManager;
use yantrik_chat::router::{AiCallback, BrainCallback, ChatRouter, RouterEvent};

use crate::config::CompanionConfig;

/// Handle to the running chat system. Drop to stop all providers.
pub struct ChatHandle {
    _router_thread: Option<thread::JoinHandle<()>>,
    _manager: ProviderManager,
    outbox: yantrik_chat::router::Outbox,
}

impl ChatHandle {
    /// What sends to a conversation on a channel at any time.
    pub fn outbox(&self) -> yantrik_chat::router::Outbox {
        self.outbox.clone()
    }
}

/// Start the multi-provider chat system.
///
/// `ai_callback`: Called when a message needs an AI response.
/// `brain_callback`: Called for every non-muted message (brain feeding).
///
/// Returns `None` if no providers are enabled.
pub fn start_chat(
    config: &CompanionConfig,
    ai_callback: AiCallback,
    brain_callback: BrainCallback,
    paused: Box<dyn Fn() -> bool + Send + Sync>,
    unkept: Box<dyn Fn(&str) -> bool + Send + Sync>,
) -> Option<ChatHandle> {
    // Check master switch + legacy providers
    let chat = &config.chat;
    let tg = &config.telegram;
    let wa = &config.whatsapp;

    let any_enabled = chat.enabled
        || (tg.enabled && tg.bot_token.is_some() && tg.chat_id.is_some())
        || (wa.enabled && wa.phone_number_id.is_some() && wa.access_token.is_some());

    if !any_enabled {
        tracing::debug!("Chat: no providers enabled, skipping");
        return None;
    }

    // Open a dedicated SQLite connection for chat transcripts
    let db_path = config.yantrikdb.db_path.clone();
    let chat_db_path = std::path::Path::new(&db_path)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("chat.db");

    let conn = match Connection::open(&chat_db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to open chat database");
            return None;
        }
    };
    let db = Arc::new(Mutex::new(conn));

    // Create router
    let mut router = ChatRouter::new(Arc::clone(&db));

    // Set up event channel
    let (event_tx, event_rx) = crossbeam_channel::unbounded();
    router.set_event_channel(event_tx.clone());

    // Wire callbacks
    router.set_ai_callback(ai_callback);
    router.set_brain_callback(brain_callback);
    // Nothing is kept while the person is private.
    router.set_paused(paused);
    // Nor any answer to an approval card.
    router.set_unkept(unkept);

    // Who the person is: the named people, and the Telegram chat the bot was set up with (a
    // private chat's id is its person's). Nobody else is ever answered.
    // Not IRC: a nick is anyone's to take while its owner is away, over plain text, so no nick is
    // the person (security review, 29 Sep 2026).
    let mut people: Vec<(String, String)> = chat
        .people
        .iter()
        .filter(|p| {
            let irc = p.provider.trim().eq_ignore_ascii_case("irc");
            if irc {
                tracing::warn!(id = %p.id, "Chat: an IRC nick cannot name the person; ignored in chat.people");
            }
            !irc
        })
        .map(|p| (p.provider.trim().to_string(), p.id.trim().to_string()))
        .collect();
    if tg.enabled {
        if let Some(chat_id) = tg.chat_id.as_deref().map(str::trim).filter(|c| !c.is_empty() && !c.starts_with('-')) {
            people.push(("telegram".to_string(), chat_id.to_string()));
        }
    }
    if people.is_empty() {
        tracing::warn!("Chat: no one is named as the person on any channel (chat.people); channels will answer no one");
    }
    router.set_people(people);

    // Create manager
    let inbound_tx = router.inbound_sender();
    let mut manager = ProviderManager::new(inbound_tx, Some(event_tx)).with_channels(router.channels());

    // Instantiate and start enabled providers
    let mut started = 0;

    // Telegram (from legacy config section)
    if tg.enabled {
        if let (Some(token), Some(chat_id)) = (&tg.bot_token, &tg.chat_id) {
            let provider = yantrik_chat::providers::telegram::TelegramProvider::new(
                token.clone(),
                chat_id.clone(),
            );
            manager.start_provider(Box::new(provider));
            started += 1;
        }
    }

    // WhatsApp (from legacy config section)
    if wa.enabled {
        if let (Some(phone_id), Some(token)) = (&wa.phone_number_id, &wa.access_token) {
            let provider = yantrik_chat::providers::whatsapp::WhatsAppProvider::new(
                phone_id.clone(),
                token.clone(),
                wa.recipient.clone(),
                wa.verify_token
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| uuid7::uuid7().to_string()),
            );
            manager.start_provider(Box::new(provider));
            started += 1;
        }
    }

    // Discord
    if chat.discord.enabled {
        if let Some(token) = &chat.discord.bot_token {
            let provider = yantrik_chat::providers::discord::DiscordProvider::new(
                token.clone(),
                chat.discord.allowed_guilds.clone(),
            );
            manager.start_provider(Box::new(provider));
            started += 1;
        }
    }

    // Matrix
    if chat.matrix.enabled {
        if let (Some(hs), Some(token)) = (&chat.matrix.homeserver, &chat.matrix.access_token) {
            let provider = yantrik_chat::providers::matrix::MatrixProvider::new(
                hs.clone(),
                token.clone(),
                chat.matrix.allowed_rooms.clone(),
            );
            manager.start_provider(Box::new(provider));
            started += 1;
        }
    }

    // IRC
    if chat.irc.enabled {
        if let Some(server) = &chat.irc.server {
            let provider = yantrik_chat::providers::irc::IrcProvider::new(
                server.clone(),
                chat.irc.port,
                chat.irc.nick.clone(),
                chat.irc.channels.clone(),
            );
            manager.start_provider(Box::new(provider));
            started += 1;
        }
    }

    // Slack
    if chat.slack.enabled {
        if let (Some(bot_token), Some(app_token)) = (&chat.slack.bot_token, &chat.slack.app_token) {
            let provider = yantrik_chat::providers::slack::SlackProvider::new(
                bot_token.clone(),
                app_token.clone(),
                chat.slack.allowed_channels.clone(),
            );
            manager.start_provider(Box::new(provider));
            started += 1;
        }
    }

    // Signal
    if chat.signal.enabled {
        if let Some(account) = &chat.signal.account {
            let provider = yantrik_chat::providers::signal::SignalProvider::new(
                account.clone(),
                chat.signal.signal_cli_path.clone(),
            );
            manager.start_provider(Box::new(provider));
            started += 1;
        }
    }

    if started == 0 {
        tracing::debug!("Chat: no providers could start (missing credentials?)");
        return None;
    }

    // Start router thread
    // Taken before the router moves to its thread: what the shell sends unasked goes through it.
    let outbox = router.outbox();
    let router_handle = thread::Builder::new()
        .name("chat-router".into())
        .spawn(move || {
            tracing::info!("Chat router thread started");
            loop {
                let processed = router.process_blocking();
                if processed == 0 {
                    break;
                }
            }
            tracing::info!("Chat router thread exiting");
        })
        .expect("Failed to spawn chat router thread");

    // Start event monitor thread
    thread::Builder::new()
        .name("chat-events".into())
        .spawn(move || {
            while let Ok(event) = event_rx.recv() {
                match event {
                    RouterEvent::MessageReceived { provider, sender_name, content_preview, replied, .. } => {
                        tracing::info!(
                            provider = %provider,
                            sender = %sender_name,
                            replied,
                            "Chat: {}",
                            &content_preview[..content_preview.floor_char_boundary(60)],
                        );
                    }
                    RouterEvent::ProviderStatus { provider, health } => {
                        tracing::info!(provider = %provider, health = ?health, "Chat provider status");
                    }
                    RouterEvent::AiReplied { provider, response_preview, .. } => {
                        tracing::debug!(provider = %provider, "Chat AI: {}", response_preview);
                    }
                }
            }
        })
        .expect("Failed to spawn chat events thread");

    tracing::info!(providers = started, "Chat system started");

    Some(ChatHandle {
        _router_thread: Some(router_handle),
        _manager: manager,
        outbox,
    })
}
