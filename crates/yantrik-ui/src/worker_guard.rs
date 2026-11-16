//! A turn the worker runs on its own account (an urge, a proactive check, a queued task) must not
//! take the worker down with it.
//!
//! The chat path has always run its turn under `catch_unwind`. The think path did not: one panic
//! in an urge's turn ended the worker thread, and every later command was dropped by a send whose
//! error nobody reads, so nothing was logged and only a restart helped. The default panic hook
//! writes to stderr alone, so the panic is also said through `tracing`, where the journal has it.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The longest summary of a panic the log keeps, in chars.
const SUMMARY_CAP: usize = 120;

/// What a caught panic may say in the log: its text up to the first backtick or quote, at most
/// [`SUMMARY_CAP`] chars. A string-slicing panic quotes up to 256 bytes of the string it cut,
/// which can be a prompt or a tool result; the quote starts at one of those two marks, so
/// nothing the turn was holding gets past this.
pub fn panic_summary(payload: &(dyn Any + Send)) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message");
    let cut = text.find(['`', '\'']).map_or(text, |at| &text[..at]);
    cut.chars().take(SUMMARY_CAP).collect::<String>().trim_end().to_string()
}

/// Run `turn`; if it panics, say so through `tracing` and give `None`, leaving the worker able to
/// take its next command.
pub fn guarded<T>(what: &str, turn: impl FnOnce() -> T) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(turn)) {
        Ok(done) => Some(done),
        Err(payload) => {
            let why = panic_summary(payload.as_ref());
            tracing::error!(turn = what, panic = %why, "{what} panicked; the worker carries on");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_panic_is_logged_without_the_text_it_was_holding() {
        let log = Log::default();
        let said = log.clone();
        let subscriber = tracing_subscriber::fmt().with_writer(move || said.clone()).with_ansi(false).finish();
        let held = tracing::subscriber::with_default(subscriber, || {
            guarded("EXECUTE urge", || {
                let s = String::from("é-SECRET-TEXT");
                s[0..1].len()
            })
        });
        assert_eq!(held, None);
        let text = String::from_utf8(log.0.lock().unwrap_or_else(|e| e.into_inner()).clone()).unwrap_or_default();
        assert!(text.contains("EXECUTE urge panicked") && text.contains("byte index 1 is not a char boundary"), "{text:?}");
        assert!(!text.contains("SECRET"), "the held text reached the log: {text:?}");
    }

    /// Newer compilers leave the string out of a slicing panic's text; older ones quote it. The
    /// log must be clean under either, so the quoting form is spelled out here.
    #[test]
    fn a_panic_that_quotes_the_string_it_cut_is_logged_without_the_quote() {
        let log = Log::default();
        let said = log.clone();
        let subscriber = tracing_subscriber::fmt().with_writer(move || said.clone()).with_ansi(false).finish();
        tracing::subscriber::with_default(subscriber, || {
            guarded("EXECUTE urge", || -> () {
                panic!("byte index 1 is not a char boundary; it is inside 'é' (bytes 0..2) of `é-SECRET-TEXT`")
            })
        });
        let text = String::from_utf8(log.0.lock().unwrap_or_else(|e| e.into_inner()).clone()).unwrap_or_default();
        assert!(text.contains("byte index 1 is not a char boundary"), "{text:?}");
        assert!(!text.contains("SECRET"), "the held text reached the log: {text:?}");
    }

    #[test]
    fn a_summary_is_capped_and_cut_before_any_quote() {
        assert_eq!(panic_summary(&"boom"), "boom");
        assert_eq!(panic_summary(&String::from("bad `secret` here")), "bad");
        assert_eq!(panic_summary(&"x".repeat(500)).chars().count(), SUMMARY_CAP);
        assert_eq!(panic_summary(&7_u8), "no message");
    }

    #[test]
    fn a_panicking_turn_is_logged_and_the_worker_answers_the_next_one() {
        let log = Log::default();
        let said = log.clone();
        let subscriber = tracing_subscriber::fmt().with_writer(move || said.clone()).with_ansi(false).finish();
        let (first, second) = tracing::subscriber::with_default(subscriber, || {
            let first = guarded("EXECUTE urge", || -> String { panic!("boom") });
            let second = guarded("EXECUTE urge", || "hello".to_string());
            (first, second)
        });
        assert_eq!(first, None, "the panic must not leave the helper");
        assert_eq!(second.as_deref(), Some("hello"), "the next turn is taken");
        let text = String::from_utf8(log.0.lock().unwrap_or_else(|e| e.into_inner()).clone()).unwrap_or_default();
        assert!(text.contains("EXECUTE urge panicked") && text.contains("boom"), "the panic is in the log: {text:?}");
    }
}
