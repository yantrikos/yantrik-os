//! A turn the worker runs on its own account (an urge, a proactive check, a queued task) must not
//! take the worker down with it.
//!
//! The chat path has always run its turn under `catch_unwind`. The think path did not: one panic
//! in an urge's turn ended the worker thread, and every later command was dropped by a send whose
//! error nobody reads, so nothing was logged and only a restart helped. The default panic hook
//! writes to stderr alone, so the panic is also said through `tracing`, where the journal has it.

use std::panic::{catch_unwind, AssertUnwindSafe};

/// Run `turn`; if it panics, say so through `tracing` and give `None`, leaving the worker able to
/// take its next command.
pub fn guarded<T>(what: &str, turn: impl FnOnce() -> T) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(turn)) {
        Ok(done) => Some(done),
        Err(payload) => {
            let why = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no message".to_string());
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
