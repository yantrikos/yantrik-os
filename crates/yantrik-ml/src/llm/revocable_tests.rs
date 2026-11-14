//! A removed provider is cut at the press: nothing is sent after it, nothing in flight is
//! answered, and the key is no longer held. Each test fails without the piece it names.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use anyhow::Result;

use super::{FallbackLLM, LlmSwitch, Stopped, NO_PROVIDER};
use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// Counts the calls it gets; the first one can be held inside until released.
struct Counting {
    calls: AtomicUsize,
    entered: Option<mpsc::SyncSender<()>>,
    release: std::sync::Mutex<Option<mpsc::Receiver<()>>>,
}

impl Counting {
    fn plain() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            entered: None,
            release: Default::default(),
        })
    }

    /// One whose first call says it is inside, then waits to be released.
    fn held() -> (Arc<Self>, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, inside) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::channel();
        let stub = Arc::new(Self {
            calls: AtomicUsize::new(0),
            entered: Some(entered),
            release: std::sync::Mutex::new(Some(wait)),
        });
        (stub, inside, release)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn answer(text: &str) -> LLMResponse {
    LLMResponse {
        text: text.into(),
        prompt_tokens: 0,
        completion_tokens: 0,
        tool_calls: Vec::new(),
        api_tool_calls: Vec::new(),
        stop_reason: "stop".into(),
    }
}

impl LLMBackend for Counting {
    fn chat(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = &self.entered {
            let _ = entered.send(());
            let wait = self.release.lock().unwrap().take();
            if let Some(wait) = wait {
                let _ = wait.recv_timeout(Duration::from_secs(10));
            }
        }
        Ok(answer("from the removed provider"))
    }

    fn chat_streaming(
        &self,
        m: &[ChatMessage],
        c: &GenerationConfig,
        t: Option<&[serde_json::Value]>,
        _: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        self.chat(m, c, t)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        Ok(text.len())
    }

    fn backend_name(&self) -> &str {
        "counting"
    }
}

fn is_stopped(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Stopped>().is_some()
}

#[test]
fn after_the_cut_a_call_is_refused_before_it_reaches_the_backend() {
    let inner = Counting::plain();
    let switch = LlmSwitch::new();
    let armed = switch.arm(inner.clone());
    assert_eq!(armed.backend_name(), "counting");
    armed
        .chat(&[], &GenerationConfig::default(), None)
        .expect("live before the cut");
    assert_eq!(inner.calls(), 1);

    switch.cut();

    let refused = armed
        .chat(&[], &GenerationConfig::default(), None)
        .expect_err("cut");
    assert!(is_stopped(&refused));
    assert_eq!(refused.to_string(), NO_PROVIDER);
    let mut shown = String::new();
    assert!(armed
        .chat_streaming(&[], &GenerationConfig::default(), None, &mut |t| shown
            .push_str(t))
        .is_err());
    assert!(shown.is_empty());
    assert_eq!(
        inner.calls(),
        1,
        "nothing reached the backend after the cut"
    );
    assert_eq!(
        armed.backend_name(),
        "none",
        "a cut backend names itself so the companion answers at once"
    );
}

#[test]
fn a_clone_handed_out_before_the_cut_is_cut_too() {
    let inner = Counting::plain();
    let switch = LlmSwitch::new();
    let armed = switch.arm(inner.clone());
    let judge = armed.clone();
    let sub_agent = armed.clone();
    switch.cut();
    for clone in [armed, judge, sub_agent] {
        assert!(is_stopped(
            &clone
                .chat(&[], &GenerationConfig::default(), None)
                .unwrap_err()
        ));
    }
    assert_eq!(inner.calls(), 0);
}

#[test]
fn a_call_in_flight_at_the_cut_is_thrown_away_and_the_key_is_let_go() {
    let (inner, inside, release) = Counting::held();
    let probe: Arc<dyn LLMBackend> = inner.clone();
    let switch = LlmSwitch::new();
    let armed = switch.arm(inner);
    let running = armed.clone();
    let turn = std::thread::spawn(move || running.chat(&[], &GenerationConfig::default(), None));
    inside
        .recv_timeout(Duration::from_secs(10))
        .expect("the call is inside the backend");

    switch.cut();
    release.send(()).unwrap();

    let result = turn.join().unwrap();
    assert!(is_stopped(&result.expect_err("the late answer is dropped")));
    assert_eq!(
        Arc::strong_count(&probe),
        1,
        "no reference to the backend, and so to its key, is left"
    );
    assert_eq!(probe.backend_name(), "counting");
}

#[test]
fn arming_a_new_backend_cuts_the_one_it_replaces() {
    let (old, new) = (Counting::plain(), Counting::plain());
    let switch = LlmSwitch::new();
    let before = switch.arm(old.clone());
    let after = switch.arm(new.clone());
    assert!(is_stopped(
        &before
            .chat(&[], &GenerationConfig::default(), None)
            .unwrap_err()
    ));
    after
        .chat(&[], &GenerationConfig::default(), None)
        .expect("the new one is live");
    assert_eq!((old.calls(), new.calls()), (0, 1));
}

#[test]
fn the_epoch_moves_on_a_cut_and_not_on_arming() {
    let switch = LlmSwitch::new();
    let start = switch.epoch();
    switch.arm(Counting::plain());
    switch.arm(Counting::plain());
    assert_eq!(switch.epoch(), start, "arming is not a stop");
    switch.cut();
    assert_ne!(switch.epoch(), start, "a reload asked for before this is stale");
    let after = switch.epoch();
    switch.cut();
    assert_ne!(switch.epoch(), after, "every stop moves it, with or without a backend armed");
}

#[test]
fn releasing_the_switch_cuts_what_is_armed_and_keeps_the_epoch() {
    let switch = LlmSwitch::new();
    let live = switch.arm(Counting::plain());
    let asked_at = switch.epoch();
    switch.release();
    assert!(is_stopped(&live.chat(&[], &GenerationConfig::default(), None).unwrap_err()));
    assert_eq!(switch.epoch(), asked_at, "a reload asked for since the press is still wanted");
}

#[test]
fn a_backend_asked_for_before_a_cut_is_not_armed_after_it() {
    let switch = LlmSwitch::new();
    let asked_at = switch.epoch();
    let live = switch.arm(Counting::plain());
    switch.cut();
    let late = Counting::plain();
    let flag = super::super::new_cut_flag();
    assert!(switch.arm_flagged_at(asked_at, flag, late.clone()).is_none());
    assert_eq!(late.calls(), 0);
    assert!(is_stopped(&live.chat(&[], &GenerationConfig::default(), None).unwrap_err()));
    // Asked for after the cut, it is armed.
    let fresh = switch
        .arm_flagged_at(switch.epoch(), super::super::new_cut_flag(), Counting::plain())
        .expect("asked for after the cut");
    fresh.chat(&[], &GenerationConfig::default(), None).expect("live");
}

#[test]
fn a_stopped_primary_is_not_covered_for_by_the_fallback() {
    let (primary, fallback) = (Counting::plain(), Counting::plain());
    let switch = LlmSwitch::new();
    let armed = switch.arm(primary);
    let wrapped = FallbackLLM::with_fallback(armed, fallback.clone());
    switch.cut();
    assert!(is_stopped(
        &wrapped
            .chat(&[], &GenerationConfig::default(), None)
            .unwrap_err()
    ));
    assert!(is_stopped(
        &wrapped
            .chat_streaming(&[], &GenerationConfig::default(), None, &mut |_| {})
            .unwrap_err()
    ));
    assert_eq!(
        fallback.calls(),
        0,
        "a stop is not a failure: nothing is sent to the fallback"
    );
}

#[cfg(feature = "api-llm")]
mod stream {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;

    use super::*;
    use crate::llm::ApiLLM;

    const SSE_WORD: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"word \"}}]}\n\n";
    /// What Ollama's `/api/chat` streams: one JSON object a line.
    const OLLAMA_WORD: &str = "{\"message\":{\"content\":\"word \"},\"done\":false}\n";

    /// A server that streams one chunk every 40 ms, for ever, and counts what it is asked.
    /// Sending fails once the client has closed the socket, which ends the thread.
    fn slow_stream(chunk: &'static str) -> (String, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let (count, saw_close) = (requests.clone(), closed.clone());
        std::thread::spawn(move || {
            while let Ok((mut conn, _)) = listener.accept() {
                count.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                if conn.write_all(head.as_bytes()).is_err() {
                    continue;
                }
                for _ in 0..500 {
                    if conn.write_all(chunk.as_bytes()).is_err() {
                        saw_close.store(true, Ordering::SeqCst);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
            }
        });
        (url, requests, closed)
    }

    fn armed_api(url: &str, switch: &LlmSwitch) -> Arc<dyn LLMBackend> {
        let cut = super::super::new_cut_flag();
        let api = ApiLLM::new(url, Some("fake-key-for-tests".into()), "stub")
            .for_provider("custom")
            .with_cut(cut.clone());
        switch.arm_flagged(cut, Arc::new(api))
    }

    #[test]
    fn a_reply_being_streamed_is_closed_within_a_chunk_of_the_cut() {
        let (url, requests, closed) = slow_stream(SSE_WORD);
        let switch = Arc::new(LlmSwitch::new());
        let armed = armed_api(&url, &switch);
        let (words, seen) = mpsc::channel::<()>();
        let turn = std::thread::spawn(move || {
            armed.chat_streaming(&[], &GenerationConfig::default(), None, &mut |_| {
                let _ = words.send(());
            })
        });
        seen.recv_timeout(Duration::from_secs(10))
            .expect("the stream has begun");

        let cut_at = std::time::Instant::now();
        switch.cut();
        let result = turn.join().unwrap();

        assert!(is_stopped(&result.expect_err("the stream is cut")));
        assert!(
            cut_at.elapsed() < Duration::from_secs(2),
            "ended within a chunk or two: {:?}",
            cut_at.elapsed()
        );
        let waited = std::time::Instant::now();
        while !closed.load(Ordering::SeqCst) && waited.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            closed.load(Ordering::SeqCst),
            "the client closed its socket"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    /// Run `call` on its own thread and give it two seconds after the cut: a call that is not
    /// closed by the cut would sit for the client's 300 s timeout.
    fn cut_then_wait(
        switch: &LlmSwitch,
        call: impl FnOnce() -> Result<LLMResponse> + Send + 'static,
    ) -> Result<LLMResponse> {
        let (done, result) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(call());
        });
        std::thread::sleep(Duration::from_millis(300));
        switch.cut();
        result
            .recv_timeout(Duration::from_secs(2))
            .expect("the call ended within two seconds of the cut")
    }

    fn closed_soon(closed: &AtomicBool) -> bool {
        let waited = std::time::Instant::now();
        while !closed.load(Ordering::SeqCst) && waited.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        closed.load(Ordering::SeqCst)
    }

    #[test]
    fn a_whole_answer_being_sent_is_closed_within_a_line_of_the_cut() {
        let (url, _, closed) = slow_stream(SSE_WORD);
        let switch = LlmSwitch::new();
        let armed = armed_api(&url, &switch);
        let result = cut_then_wait(&switch, move || {
            armed.chat(&[], &GenerationConfig::default(), None)
        });
        assert!(is_stopped(&result.expect_err("the call is cut")));
        assert!(closed_soon(&closed), "the client closed its socket");
    }

    #[test]
    fn an_ollama_whole_answer_being_sent_is_closed_within_a_line_of_the_cut() {
        let (url, _, closed) = slow_stream(OLLAMA_WORD);
        let switch = LlmSwitch::new();
        let cut = super::super::new_cut_flag();
        let api = ApiLLM::with_flavor(&url, Some("fake-key-for-tests".into()), "stub", true)
            .with_cut(cut.clone());
        let armed = switch.arm_flagged(cut, Arc::new(api));
        let result = cut_then_wait(&switch, move || {
            armed.chat(&[], &GenerationConfig::default(), None)
        });
        assert!(is_stopped(&result.expect_err("the call is cut")));
        assert!(closed_soon(&closed), "the client closed its socket");
    }

    /// A server that takes the request and then says nothing: a model still thinking. It sets
    /// the flag when the client closes.
    fn silent_server() -> (String, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let closed = Arc::new(AtomicBool::new(false));
        let saw_close = closed.clone();
        std::thread::spawn(move || {
            while let Ok((mut conn, _)) = listener.accept() {
                let mut sink = [0u8; 4096];
                // The request, then nothing but the client's close, which reads as zero bytes.
                while let Ok(n) = conn.read(&mut sink) {
                    if n == 0 {
                        saw_close.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });
        (url, closed)
    }

    #[test]
    fn a_call_waiting_for_the_first_byte_is_closed_by_the_cut() {
        let (url, closed) = silent_server();
        let switch = LlmSwitch::new();
        let armed = armed_api(&url, &switch);
        let result = cut_then_wait(&switch, move || {
            armed.chat(&[], &GenerationConfig::default(), None)
        });
        assert!(is_stopped(&result.expect_err("the call is cut")));
        assert!(closed_soon(&closed), "the client closed its socket");
    }

    #[test]
    fn a_fallback_call_in_flight_is_closed_by_the_cut() {
        use crate::llm::{FallbackConfig, FallbackLLM};
        let (url, closed) = silent_server();
        let switch = LlmSwitch::new();
        let cut = super::super::new_cut_flag();
        // The primary refuses at once (nothing listens on port 1), so the call goes to the fallback.
        let primary = ApiLLM::new("http://127.0.0.1:1/v1", None, "stub");
        let fallback = FallbackLLM::new(
            Arc::new(primary),
            Some(FallbackConfig::Api { base_url: url, model: "stub".into() }),
        )
        .with_cut(cut.clone());
        let armed = switch.arm_flagged(cut, Arc::new(fallback));
        let result = cut_then_wait(&switch, move || {
            armed.chat(&[], &GenerationConfig::default(), None)
        });
        assert!(is_stopped(&result.expect_err("the call is cut")));
        assert!(closed_soon(&closed), "the fallback's socket was closed");
    }

    #[test]
    fn a_finished_call_leaves_no_socket_for_the_cut_to_hold() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut conn, _)) = listener.accept() {
                let mut sink = [0u8; 4096];
                let _ = conn.read(&mut sink);
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{SSE_WORD}data: [DONE]\n\n"
                );
                let _ = conn.write_all(reply.as_bytes());
            }
        });
        let cut = super::super::new_cut_flag();
        let api = ApiLLM::new(&url, Some("fake-key-for-tests".into()), "stub").with_cut(cut.clone());
        api.chat(&[], &GenerationConfig::default(), None)
            .expect("the server answered");
        assert_eq!(cut.open_sockets(), 0, "its extra handle would keep the connection open");
    }

    #[test]
    fn a_cut_client_sends_no_request_at_all() {
        let (url, requests, _) = slow_stream(SSE_WORD);
        let cut = super::super::new_cut_flag();
        let api =
            ApiLLM::new(&url, Some("fake-key-for-tests".into()), "stub").with_cut(cut.clone());
        cut.cut();
        // Straight at the client, with no `Revocable` in front: the client's own flag is what
        // stops the call that a wrapper already past its check would make.
        assert!(is_stopped(
            &api.chat(&[], &GenerationConfig::default(), None)
                .unwrap_err()
        ));
        assert!(is_stopped(
            &api.chat_streaming(&[], &GenerationConfig::default(), None, &mut |_| {})
                .unwrap_err()
        ));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }
}
