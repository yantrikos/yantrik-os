//! IRC provider — raw TCP socket, line-based protocol.
//!
//! Minimal IRC client: NICK, USER, JOIN, PRIVMSG, PING/PONG.
//! ~250 LOC. Validates the minimal provider implementation pattern.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::model::*;
use crate::provider::*;

pub struct IrcProvider {
    server: String,
    port: u16,
    nick: String,
    channels: Vec<String>,
    health: ProviderHealth,
    stream: Option<BufReader<TcpStream>>,
    raw_stream: Option<TcpStream>,
}

impl IrcProvider {
    pub fn new(server: String, port: u16, nick: String, channels: Vec<String>) -> Self {
        Self {
            server,
            port,
            nick,
            channels,
            health: ProviderHealth::Disconnected,
            stream: None,
            raw_stream: None,
        }
    }

    fn send_raw(&mut self, line: &str) -> Result<(), ChatError> {
        if let Some(stream) = &mut self.raw_stream {
            write!(stream, "{}\r\n", line)
                .map_err(|e| ChatError::Network(format!("write: {e}")))?;
            stream.flush()
                .map_err(|e| ChatError::Network(format!("flush: {e}")))?;
            Ok(())
        } else {
            Err(ChatError::Network("not connected".into()))
        }
    }

    fn read_line(&mut self) -> Result<Option<String>, ChatError> {
        if let Some(reader) = &mut self.stream {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => Err(ChatError::Network("connection closed".into())),
                Ok(_) => Ok(Some(line.trim_end().to_string())),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
                Err(e) => Err(ChatError::Network(format!("read: {e}"))),
            }
        } else {
            Err(ChatError::Network("not connected".into()))
        }
    }

    /// Parse an IRC message line.
    /// Format: [:prefix] COMMAND [params] [:trailing]
    fn parse_line(&self, line: &str) -> Option<IrcMessage> {
        let mut rest = line;

        // Parse prefix
        let prefix = if rest.starts_with(':') {
            let space = rest.find(' ')?;
            let p = &rest[1..space];
            rest = &rest[space + 1..];
            Some(p.to_string())
        } else {
            None
        };

        // Parse command
        let space = rest.find(' ').unwrap_or(rest.len());
        let command = rest[..space].to_string();
        rest = if space < rest.len() { &rest[space + 1..] } else { "" };

        // Parse params and trailing
        let (params, trailing) = if let Some(colon_pos) = rest.find(" :") {
            let params: Vec<String> = rest[..colon_pos].split(' ').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
            let trailing = rest[colon_pos + 2..].to_string();
            (params, Some(trailing))
        } else if rest.starts_with(':') {
            (vec![], Some(rest[1..].to_string()))
        } else {
            let params: Vec<String> = rest.split(' ').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
            (params, None)
        };

        Some(IrcMessage { prefix, command, params, trailing })
    }
}

struct IrcMessage {
    prefix: Option<String>,
    command: String,
    params: Vec<String>,
    trailing: Option<String>,
}

impl IrcMessage {
    /// Extract nick from prefix (nick!user@host → nick).
    fn nick(&self) -> Option<&str> {
        self.prefix.as_ref()?.split('!').next()
    }
}

impl ChatProvider for IrcProvider {
    fn id(&self) -> &'static str { "irc" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::Polling,
            media: false,
            voice: false,
            reactions: false,
            typing: false,
            threads: false,
            groups: true,
            channels: true,
            read_receipts: false,
            edits: false,
            deletes: false,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        let addr = format!("{}:{}", self.server, self.port);
        let stream = TcpStream::connect_timeout(
            &addr.parse().map_err(|e| ChatError::Network(format!("bad addr: {e}")))?,
            Duration::from_secs(10),
        ).map_err(|e| ChatError::Network(format!("connect: {e}")))?;

        stream.set_read_timeout(Some(Duration::from_millis(200)))
            .map_err(|e| ChatError::Network(format!("timeout: {e}")))?;

        self.raw_stream = Some(stream.try_clone()
            .map_err(|e| ChatError::Network(format!("clone: {e}")))?);
        self.stream = Some(BufReader::new(stream));

        // Register
        self.send_raw(&format!("NICK {}", self.nick))?;
        self.send_raw(&format!("USER {} 0 * :Yantrik AI", self.nick))?;

        // Wait for welcome (001) or error
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut registered = false;
        while std::time::Instant::now() < deadline {
            if let Some(line) = self.read_line()? {
                if let Some(msg) = self.parse_line(&line) {
                    match msg.command.as_str() {
                        "001" => { registered = true; break; }
                        "PING" => {
                            let token = msg.trailing.as_deref().unwrap_or("");
                            self.send_raw(&format!("PONG :{token}"))?;
                        }
                        "433" => {
                            // Nick already in use — try with underscore
                            self.nick.push('_');
                            self.send_raw(&format!("NICK {}", self.nick))?;
                        }
                        _ => {}
                    }
                }
            }
        }

        if !registered {
            return Err(ChatError::Network("IRC registration timed out".into()));
        }

        // Join channels
        let channels: Vec<String> = self.channels.iter()
            .map(|c| if c.starts_with('#') { c.clone() } else { format!("#{c}") })
            .collect();
        for ch in &channels {
            self.send_raw(&format!("JOIN {ch}"))?;
        }

        tracing::info!(nick = %self.nick, channels = ?self.channels, server = %self.server, "IRC connected");
        self.health = ProviderHealth::Connected;
        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), ChatError> {
        let _ = self.send_raw("QUIT :Yantrik signing off");
        self.stream = None;
        self.raw_stream = None;
        self.health = ProviderHealth::Disconnected;
        Ok(())
    }

    fn health(&self) -> ProviderHealth {
        self.health
    }

    fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
        let mut events = Vec::new();

        // Read up to 50 lines per poll
        for _ in 0..50 {
            let line = match self.read_line() {
                Ok(Some(l)) => l,
                Ok(None) => break,
                Err(e) => return Err(e),
            };

            let msg = match self.parse_line(&line) {
                Some(m) => m,
                None => continue,
            };

            match msg.command.as_str() {
                "PING" => {
                    let token = msg.trailing.as_deref().unwrap_or("");
                    self.send_raw(&format!("PONG :{token}"))?;
                }
                "PRIVMSG" => {
                    let nick = match msg.nick() {
                        Some(n) => n.to_string(),
                        None => continue,
                    };

                    // Skip our own messages
                    if nick == self.nick {
                        continue;
                    }

                    let target = msg.params.first().map(|s| s.as_str()).unwrap_or("");
                    let text = msg.trailing.as_deref().unwrap_or("");
                    if text.is_empty() {
                        continue;
                    }

                    let (kind, conv_id) = if target.starts_with('#') {
                        (ConversationKind::Channel, target.to_string())
                    } else {
                        // DM: conversation is with the sender
                        (ConversationKind::Direct, nick.clone())
                    };

                    // Check if bot was mentioned
                    let mentions_ai = text.contains(&self.nick);

                    let event_id = format!(
                        "irc_{}_{}_{}",
                        conv_id,
                        nick,
                        chrono::Utc::now().timestamp_millis(),
                    );

                    events.push(InboundEvent::Message(InboundMessage {
                        event_id,
                        conversation: ConversationRef {
                            provider: "irc".into(),
                            kind,
                            id: conv_id,
                            parent_id: None,
                            title: None,
                        },
                        message: MessageRef {
                            provider: "irc".into(),
                            id: format!("{}", chrono::Utc::now().timestamp_millis()),
                        },
                        sender: ActorRef {
                            id: nick.clone(),
                            display_name: nick,
                            is_bot: false,
                        },
                        timestamp_ms: chrono::Utc::now().timestamp_millis(),
                        content: MessageContent::Text { text: text.to_string() },
                        reply_to: None,
                        mentions_ai,
                        raw: Some(line.clone()),
                    }));
                }
                _ => {}
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
                // IRC has a ~512 byte line limit. Split long messages.
                let max_len = 400; // Leave room for PRIVMSG prefix
                for line in text.lines() {
                    if line.len() <= max_len {
                        self.send_raw(&format!("PRIVMSG {} :{}", target.id, line))?;
                    } else {
                        // Split at word boundaries
                        let mut remaining = line;
                        while !remaining.is_empty() {
                            let split_at = if remaining.len() <= max_len {
                                remaining.len()
                            } else {
                                remaining[..max_len]
                                    .rfind(' ')
                                    .unwrap_or(max_len)
                            };
                            let (chunk, rest) = remaining.split_at(split_at);
                            self.send_raw(&format!("PRIVMSG {} :{}", target.id, chunk.trim()))?;
                            remaining = rest.trim_start();
                        }
                    }
                }

                Ok(SendReceipt {
                    message: MessageRef {
                        provider: "irc".into(),
                        id: chrono::Utc::now().timestamp_millis().to_string(),
                    },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }
}
