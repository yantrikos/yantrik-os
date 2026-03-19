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
use crate::provider::ChatProvider;
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

/// Callback for AI processing. The router calls this when a message needs a response.
/// Receives: (message_text, conversation_context, policy) → AI response text.
pub type AiCallback = Box<dyn Fn(&str, &[String], &ConversationPolicy) -> Option<String> + Send + Sync>;

/// Callback for brain integration. Called for every non-muted message.
/// Receives: (sender_name, sender_id, provider, content_type).
pub type BrainCallback = Box<dyn Fn(&str, &str, &str, &str) + Send + Sync>;

/// Central message router. One per Yantrik instance.
pub struct ChatRouter {
    /// Channel receiving events from all provider threads.
    inbound_rx: Receiver<(String, InboundEvent)>,
    /// Sender side — cloned and given to each provider thread.
    inbound_tx: Sender<(String, InboundEvent)>,
    /// Providers indexed by id, behind mutex for thread-safe access.
    providers: Arc<Mutex<HashMap<String, Box<dyn ChatProvider>>>>,
    /// Shared database connection.
    db: Arc<Mutex<Connection>>,
    /// External event channel (for UI updates, brain feeding).
    event_tx: Option<Sender<RouterEvent>>,
    /// AI processing callback.
    ai_callback: Option<AiCallback>,
    /// Brain integration callback.
    brain_callback: Option<BrainCallback>,
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
            providers: Arc::new(Mutex::new(HashMap::new())),
            db,
            event_tx: None,
            ai_callback: None,
            brain_callback: None,
        }
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

    /// Register a provider. Does NOT start it — call manager for that.
    pub fn register_provider(&self, provider: Box<dyn ChatProvider>) {
        let id = provider.id().to_string();
        if let Ok(mut providers) = self.providers.lock() {
            providers.insert(id, provider);
        }
    }

    /// Get a reference to the providers map (for manager to start threads).
    pub fn providers(&self) -> Arc<Mutex<HashMap<String, Box<dyn ChatProvider>>>> {
        Arc::clone(&self.providers)
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

        // Resolve conversation + policy
        let (conv_rowid, policy) = match store::get_or_create_conversation(&db, &msg.conversation) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(provider = provider_id, error = %e, "Failed to resolve conversation");
                return;
            }
        };

        // Store message in transcript
        store::store_message(&db, conv_rowid, &msg);

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
        let should_reply = policy::should_ai_reply(&msg, &policy, current_hour);

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

                // Get AI response
                if let Some(response) = ai_cb(&content_text, &context, &policy) {
                    // Send response through provider
                    let out_msg = OutboundMessage::text(&response)
                        .with_reply(msg.message.clone());

                    if let Ok(mut providers) = self.providers.lock() {
                        if let Some(provider) = providers.get_mut(provider_id) {
                            // Show typing first
                            let _ = provider.send_typing(&msg.conversation);

                            match provider.send(&msg.conversation, &out_msg) {
                                Ok(receipt) => {
                                    // Store AI response in transcript
                                    if let Ok(db) = self.db.lock() {
                                        store::store_ai_response(
                                            &db,
                                            conv_rowid,
                                            &receipt.message.id,
                                            &response,
                                            receipt.timestamp_ms,
                                        );
                                    }

                                    // Emit event
                                    if let Some(tx) = &self.event_tx {
                                        let _ = tx.send(RouterEvent::AiReplied {
                                            provider: provider_id.to_string(),
                                            conversation_id: msg.conversation.id.clone(),
                                            response_preview: truncate(&response, 100),
                                        });
                                    }

                                    tracing::info!(
                                        provider = provider_id,
                                        conversation = %msg.conversation.id,
                                        sender = %msg.sender.display_name,
                                        "Chat: AI replied"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        provider = provider_id,
                                        error = %e,
                                        "Failed to send AI response"
                                    );
                                }
                            }
                        }
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
