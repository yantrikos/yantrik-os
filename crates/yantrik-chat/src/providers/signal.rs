//! Signal provider — signal-cli subprocess bridge.
//!
//! Uses signal-cli in JSON-RPC mode as a subprocess.
//! signal-cli handles the Signal protocol (libsignal-client);
//! we just parse its JSON output and send commands via stdin.
//!
//! Requires: signal-cli installed and registered with a phone number.
//! Run: `signal-cli -a +1234567890 daemon --json` as the subprocess.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::model::*;
use crate::provider::*;

pub struct SignalProvider {
    /// Phone number registered with signal-cli (e.g., "+1234567890").
    account: String,
    /// Path to signal-cli binary.
    signal_cli_path: String,
    health: ProviderHealth,
    /// Running signal-cli daemon process.
    child: Option<Child>,
    /// Buffered reader for stdout.
    stdout: Option<BufReader<std::process::ChildStdout>>,
    /// Stdin writer for sending commands.
    stdin: Option<std::process::ChildStdin>,
}

impl SignalProvider {
    pub fn new(account: String, signal_cli_path: Option<String>) -> Self {
        Self {
            account,
            signal_cli_path: signal_cli_path.unwrap_or_else(|| "signal-cli".to_string()),
            health: ProviderHealth::Disconnected,
            child: None,
            stdout: None,
            stdin: None,
        }
    }

    /// Send a JSON-RPC command to signal-cli daemon.
    fn send_command(&mut self, method: &str, params: serde_json::Value) -> Result<(), ChatError> {
        let stdin = self.stdin.as_mut()
            .ok_or_else(|| ChatError::Network("not connected".into()))?;

        let rpc = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": chrono::Utc::now().timestamp_millis().to_string(),
        });

        let mut line = rpc.to_string();
        line.push('\n');

        stdin.write_all(line.as_bytes())
            .map_err(|e| ChatError::Network(format!("stdin write: {e}")))?;
        stdin.flush()
            .map_err(|e| ChatError::Network(format!("stdin flush: {e}")))?;

        Ok(())
    }

    /// Read one line from signal-cli stdout (non-blocking via timeout).
    fn read_line(&mut self) -> Result<Option<String>, ChatError> {
        let reader = self.stdout.as_mut()
            .ok_or_else(|| ChatError::Network("not connected".into()))?;

        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => Err(ChatError::Network("signal-cli process exited".into())),
            Ok(_) => {
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
            Err(e) => Err(ChatError::Network(format!("stdout read: {e}"))),
        }
    }
}

impl Drop for SignalProvider {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl ChatProvider for SignalProvider {
    fn id(&self) -> &'static str { "signal" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::Polling,
            media: true,
            voice: true,
            reactions: true,
            typing: true,
            threads: false,
            groups: true,
            channels: false,
            read_receipts: true,
            edits: false,
            deletes: true,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        // Start signal-cli daemon in JSON-RPC mode
        let mut cmd = Command::new(&self.signal_cli_path);
        cmd.args(["-a", &self.account, "daemon", "--json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = cmd.spawn()
            .map_err(|e| ChatError::Network(format!("spawn signal-cli: {e}")))?;

        let stdout = child.stdout.take()
            .ok_or_else(|| ChatError::Internal("no stdout".into()))?;
        let stdin = child.stdin.take()
            .ok_or_else(|| ChatError::Internal("no stdin".into()))?;

        self.stdout = Some(BufReader::new(stdout));
        self.stdin = Some(stdin);
        self.child = Some(child);

        // Wait briefly for daemon to start, read any initial output
        std::thread::sleep(Duration::from_secs(2));

        // Drain any startup messages
        for _ in 0..20 {
            match self.read_line() {
                Ok(Some(_)) => continue,
                _ => break,
            }
        }

        tracing::info!(account = %self.account, "Signal daemon started");
        self.health = ProviderHealth::Connected;
        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), ChatError> {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.stdout = None;
        self.stdin = None;
        self.health = ProviderHealth::Disconnected;
        Ok(())
    }

    fn health(&self) -> ProviderHealth {
        self.health
    }

    fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
        // Check if process is still alive
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(ChatError::Network(format!("signal-cli exited: {status}")));
                }
                Err(e) => {
                    return Err(ChatError::Network(format!("process check: {e}")));
                }
                Ok(None) => {} // Still running
            }
        }

        let mut events = Vec::new();

        for _ in 0..50 {
            let line = match self.read_line() {
                Ok(Some(l)) => l,
                Ok(None) => break,
                Err(e) => return Err(e),
            };

            // Parse JSON line
            let val: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // signal-cli daemon outputs envelope objects
            let envelope = match val.get("envelope") {
                Some(e) => e,
                None => continue,
            };

            if let Some(event) = parse_signal_envelope(envelope) {
                events.push(event);
            }
        }

        Ok(events)
    }

    fn send(
        &mut self,
        target: &ConversationRef,
        msg: &OutboundMessage,
    ) -> Result<SendReceipt, ChatError> {
        match &msg.content {
            OutboundContent::Text(text) => {
                let params = match target.kind {
                    ConversationKind::Group => {
                        serde_json::json!({
                            "message": text,
                            "groupId": target.id,
                        })
                    }
                    _ => {
                        serde_json::json!({
                            "message": text,
                            "recipient": [target.id],
                        })
                    }
                };

                self.send_command("send", params)?;

                Ok(SendReceipt {
                    message: MessageRef {
                        provider: "signal".into(),
                        id: chrono::Utc::now().timestamp_millis().to_string(),
                    },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            OutboundContent::Reaction { target: msg_ref, emoji } => {
                let params = serde_json::json!({
                    "recipient": [target.id],
                    "emoji": emoji,
                    "targetAuthor": msg_ref.provider,
                    "targetTimestamp": msg_ref.id.parse::<i64>().unwrap_or(0),
                });
                self.send_command("sendReaction", params)?;

                Ok(SendReceipt {
                    message: msg_ref.clone(),
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }

    fn send_typing(&mut self, target: &ConversationRef) -> Result<(), ChatError> {
        let params = serde_json::json!({
            "recipient": [target.id],
        });
        self.send_command("sendTyping", params)?;
        Ok(())
    }

    fn mark_read(
        &mut self,
        _target: &ConversationRef,
        up_to: Option<&MessageRef>,
    ) -> Result<(), ChatError> {
        if let Some(msg) = up_to {
            let params = serde_json::json!({
                "recipient": [msg.provider],
                "targetTimestamp": [msg.id.parse::<i64>().unwrap_or(0)],
            });
            self.send_command("sendReceipt", params)?;
        }
        Ok(())
    }
}

fn parse_signal_envelope(envelope: &serde_json::Value) -> Option<InboundEvent> {
    let source = envelope.get("source").and_then(|v| v.as_str())?;
    let timestamp = envelope.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

    // Data message (regular messages)
    let data_msg = envelope.get("dataMessage")?;

    let msg_body = data_msg.get("message").and_then(|v| v.as_str());
    let attachments = data_msg.get("attachments").and_then(|v| v.as_array());
    let group_info = data_msg.get("groupInfo");

    // Determine content
    let content = if let Some(attachments) = attachments {
        if let Some(attachment) = attachments.first() {
            let content_type = attachment.get("contentType")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let id = attachment.get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let filename = attachment.get("filename")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let size = attachment.get("size")
                .and_then(|v| v.as_u64());

            let media = MediaRef {
                provider: "signal".into(),
                id,
                mime_type: Some(content_type.to_string()),
                size_bytes: size,
                filename,
            };

            if content_type.starts_with("image/") {
                MessageContent::Image {
                    media,
                    caption: msg_body.map(|s| s.to_string()),
                }
            } else if content_type.starts_with("audio/") {
                MessageContent::Voice {
                    media,
                    duration_ms: None,
                }
            } else {
                MessageContent::File {
                    media,
                    caption: msg_body.map(|s| s.to_string()),
                }
            }
        } else if let Some(text) = msg_body {
            MessageContent::Text { text: text.to_string() }
        } else {
            return None;
        }
    } else if let Some(text) = msg_body {
        MessageContent::Text { text: text.to_string() }
    } else {
        return None;
    };

    // Conversation: group or DM
    let (kind, conv_id) = if let Some(group) = group_info {
        let gid = group.get("groupId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        (ConversationKind::Group, gid)
    } else {
        (ConversationKind::Direct, source.to_string())
    };

    // Quote (reply-to)
    let reply_to = data_msg.get("quote")
        .and_then(|q| {
            let author = q.get("author").and_then(|v| v.as_str())?;
            let ts = q.get("id").and_then(|v| v.as_i64())?;
            Some(MessageRef {
                provider: author.to_string(),
                id: ts.to_string(),
            })
        });

    let event_id = format!("signal_{}_{}", source, timestamp);

    // Display name: source name if available, else phone number
    let source_name = envelope.get("sourceName")
        .and_then(|v| v.as_str())
        .unwrap_or(source)
        .to_string();

    Some(InboundEvent::Message(InboundMessage {
        event_id,
        conversation: ConversationRef {
            provider: "signal".into(),
            kind,
            id: conv_id,
            parent_id: None,
            title: None,
        },
        message: MessageRef {
            provider: source.to_string(),
            id: timestamp.to_string(),
        },
        sender: ActorRef {
            id: source.to_string(),
            display_name: source_name,
            is_bot: false,
        },
        timestamp_ms: timestamp,
        content,
        reply_to,
        mentions_ai: true, // Signal DMs are directed at the bot; groups need mention detection
        raw: Some(envelope.to_string()),
    }))
}
