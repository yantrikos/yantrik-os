//! ChatRouter — central event dispatch between providers and the companion.
//!
//! Receives InboundEvents from provider threads via crossbeam channel,
//! deduplicates, resolves conversations, applies policy, and invokes the
//! AI via a callback. Sends responses back through the correct provider.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use rusqlite::Connection;

use crate::model::*;
use crate::policy::{self, ConversationPolicy};
use crate::provider::{ChatProvider, Outbound, ProviderCapabilities};
use crate::store;

/// Events emitted by the router for external consumption (UI, brain).
#[derive(Debug, Clone)]
pub enum RouterEvent {
    /// A message was received and processed.
    MessageReceived {
        provider: String,
        conversation_id: String,
        sender_name: String,
        content_preview: String,
        replied: bool,
    },
    /// Provider health changed.
    ProviderStatus {
        provider: String,
        health: ProviderHealth,
    },
    /// AI responded to a message.
    AiReplied {
        provider: String,
        conversation_id: String,
        response_preview: String,
    },
}

/// Who asked, and on what channel: what a turn's origin is made from, and where an answer goes.
#[derive(Debug, Clone)]
pub struct Asker {
    /// The provider's id: `telegram`, `signal`, `slack`, …
    pub provider: String,
    pub sender_name: String,
    pub sender_id: String,
    /// What an answer may carry there: `text`, `voice`, `photo`.
    pub carries: Vec<String>,
    /// The conversation it was asked in: where a later answer is sent ([`Outbox::send`]).
    pub conversation: ConversationRef,
}

/// A running channel, as the router reaches it: what it can carry, and how to send to it from any
/// thread. Filled by the [`crate::manager::ProviderManager`] as it starts each provider — the
/// provider itself lives on its polling thread, where nothing else can reach it.
#[derive(Clone)]
pub struct Channel {
    pub capabilities: ProviderCapabilities,
    pub outbound: Option<Arc<dyn Outbound>>,
}

/// The running channels by provider id, shared by the manager that starts them and the router.
pub type Channels = Arc<Mutex<HashMap<String, Channel>>>;

/// Sends to a conversation on a channel at any time, not only as the reply the AI callback
/// returns: an answer that takes a while, or a question for the person (an approval card on
/// their phone). What it sends is kept in the transcript as the AI's, unless paused or asked not
/// to be ([`Outbox::send_unkept`]).
#[derive(Clone)]
pub struct Outbox {
    channels: Channels,
    db: Arc<Mutex<Connection>>,
    paused: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Outbox {
    /// Send `text` to `conversation` on `provider`, and keep it in the transcript.
    pub fn send(&self, provider: &str, conversation: &ConversationRef, text: &str) -> Result<(), String> {
        self.deliver(provider, conversation, text, true)
    }

    /// Send `text`, and keep nothing of it: a message that must not become context for the next
    /// turn (an approval card's code).
    pub fn send_unkept(&self, provider: &str, conversation: &ConversationRef, text: &str) -> Result<(), String> {
        self.deliver(provider, conversation, text, false)
    }

    fn deliver(&self, provider: &str, conversation: &ConversationRef, text: &str, keep: bool) -> Result<(), String> {
        let outbound = {
            let channels = self.channels.lock().map_err(|_| "the channels are shut".to_string())?;
            let channel = channels.get(provider).ok_or_else(|| format!("no channel `{provider}` is running"))?;
            channel.outbound.clone().ok_or_else(|| format!("the `{provider}` channel cannot send yet"))?
        };
        let receipt = outbound.send(conversation, &OutboundMessage::text(text)).map_err(|e| e.to_string())?;
        if keep && !(self.paused)() {
            if let Ok(db) = self.db.lock() {
                if let Ok((rowid, _)) = store::get_or_create_conversation(&db, conversation) {
                    store::store_ai_response(&db, rowid, &receipt.message.id, text, receipt.timestamp_ms);
                }
            }
        }
        Ok(())
    }
}

/// Callback for AI processing. The router calls this when a message needs a response.
/// Receives: (message_text, conversation_context, policy, asker, outbox) → the reply now, or
/// `None` to answer later through the outbox (or not at all).
pub type AiCallback =
    Box<dyn Fn(&str, &[String], &ConversationPolicy, &Asker, &Outbox) -> Option<String> + Send + Sync>;

/// Callback for brain integration. Called for every non-muted message.
/// Receives: (sender_name, sender_id, provider, content_type).
pub type BrainCallback = Box<dyn Fn(&str, &str, &str, &str) + Send + Sync>;

/// Central message router. One per Yantrik instance.
pub struct ChatRouter {
    /// Channel receiving events from all provider threads.
    inbound_rx: Receiver<(String, InboundEvent)>,
    /// Sender side — cloned and given to each provider thread.
    inbound_tx: Sender<(String, InboundEvent)>,
    /// The running channels, as the manager started them.
    channels: Channels,
    /// Shared database connection.
    db: Arc<Mutex<Connection>>,
    /// External event channel (for UI updates, brain feeding).
    event_tx: Option<Sender<RouterEvent>>,
    /// AI processing callback.
    ai_callback: Option<AiCallback>,
    /// Brain integration callback.
    brain_callback: Option<BrainCallback>,
    /// Who the person is on each channel: `(provider id, sender id)`. See [`ChatRouter::set_people`].
    people: std::collections::HashSet<(String, String)>,
    /// Whether nothing is to be kept right now: the person's Private mode. See [`ChatRouter::set_paused`].
    paused: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Messages never kept, whatever else is: see [`ChatRouter::set_unkept`].
    unkept: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

impl ChatRouter {
    /// Create a new router with a shared database.
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        let (inbound_tx, inbound_rx) = crossbeam_channel::unbounded();

        // Ensure store tables exist
        if let Ok(conn) = db.lock() {
            let _ = store::ensure_tables(&conn);
        }

        Self {
            inbound_rx,
            inbound_tx,
            channels: Arc::new(Mutex::new(HashMap::new())),
            db,
            event_tx: None,
            ai_callback: None,
            brain_callback: None,
            people: std::collections::HashSet::new(),
            paused: Arc::new(|| false),
            unkept: Arc::new(|_| false),
        }
    }

    /// Never keep a message `unkept` says yes to: an answer to an approval card, whose code must
    /// not become context for a later turn.
    pub fn set_unkept(&mut self, unkept: Box<dyn Fn(&str) -> bool + Send + Sync>) {
        self.unkept = Arc::from(unkept);
    }

    /// What sends to a conversation at any time: see [`Outbox`].
    pub fn outbox(&self) -> Outbox {
        Outbox { channels: Arc::clone(&self.channels), db: Arc::clone(&self.db), paused: Arc::clone(&self.paused) }
    }

    /// Ask `paused` before keeping anything: while it says yes (the person's Private mode), no
    /// message and no answer is written to the transcript.
    pub fn set_paused(&mut self, paused: Box<dyn Fn() -> bool + Send + Sync>) {
        self.paused = Arc::from(paused);
    }

    fn keeping(&self) -> bool {
        !(self.paused)()
    }

    /// Who the person is on each channel. The AI is asked only about a direct message from one
    /// of them: a stranger who finds the bot — on Signal, any number that writes to it — and
    /// every group, where an answer would be read by others, get nothing. With no one named, no
    /// message is ever answered.
    pub fn set_people(&mut self, people: impl IntoIterator<Item = (String, String)>) {
        self.people = people.into_iter().collect();
    }

    /// Get a sender for provider threads to push events into.
    pub fn inbound_sender(&self) -> Sender<(String, InboundEvent)> {
        self.inbound_tx.clone()
    }

    /// Set the external event channel (for UI/brain).
    pub fn set_event_channel(&mut self, tx: Sender<RouterEvent>) {
        self.event_tx = Some(tx);
    }

    /// Set the AI processing callback.
    pub fn set_ai_callback(&mut self, cb: AiCallback) {
        self.ai_callback = Some(cb);
    }

    /// Set the brain integration callback (called for every non-muted message).
    pub fn set_brain_callback(&mut self, cb: BrainCallback) {
        self.brain_callback = Some(cb);
    }

    /// The running channels, for the manager to fill as it starts each provider.
    pub fn channels(&self) -> Channels {
        Arc::clone(&self.channels)
    }

    /// Process one batch of pending inbound events. Non-blocking.
    /// Returns the number of events processed.
    pub fn process_pending(&self) -> usize {
        let mut count = 0;
        loop {
            match self.inbound_rx.try_recv() {
                Ok((provider_id, event)) => {
                    self.handle_event(&provider_id, event);
                    count += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        count
    }

    /// Process events blocking until one arrives. Returns number processed.
    /// Use this in a dedicated router thread.
    pub fn process_blocking(&self) -> usize {
        match self.inbound_rx.recv() {
            Ok((provider_id, event)) => {
                self.handle_event(&provider_id, event);
                // Drain any additional pending events
                1 + self.process_pending()
            }
            Err(_) => 0,
        }
    }

    fn handle_event(&self, provider_id: &str, event: InboundEvent) {
        // Only handle messages for now (edits, reactions, typing are future)
        let msg = match event {
            InboundEvent::Message(msg) => msg,
            _ => return,
        };

        // Skip bot messages to prevent loops
        if msg.sender.is_bot {
            return;
        }

        let db = match self.db.lock() {
            Ok(db) => db,
            Err(_) => return,
        };

        // Dedupe
        if store::is_event_seen(&db, provider_id, &msg.event_id) {
            return;
        }
        store::mark_event_seen(&db, provider_id, &msg.event_id);

        // Only the person's words are kept, and nothing at all while the person is private: a
        // stranger who writes to the bot gets no reply and no row, and cannot grow the store.
        let from_person = self.people.contains(&(provider_id.to_string(), msg.sender.id.clone()));
        if !from_person {
            tracing::debug!(provider = provider_id, "Chat: not the person; not answered, not kept");
            return;
        }
        let keeping = self.keeping();

        // Resolve conversation + policy
        let (conv_rowid, policy) = match store::get_or_create_conversation(&db, &msg.conversation) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(provider = provider_id, error = %e, "Failed to resolve conversation");
                return;
            }
        };

        // Store message in transcript
        if keeping && !(self.unkept)(msg.content.text().unwrap_or("")) {
            store::store_message(&db, conv_rowid, &msg);
        }

        // Feed brain (for all non-muted conversations)
        if policy::should_feed_brain(&policy) {
            if let Some(brain_cb) = &self.brain_callback {
                brain_cb(
                    &msg.sender.display_name,
                    &msg.sender.id,
                    provider_id,
                    msg.content.kind_str(),
                );
            }
        }

        let content_text = msg.content.text().unwrap_or("").to_string();

        // Check if AI should reply
        let current_hour = chrono::Local::now().hour() as u8;
        let direct = msg.conversation.kind == crate::model::ConversationKind::Direct;
        if !direct {
            tracing::debug!(provider = provider_id, "Chat: the person, but not in a direct message; nothing is asked of the AI");
        }
        let should_reply = direct && policy::should_ai_reply(&msg, &policy, current_hour);

        if should_reply {
            if let Some(ai_cb) = &self.ai_callback {
                // Build context from recent messages
                let context: Vec<String> = store::recent_messages(&db, conv_rowid, 10)
                    .into_iter()
                    .map(|(name, text, is_ai, _ts)| {
                        if is_ai {
                            format!("Yantrik: {text}")
                        } else {
                            format!("{name}: {text}")
                        }
                    })
                    .collect();

                // Drop db lock before calling AI (may take a while)
                drop(db);

                // Who asked, and what the channel can carry back.
                let mut carries = vec!["text".to_string()];
                if let Ok(channels) = self.channels.lock() {
                    if let Some(caps) = channels.get(provider_id).map(|c| &c.capabilities) {
                        if caps.voice {
                            carries.push("voice".into());
                        }
                        if caps.media {
                            carries.push("photo".into());
                        }
                    }
                }
                let asker = Asker {
                    provider: provider_id.to_string(),
                    sender_name: msg.sender.display_name.clone(),
                    sender_id: msg.sender.id.clone(),
                    carries,
                    conversation: msg.conversation.clone(),
                };

                // Get AI response
                if let Some(response) = ai_cb(&content_text, &context, &policy, &asker, &self.outbox()) {
                    // Sent the way everything to a channel is: through its outbound sender.
                    match self.outbox().send(provider_id, &msg.conversation, &response) {
                        Ok(()) => {
                            if let Some(tx) = &self.event_tx {
                                let _ = tx.send(RouterEvent::AiReplied {
                                    provider: provider_id.to_string(),
                                    conversation_id: msg.conversation.id.clone(),
                                    response_preview: truncate(&response, 100),
                                });
                            }
                            tracing::info!(provider = provider_id, conversation = %msg.conversation.id, "Chat: AI replied");
                        }
                        Err(e) => tracing::error!(provider = provider_id, error = %e, "Failed to send AI response"),
                    }

                    // Emit message received event (with reply)
                    if let Some(tx) = &self.event_tx {
                        let _ = tx.send(RouterEvent::MessageReceived {
                            provider: provider_id.to_string(),
                            conversation_id: msg.conversation.id.clone(),
                            sender_name: msg.sender.display_name.clone(),
                            content_preview: truncate(&content_text, 100),
                            replied: true,
                        });
                    }
                    return;
                }
            }
        }

        // Emit message received event (no reply)
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(RouterEvent::MessageReceived {
                provider: provider_id.to_string(),
                conversation_id: msg.conversation.id.clone(),
                sender_name: msg.sender.display_name.clone(),
                content_preview: truncate(&content_text, 100),
                replied: false,
            });
        }

        tracing::debug!(
            provider = provider_id,
            conversation = %msg.conversation.id,
            sender = %msg.sender.display_name,
            mode = ?policy.mode,
            "Chat: message received (no reply)"
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = s.floor_char_boundary(max.saturating_sub(3));
        format!("{}...", &s[..boundary])
    }
}

// Needed for chrono::Local::now().hour()
use chrono::Timelike;

#[cfg(test)]
mod people_tests {
    use super::*;
    use crate::model::{ActorRef, ConversationKind, ConversationRef, InboundMessage, MessageContent, MessageRef};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn message(provider: &str, sender: &str, kind: ConversationKind, n: u32) -> InboundEvent {
        let mut conversation = ConversationRef::direct(provider, sender);
        conversation.kind = kind;
        InboundEvent::Message(InboundMessage {
            event_id: format!("e{n}"),
            conversation,
            message: MessageRef { provider: provider.into(), id: format!("m{n}") },
            sender: ActorRef { id: sender.into(), display_name: sender.into(), is_bot: false },
            timestamp_ms: 0,
            content: MessageContent::Text { text: "what is on my screen?".into() },
            reply_to: None,
            mentions_ai: true,
            raw: None,
        })
    }

    fn router(asked: Arc<AtomicUsize>) -> ChatRouter {
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut router = ChatRouter::new(db);
        router.set_ai_callback(Box::new(move |_, _, _, _, _| {
            asked.fetch_add(1, Ordering::SeqCst);
            None
        }));
        router
    }

    #[test]
    fn only_the_person_in_a_direct_message_is_answered() {
        let asked = Arc::new(AtomicUsize::new(0));
        let mut r = router(asked.clone());
        r.set_people([("signal".to_string(), "+15550001".to_string())]);
        let tx = r.inbound_sender();
        tx.send(("signal".into(), message("signal", "+15559999", ConversationKind::Direct, 1))).unwrap();
        tx.send(("signal".into(), message("signal", "+15550001", ConversationKind::Group, 2))).unwrap();
        tx.send(("telegram".into(), message("telegram", "+15550001", ConversationKind::Direct, 3))).unwrap();
        r.process_pending();
        assert_eq!(asked.load(Ordering::SeqCst), 0, "a stranger, a group, and the same id on another channel");
        tx.send(("signal".into(), message("signal", "+15550001", ConversationKind::Direct, 4))).unwrap();
        r.process_pending();
        assert_eq!(asked.load(Ordering::SeqCst), 1, "the person, directly");
    }

    #[test]
    fn with_no_one_named_no_one_is_answered() {
        let asked = Arc::new(AtomicUsize::new(0));
        let r = router(asked.clone());
        r.inbound_sender().send(("signal".into(), message("signal", "+15550001", ConversationKind::Direct, 1))).unwrap();
        r.process_pending();
        assert_eq!(asked.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use crate::manager::ProviderManager;
    use crate::model::{ActorRef, ConversationRef, InboundMessage, MessageContent, MessageRef, ProviderHealth, SendReceipt};
    use crate::provider::{ChatError, ChatProvider, Outbound, ProviderCapabilities};

    /// Says one thing, once, then waits; what is sent to it lands in `sent`.
    struct Scripted {
        said: bool,
        sent: Arc<Mutex<Vec<String>>>,
    }

    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl Outbound for Recorder {
        fn send(&self, target: &ConversationRef, msg: &OutboundMessage) -> Result<SendReceipt, ChatError> {
            if let crate::model::OutboundContent::Text(t) = &msg.content {
                self.0.lock().unwrap().push(format!("{}: {t}", target.id));
            }
            Ok(SendReceipt { message: MessageRef { provider: "fake".into(), id: "out-1".into() }, timestamp_ms: 0 })
        }
    }

    impl ChatProvider for Scripted {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::minimal()
        }
        fn outbound(&self) -> Option<Arc<dyn Outbound>> {
            Some(Arc::new(Recorder(self.sent.clone())))
        }
        fn connect(&mut self) -> Result<(), ChatError> {
            Ok(())
        }
        fn disconnect(&mut self) -> Result<(), ChatError> {
            Ok(())
        }
        fn health(&self) -> ProviderHealth {
            ProviderHealth::Connected
        }
        fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
            if self.said {
                std::thread::sleep(std::time::Duration::from_millis(50));
                return Ok(Vec::new());
            }
            self.said = true;
            Ok(vec![InboundEvent::Message(InboundMessage {
                event_id: "e1".into(),
                conversation: ConversationRef::direct("fake", "person-1"),
                message: MessageRef { provider: "fake".into(), id: "m1".into() },
                sender: ActorRef { id: "person-1".into(), display_name: "Pranab".into(), is_bot: false },
                timestamp_ms: 0,
                content: MessageContent::Text { text: "hello".into() },
                reply_to: None,
                mentions_ai: true,
                raw: None,
            })])
        }
        fn send(&mut self, _: &ConversationRef, _: &OutboundMessage) -> Result<SendReceipt, ChatError> {
            Err(ChatError::Unsupported)
        }
    }

    #[test]
    fn a_message_from_the_person_is_answered_back_through_its_channel() {
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut router = ChatRouter::new(db);
        router.set_people([("fake".to_string(), "person-1".to_string())]);
        router.set_ai_callback(Box::new(|text, _, _, _, _| Some(format!("you said {text}"))));
        let later = router.outbox();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut manager = ProviderManager::new(router.inbound_sender(), None).with_channels(router.channels());
        manager.start_provider(Box::new(Scripted { said: false, sent: sent.clone() }));
        for _ in 0..100 {
            router.process_pending();
            if !sent.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(sent.lock().unwrap().as_slice(), ["person-1: you said hello"], "the reply went out through the channel");
        later.send("fake", &ConversationRef::direct("fake", "person-1"), "and later").unwrap();
        assert_eq!(sent.lock().unwrap().last().unwrap(), "person-1: and later", "so does what is sent later");
        assert!(later.send("nowhere", &ConversationRef::direct("nowhere", "x"), "x").is_err());
    }
}
