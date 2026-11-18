//! The flag a cut sets, and the sockets it closes.
//!
//! One [`CutFlag`] is shared by a [`Revocable`](super::Revocable) and the `ApiLLM` inside it. A
//! cut sets the flag, so a request not yet sent is not sent and a stream is dropped at its next
//! line, and shuts down every socket the client has open, so a call still waiting for the first
//! byte, or for the rest of a body, is released at once and not when the server answers or the
//! client times out.

use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

/// Set when the person removes the provider; holds the sockets of the calls in flight.
#[derive(Default)]
pub struct CutFlag {
    cut: AtomicBool,
    /// A second handle on each open socket, with the thread whose call opened it.
    sockets: Mutex<Vec<(ThreadId, TcpStream)>>,
}

impl std::fmt::Debug for CutFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CutFlag").field("cut", &self.is_cut()).finish()
    }
}

impl CutFlag {
    pub fn is_cut(&self) -> bool {
        self.cut.load(Ordering::SeqCst)
    }

    /// Set the flag and shut down every socket in flight, both directions. Idempotent.
    pub fn cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
        let open = std::mem::take(&mut *self.lock());
        for (_, socket) in open {
            // A socket the server closed already has nothing left to shut.
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    /// Remember `socket` so a cut can close it. Returns false, with the socket shut, when the cut
    /// came first: the caller must not use it.
    pub(super) fn register(&self, socket: &TcpStream) -> bool {
        let Ok(handle) = socket.try_clone() else {
            // Without a handle the cut could not close it: do not run the call on it.
            return false;
        };
        self.lock().push((std::thread::current().id(), handle));
        if self.is_cut() {
            // The cut may have drained the list just before the push: close it here.
            self.release_thread();
            let _ = socket.shutdown(Shutdown::Both);
            return false;
        }
        true
    }

    /// Forget the sockets this thread's call opened. Without this the extra handle would keep a
    /// finished call's connection open.
    pub(super) fn release_thread(&self) {
        let me = std::thread::current().id();
        self.lock().retain(|(owner, _)| *owner != me);
    }

    /// Ends the sockets' registration when the call on this thread ends, however it ends.
    pub(super) fn scope(self: &Arc<Self>) -> CallScope {
        CallScope(self.clone())
    }

    #[cfg(test)]
    pub(super) fn open_sockets(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(ThreadId, TcpStream)>> {
        self.sockets.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// See [`CutFlag::scope`].
pub(super) struct CallScope(Arc<CutFlag>);

impl Drop for CallScope {
    fn drop(&mut self) {
        self.0.release_thread();
    }
}
