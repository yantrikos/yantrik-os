//! Native mobile WebSocket provider — server that accepts connections from
//! the Yantrik mobile app for real-time text + voice conversations.
//!
//! Unlike other providers (which are WS/HTTP clients), this is a server:
//! it binds a TCP port, accepts WebSocket upgrades, and manages sessions.
//!
//! Voice flow (V1, half-duplex):
//!   Client sends voice_start + N voice_chunk + voice_end
//!   → server accumulates opus chunks → decodes to PCM → STT (Whisper)
//!   → transcribed text pushed as InboundEvent
//!   → AI responds → text + TTS audio sent back

use std::collections::HashMap;
use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use tungstenite::{accept, Message as WsMessage, WebSocket};

use crate::model::*;
use crate::provider::*;

use super::native_protocol::{ClientMessage, ServerMessage};

/// Handle to a connected mobile client session.
struct SessionHandle {
    /// Write-half of the WebSocket (read-half owned by session thread).
    ws_writer: Arc<Mutex<WebSocket<TcpStream>>>,
    client_id: String,
    client_name: String,
    session_id: String,
}

/// Accumulates opus chunks for a single voice turn.
struct VoiceAccumulator {
    id: String,
    chunks: Vec<Vec<u8>>,
    sample_rate: u32,
}

pub struct NativeProvider {
    port: u16,
    auth_tokens: Vec<String>,
    max_clients: usize,
    health: ProviderHealth,
    /// Active sessions indexed by session_id.
    sessions: Arc<Mutex<HashMap<String, SessionHandle>>>,
    /// Inbound events from session threads → drained by poll().
    inbound_rx: Option<Receiver<InboundEvent>>,
    inbound_tx: Option<Sender<InboundEvent>>,
    /// Listener thread handle.
    listener_thread: Option<thread::JoinHandle<()>>,
    /// Signal to stop the acceptor loop.
    shutdown_tx: Option<Sender<()>>,
}

impl NativeProvider {
    pub fn new(port: u16, auth_tokens: Vec<String>, max_clients: usize) -> Self {
        Self {
            port,
            auth_tokens,
            max_clients: max_clients.max(1),
            health: ProviderHealth::Disconnected,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            inbound_rx: None,
            inbound_tx: None,
            listener_thread: None,
            shutdown_tx: None,
        }
    }
}

impl ChatProvider for NativeProvider {
    fn id(&self) -> &'static str { "native" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::WebSocket,
            media: true,
            voice: true,
            reactions: false,
            typing: true,
            threads: false,
            groups: false,
            channels: false,
            read_receipts: false,
            edits: false,
            deletes: false,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .map_err(|e| ChatError::Network(format!("bind {addr}: {e}")))?;

        // Non-blocking accept with 1s timeout so we can check shutdown signal
        listener.set_nonblocking(true)
            .map_err(|e| ChatError::Network(format!("set_nonblocking: {e}")))?;

        let (inbound_tx, inbound_rx) = crossbeam_channel::unbounded();
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);

        self.inbound_tx = Some(inbound_tx.clone());
        self.inbound_rx = Some(inbound_rx);
        self.shutdown_tx = Some(shutdown_tx);

        let sessions = Arc::clone(&self.sessions);
        let auth_tokens = self.auth_tokens.clone();
        let max_clients = self.max_clients;

        let handle = thread::Builder::new()
            .name("native-acceptor".into())
            .spawn(move || {
                acceptor_loop(listener, sessions, inbound_tx, shutdown_rx, auth_tokens, max_clients);
            })
            .map_err(|e| ChatError::Internal(format!("spawn acceptor: {e}")))?;

        self.listener_thread = Some(handle);
        self.health = ProviderHealth::Connected;

        tracing::info!(port = self.port, "Native WS listening on :{}", self.port);
        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), ChatError> {
        // Signal acceptor to stop
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Close all sessions
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, session) in sessions.drain() {
                if let Ok(mut ws) = session.ws_writer.lock() {
                    let _ = ws.close(None);
                }
            }
        }

        self.listener_thread = None;
        self.health = ProviderHealth::Disconnected;
        Ok(())
    }

    fn health(&self) -> ProviderHealth {
        self.health
    }

    fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
        let rx = self.inbound_rx.as_ref()
            .ok_or_else(|| ChatError::Network("not connected".into()))?;

        let mut events = Vec::new();

        // Block for up to 1 second waiting for the first event
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => events.push(event),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => return Ok(events),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return Err(ChatError::Network("channel closed".into()));
            }
        }

        // Drain any additional pending events
        for _ in 0..50 {
            match rx.try_recv() {
                Ok(event) => events.push(event),
                Err(_) => break,
            }
        }

        Ok(events)
    }

    fn send(
        &mut self,
        target: &ConversationRef,
        msg: &OutboundMessage,
    ) -> Result<SendReceipt, ChatError> {
        let session_id = &target.id;

        let sessions = self.sessions.lock()
            .map_err(|_| ChatError::Internal("session lock poisoned".into()))?;

        let session = sessions.get(session_id)
            .ok_or_else(|| ChatError::Provider(format!("session {session_id} not found")))?;

        match &msg.content {
            OutboundContent::Text(text) => {
                let msg_id = format!("native_{}", chrono::Utc::now().timestamp_millis());
                let server_msg = ServerMessage::Text {
                    id: msg_id.clone(),
                    text: text.clone(),
                    is_final: true,
                };

                send_to_session(&session.ws_writer, &server_msg)?;

                Ok(SendReceipt {
                    message: MessageRef {
                        provider: "native".into(),
                        id: msg_id,
                    },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }

    fn send_typing(&mut self, target: &ConversationRef) -> Result<(), ChatError> {
        let sessions = self.sessions.lock()
            .map_err(|_| ChatError::Internal("lock".into()))?;
        if let Some(session) = sessions.get(&target.id) {
            send_to_session(&session.ws_writer, &ServerMessage::Typing)?;
        }
        Ok(())
    }
}

// ── Acceptor loop ───────────────────────────────────────────────────────────

fn acceptor_loop(
    listener: TcpListener,
    sessions: Arc<Mutex<HashMap<String, SessionHandle>>>,
    inbound_tx: Sender<InboundEvent>,
    shutdown_rx: Receiver<()>,
    auth_tokens: Vec<String>,
    max_clients: usize,
) {
    loop {
        // Check shutdown
        if shutdown_rx.try_recv().is_ok() {
            tracing::info!("Native acceptor shutting down");
            return;
        }

        // Try to accept a connection (non-blocking)
        match listener.accept() {
            Ok((stream, addr)) => {
                // Check max clients
                let count = sessions.lock().map(|s| s.len()).unwrap_or(0);
                if count >= max_clients {
                    tracing::warn!(addr = %addr, max = max_clients, "Native: max clients reached, rejecting");
                    drop(stream);
                    continue;
                }

                tracing::info!(addr = %addr, "Native: new connection");

                // Set blocking mode for the accepted connection
                if let Err(e) = stream.set_nonblocking(false) {
                    tracing::error!(error = %e, "Failed to set blocking mode");
                    continue;
                }

                // Set read timeout so session thread can check for shutdown
                let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));

                let sessions_clone = Arc::clone(&sessions);
                let tx = inbound_tx.clone();
                let tokens = auth_tokens.clone();

                thread::Builder::new()
                    .name(format!("native-session-{}", addr))
                    .spawn(move || {
                        session_loop(stream, sessions_clone, tx, tokens);
                    })
                    .ok();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No pending connection — sleep briefly
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                tracing::error!(error = %e, "Native accept error");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

// ── Session loop ────────────────────────────────────────────────────────────

fn session_loop(
    stream: TcpStream,
    sessions: Arc<Mutex<HashMap<String, SessionHandle>>>,
    inbound_tx: Sender<InboundEvent>,
    auth_tokens: Vec<String>,
) {
    // Accept WebSocket handshake — single WS wrapped in Arc<Mutex<>> for
    // both reading (session thread) and writing (send_to_session).
    let ws = match accept(stream) {
        Ok(ws) => Arc::new(Mutex::new(ws)),
        Err(e) => {
            tracing::error!(error = %e, "Native WS handshake failed");
            return;
        }
    };
    let ws_write = Arc::clone(&ws);

    // Wait for auth message (first message must be auth)
    let (session_id, client_id, client_name) = match wait_for_auth(&ws, &ws_write, &auth_tokens) {
        Some(info) => info,
        None => {
            tracing::warn!("Native: client failed auth, disconnecting");
            return;
        }
    };

    // Register session
    let session_id_clone = session_id.clone();
    if let Ok(mut map) = sessions.lock() {
        map.insert(session_id.clone(), SessionHandle {
            ws_writer: Arc::clone(&ws_write),
            client_id: client_id.clone(),
            client_name: client_name.clone(),
            session_id: session_id.clone(),
        });
    }

    tracing::info!(
        session = %session_id,
        client_id = %client_id,
        client_name = %client_name,
        "Native: session authenticated"
    );

    // Main read loop
    let mut voice_state: Option<VoiceAccumulator> = None;

    loop {
        // Lock WS briefly to read one frame, then release
        let read_result = {
            let mut ws_guard = match ws.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            ws_guard.read()
        };

        let msg = match read_result {
            Ok(WsMessage::Text(text)) => text,
            Ok(WsMessage::Ping(data)) => {
                if let Ok(mut g) = ws.lock() { let _ = g.send(WsMessage::Pong(data)); }
                continue;
            }
            Ok(WsMessage::Close(_)) => {
                tracing::info!(session = %session_id, "Native: client disconnected");
                break;
            }
            Ok(_) => continue,
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Read timeout — send a ping to check liveness
                let alive = ws.lock().map(|mut g| g.send(WsMessage::Ping(vec![])).is_ok()).unwrap_or(false);
                if !alive { break; }
                continue;
            }
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "Native: WS read error");
                break;
            }
        };

        // Parse client message
        let client_msg: ClientMessage = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(error = %e, raw = %msg, "Native: invalid client message");
                continue;
            }
        };

        match client_msg {
            ClientMessage::Text { id, text } => {
                if text.is_empty() {
                    continue;
                }

                // Send "thinking" status
                let _ = send_to_session(&ws_write, &ServerMessage::Thinking);

                // Push as inbound event
                let event = InboundEvent::Message(InboundMessage {
                    event_id: id.clone(),
                    conversation: ConversationRef {
                        provider: "native".into(),
                        kind: ConversationKind::Direct,
                        id: session_id.clone(),
                        parent_id: None,
                        title: None,
                    },
                    message: MessageRef {
                        provider: "native".into(),
                        id: id.clone(),
                    },
                    sender: ActorRef {
                        id: client_id.clone(),
                        display_name: client_name.clone(),
                        is_bot: false,
                    },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                    content: MessageContent::Text { text },
                    reply_to: None,
                    mentions_ai: true,
                    raw: Some(msg.clone()),
                });

                if inbound_tx.send(event).is_err() {
                    break; // Router channel closed
                }
            }

            ClientMessage::VoiceStart { id, sample_rate, .. } => {
                voice_state = Some(VoiceAccumulator {
                    id,
                    chunks: Vec::new(),
                    sample_rate,
                });
                let _ = send_to_session(&ws_write, &ServerMessage::Listening);
            }

            ClientMessage::VoiceChunk { data, .. } => {
                if let Some(ref mut acc) = voice_state {
                    // Decode base64 to raw bytes
                    match base64_decode(&data) {
                        Ok(bytes) => acc.chunks.push(bytes),
                        Err(e) => tracing::warn!(error = %e, "Native: bad base64 audio chunk"),
                    }
                }
            }

            ClientMessage::VoiceEnd { .. } => {
                if let Some(acc) = voice_state.take() {
                    let _ = send_to_session(&ws_write, &ServerMessage::Thinking);

                    // Process voice: decode opus → STT → push as text event
                    match process_voice(&acc) {
                        Ok(transcribed_text) => {
                            // Send transcription back to client
                            let _ = send_to_session(&ws_write, &ServerMessage::Transcription {
                                voice_id: acc.id.clone(),
                                text: transcribed_text.clone(),
                            });

                            // Push as text message to router
                            let msg_id = format!("voice_{}", chrono::Utc::now().timestamp_millis());
                            let event = InboundEvent::Message(InboundMessage {
                                event_id: msg_id.clone(),
                                conversation: ConversationRef {
                                    provider: "native".into(),
                                    kind: ConversationKind::Direct,
                                    id: session_id.clone(),
                                    parent_id: None,
                                    title: None,
                                },
                                message: MessageRef {
                                    provider: "native".into(),
                                    id: msg_id,
                                },
                                sender: ActorRef {
                                    id: client_id.clone(),
                                    display_name: client_name.clone(),
                                    is_bot: false,
                                },
                                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                                content: MessageContent::Text { text: transcribed_text },
                                reply_to: None,
                                mentions_ai: true,
                                raw: None,
                            });

                            if inbound_tx.send(event).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Native: voice processing failed");
                            let _ = send_to_session(&ws_write, &ServerMessage::Text {
                                id: format!("err_{}", chrono::Utc::now().timestamp_millis()),
                                text: "Sorry, I couldn't understand the audio.".into(),
                                is_final: true,
                            });
                        }
                    }
                }
            }

            ClientMessage::Typing => {
                // Client is typing — could update presence, but no action needed
            }

            ClientMessage::Ping { ts } => {
                let _ = send_to_session(&ws_write, &ServerMessage::Pong {
                    ts: ts.unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
                });
            }

            ClientMessage::Auth { .. } => {
                // Already authenticated, ignore duplicate auth
            }
        }
    }

    // Clean up session
    if let Ok(mut map) = sessions.lock() {
        map.remove(&session_id_clone);
    }
    tracing::info!(session = %session_id_clone, "Native: session ended");
}

// ── Auth handshake ──────────────────────────────────────────────────────────

fn wait_for_auth(
    ws: &Arc<Mutex<WebSocket<TcpStream>>>,
    ws_write: &Arc<Mutex<WebSocket<TcpStream>>>,
    auth_tokens: &[String],
) -> Option<(String, String, String)> {
    // Wait up to 10 seconds for auth message
    for _ in 0..100 {
        let read_result = {
            let mut g = ws.lock().ok()?;
            g.read()
        };
        match read_result {
            Ok(WsMessage::Text(text)) => {
                let msg: ClientMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if let ClientMessage::Auth { token, client_id, client_name } = msg {
                    if auth_tokens.contains(&token) {
                        let session_id = format!("sess_{}_{}", client_id, chrono::Utc::now().timestamp_millis());
                        let name = client_name.unwrap_or_else(|| client_id.clone());

                        let _ = send_to_session(ws_write, &ServerMessage::AuthOk {
                            session_id: session_id.clone(),
                            companion_name: "Yantrik".into(),
                            bond_level: "friend".into(),
                        });

                        return Some((session_id, client_id, name));
                    } else {
                        let _ = send_to_session(ws_write, &ServerMessage::AuthFail {
                            reason: "invalid token".into(),
                        });
                        return None;
                    }
                }
            }
            Ok(WsMessage::Ping(data)) => {
                if let Ok(mut g) = ws.lock() { let _ = g.send(WsMessage::Pong(data)); }
            }
            Ok(WsMessage::Close(_)) => return None,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(_) => return None,
            _ => continue,
        }
    }

    // Timeout
    let _ = send_to_session(ws_write, &ServerMessage::AuthFail {
        reason: "auth timeout".into(),
    });
    None
}

// ── Voice processing ────────────────────────────────────────────────────────

fn process_voice(acc: &VoiceAccumulator) -> Result<String, String> {
    if acc.chunks.is_empty() {
        return Err("no audio chunks".into());
    }

    // Write accumulated opus chunks to a temp OGG file.
    // For V1, we write raw opus frames wrapped in a simple container
    // that ffmpeg can decode. The simplest approach: concatenate as raw
    // and let ffmpeg auto-detect, or wrap in OGG.
    //
    // Actually, the mobile client sends complete opus packets.
    // We write them to a temp file and use ffmpeg to decode to WAV,
    // then feed to Whisper.

    let tmp_dir = std::env::temp_dir();
    let opus_path = tmp_dir.join(format!("yantrik_voice_{}.opus", acc.id));
    let wav_path = tmp_dir.join(format!("yantrik_voice_{}.wav", acc.id));

    // Write raw opus data
    {
        let mut f = std::fs::File::create(&opus_path)
            .map_err(|e| format!("create temp: {e}"))?;
        for chunk in &acc.chunks {
            f.write_all(chunk).map_err(|e| format!("write chunk: {e}"))?;
        }
    }

    // Decode to WAV via ffmpeg
    let ffmpeg_result = std::process::Command::new("ffmpeg")
        .args([
            "-y", "-i", opus_path.to_str().unwrap_or(""),
            "-ar", "16000", "-ac", "1", "-f", "wav",
            wav_path.to_str().unwrap_or(""),
        ])
        .output();

    // Clean up opus temp
    let _ = std::fs::remove_file(&opus_path);

    match ffmpeg_result {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = std::fs::remove_file(&wav_path);
            return Err(format!("ffmpeg failed: {stderr}"));
        }
        Err(e) => {
            return Err(format!("ffmpeg not available: {e}"));
        }
    }

    // Run Whisper STT on the WAV
    // Use whisper.cpp CLI or the API backend — for now, shell out to whisper
    let whisper_result = std::process::Command::new("whisper")
        .args([
            wav_path.to_str().unwrap_or(""),
            "--model", "base",
            "--output_format", "txt",
            "--output_dir", tmp_dir.to_str().unwrap_or("/tmp"),
        ])
        .output();

    // Clean up WAV
    let _ = std::fs::remove_file(&wav_path);

    match whisper_result {
        Ok(output) if output.status.success() => {
            // Whisper writes a .txt file next to the input
            let txt_path = tmp_dir.join(format!("yantrik_voice_{}.txt", acc.id));
            let text = std::fs::read_to_string(&txt_path)
                .unwrap_or_else(|_| String::from_utf8_lossy(&output.stdout).to_string());
            let _ = std::fs::remove_file(&txt_path);
            let text = text.trim().to_string();
            if text.is_empty() {
                Err("STT returned empty transcription".into())
            } else {
                Ok(text)
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("whisper failed: {stderr}"))
        }
        Err(_) => {
            // Whisper not available — try to return a fallback message
            Err("whisper STT not available".into())
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn send_to_session(
    ws: &Arc<Mutex<WebSocket<TcpStream>>>,
    msg: &ServerMessage,
) -> Result<(), ChatError> {
    let json = msg.to_json();
    let mut ws = ws.lock()
        .map_err(|_| ChatError::Internal("ws lock poisoned".into()))?;
    ws.send(WsMessage::Text(json))
        .map_err(|e| ChatError::Network(format!("ws send: {e}")))
}

fn base64_decode(data: &str) -> Result<Vec<u8>, String> {
    // Simple base64 decode without pulling in the base64 crate
    // Use the standard base64 alphabet
    use std::io::Read;
    let mut decoder = base64_reader(data.as_bytes());
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf).map_err(|e| format!("base64: {e}"))?;
    Ok(buf)
}

/// Minimal base64 decoder (avoids adding base64 crate dependency).
fn base64_reader(input: &[u8]) -> impl std::io::Read + '_ {
    Base64Reader { input, pos: 0, buf: [0; 3], buf_len: 0, buf_pos: 0 }
}

struct Base64Reader<'a> {
    input: &'a [u8],
    pos: usize,
    buf: [u8; 3],
    buf_len: usize,
    buf_pos: usize,
}

impl<'a> std::io::Read for Base64Reader<'a> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let mut written = 0;

        while written < out.len() {
            // Drain buffered bytes first
            if self.buf_pos < self.buf_len {
                out[written] = self.buf[self.buf_pos];
                self.buf_pos += 1;
                written += 1;
                continue;
            }

            // Decode next 4-byte block
            let mut block = [0u8; 4];
            let mut count = 0;
            while count < 4 && self.pos < self.input.len() {
                let b = self.input[self.pos];
                self.pos += 1;
                if b == b'=' || b == b'\n' || b == b'\r' || b == b' ' {
                    if b == b'=' { block[count] = 64; count += 1; }
                    continue;
                }
                block[count] = match b {
                    b'A'..=b'Z' => b - b'A',
                    b'a'..=b'z' => b - b'a' + 26,
                    b'0'..=b'9' => b - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => continue,
                };
                count += 1;
            }

            if count == 0 {
                break; // No more input
            }

            // Decode the block
            let n = (block[0] as u32) << 18 | (block[1] as u32) << 12
                | (block[2] as u32) << 6 | block[3] as u32;

            self.buf[0] = (n >> 16) as u8;
            self.buf[1] = (n >> 8) as u8;
            self.buf[2] = n as u8;

            self.buf_len = if block[3] == 64 { if block[2] == 64 { 1 } else { 2 } } else { 3 };
            self.buf_pos = 0;
        }

        Ok(written)
    }
}

// tungstenite::Error doesn't implement kind() — add a helper trait
trait ErrorKindExt {
    fn kind(&self) -> std::io::ErrorKind;
}

impl ErrorKindExt for tungstenite::Error {
    fn kind(&self) -> std::io::ErrorKind {
        match self {
            tungstenite::Error::Io(e) => e.kind(),
            _ => std::io::ErrorKind::Other,
        }
    }
}
