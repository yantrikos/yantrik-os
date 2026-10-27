//! The instance's end of a view helper's socket: the frames and requests a Mind's window sends,
//! and what the helper hears from its display, brought back to the UI thread.
//!
//! The helper names its socket in its hand-over. Before anything is sent the instance holds it to
//! what a helper's socket is: in this session's private socket directory, named [`VIEW_PREFIX`]
//! something, a socket of this account's; and the process at the other end, by the kernel's word,
//! is the one that handed the launch over. So a Mind's window is only ever drawn for the launch
//! the instance just checked was a Mind's.
//!
//! Writing is a thread of its own, so the UI thread never waits on a helper. Requests (title,
//! raise, close) queue in order; a frame waits in a slot of one, and a frame offered while the
//! slot is taken is refused, which the window answers by sending the whole frame next time.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use super::streamed::Sink;
use super::wire::{FromHelper, ToHelper};
use super::VIEW_PREFIX;

/// What a link heard, on the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    Message(FromHelper),
    /// The helper is gone: its socket closed, or it sent what no helper sends.
    Gone,
}

#[derive(Default)]
struct Outbox {
    control: VecDeque<Vec<u8>>,
    frame: Option<Vec<u8>>,
    closed: bool,
}

type Shared = Arc<(Mutex<Outbox>, Condvar)>;

/// The instance's end of one helper's socket. Dropping it tells the helper the window closed.
pub struct Link {
    id: u64,
    outbox: Shared,
}

impl Link {
    /// The key what it hears is routed by ([`on_ui`]).
    pub fn id(&self) -> u64 {
        self.id
    }

    fn push(&self, body: Vec<u8>, frame: bool) -> bool {
        let (lock, wake) = &*self.outbox;
        let mut out = lock.lock().unwrap_or_else(|p| p.into_inner());
        if out.closed {
            return false;
        }
        if frame {
            if out.frame.is_some() {
                return false;
            }
            out.frame = Some(body);
        } else {
            out.control.push_back(body);
        }
        wake.notify_one();
        true
    }
}

impl Sink for Link {
    fn send(&self, message: ToHelper) -> bool {
        let frame = matches!(message, ToHelper::Frame { .. });
        self.push(message.encode(), frame)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.push(ToHelper::Close.encode(), false);
        let (lock, wake) = &*self.outbox;
        lock.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
        wake.notify_one();
        forget(self.id);
    }
}

/// What one link's window does with what it hears.
type Route = Box<dyn Fn(Heard)>;

thread_local! {
    /// What each link's window does with what it hears, on the UI thread.
    static ROUTES: RefCell<HashMap<u64, Route>> = RefCell::new(HashMap::new());
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What the window of link `id` does with what its helper says. Called on the UI thread, once
/// the window is made.
pub fn on_ui(id: u64, route: Route) {
    ROUTES.with(|r| r.borrow_mut().insert(id, route));
}

fn forget(id: u64) {
    let _ = ROUTES.try_with(|r| r.borrow_mut().remove(&id));
}

fn route(id: u64, heard: Heard) {
    // Taken out while it runs: what it does may open or close a link.
    let taken = ROUTES.with(|r| r.borrow_mut().remove(&id));
    if let Some(f) = taken {
        let gone = heard == Heard::Gone;
        f(heard);
        if !gone {
            ROUTES.with(|r| {
                r.borrow_mut().entry(id).or_insert(f);
            });
        }
    }
}

/// Whether `path` is where a view helper of this session listens: in the socket directory, named
/// as a helper names its socket, and a socket of this account's.
pub fn acceptable(path: &Path, dir: &Path) -> Result<(), String> {
    let named = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n.starts_with(VIEW_PREFIX)
            && n.ends_with(".sock")
            && n.len() <= 108
            && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    });
    if !named || path.parent() != Some(dir) {
        return Err("a view helper's socket is a `view-` socket in this session's socket directory".into());
    }
    is_our_socket(path)
}

#[cfg(unix)]
fn is_our_socket(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("no view helper at {}: {e}", path.display()))?;
    // SAFETY: geteuid cannot fail and touches no memory.
    let me = unsafe { libc::geteuid() };
    if !meta.file_type().is_socket() || meta.uid() != me {
        return Err("that is not a view helper's socket of this account's".into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn is_our_socket(_path: &Path) -> Result<(), String> {
    Err("a Mind's window needs a Unix socket to its view helper".into())
}

/// Connects to the helper listening at `path`, which must be the process `asker` (the kernel's
/// word on who handed the launch over; `None` when there was no socket call, as in a test).
#[cfg(unix)]
pub fn connect(path: &Path, asker: Option<i32>) -> Result<Link, String> {
    use std::os::unix::net::UnixStream;
    acceptable(path, &yantrik_ipc_transport::server::socket_dir())?;
    let stream = UnixStream::connect(path).map_err(|e| format!("the view helper did not answer: {e}"))?;
    peer_is(&stream, asker)?;
    let reading = stream.try_clone().map_err(|e| format!("the view helper's socket: {e}"))?;
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let outbox: Shared = Arc::new((Mutex::new(Outbox::default()), Condvar::new()));
    let writing = outbox.clone();
    std::thread::Builder::new()
        .name("yantrik-view-out".into())
        .spawn(move || write_loop(stream, writing))
        .map_err(|e| format!("no thread for the view helper: {e}"))?;
    std::thread::Builder::new()
        .name("yantrik-view-in".into())
        .spawn(move || read_loop(reading, id))
        .map_err(|e| format!("no thread for the view helper: {e}"))?;
    Ok(Link { id, outbox })
}

#[cfg(not(unix))]
pub fn connect(_path: &Path, _asker: Option<i32>) -> Result<Link, String> {
    Err("a Mind's window needs a Unix socket to its view helper".into())
}

/// The process at the other end of `stream`, by the kernel's word: its pid, and whether it is of
/// this account. `None` where the kernel cannot say.
#[cfg(target_os = "linux")]
pub(crate) fn peer(stream: &std::os::unix::net::UnixStream) -> Option<(i32, bool)> {
    use std::os::unix::io::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the buffer and its length are the ucred the call fills, on a socket we own.
    let got = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    // SAFETY: geteuid cannot fail and touches no memory.
    let me = unsafe { libc::geteuid() };
    (got == 0).then_some((cred.pid, cred.uid == me))
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn peer(_stream: &std::os::unix::net::UnixStream) -> Option<(i32, bool)> {
    None
}

/// The process at the other end of `stream` is `asker`, and of this account.
#[cfg(unix)]
fn peer_is(stream: &std::os::unix::net::UnixStream, asker: Option<i32>) -> Result<(), String> {
    match peer(stream) {
        Some((_, false)) => Err("the view helper is not of this account".into()),
        Some((pid, true)) if asker.is_some_and(|a| a != pid) => {
            Err("the view helper is not the launch that handed over".into())
        }
        Some(_) => Ok(()),
        // A kernel that cannot say: the socket's own owner and place were checked.
        None if cfg!(target_os = "linux") => Err("the view helper could not be identified".into()),
        None => Ok(()),
    }
}

#[cfg(unix)]
fn write_loop(mut stream: std::os::unix::net::UnixStream, outbox: Shared) {
    let (lock, wake) = &*outbox;
    loop {
        let next = {
            let mut out = lock.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(control) = out.control.pop_front() {
                    break Some(control);
                }
                if let Some(frame) = out.frame.take() {
                    break Some(frame);
                }
                if out.closed {
                    break None;
                }
                out = wake.wait(out).unwrap_or_else(|p| p.into_inner());
            }
        };
        let Some(body) = next else { break };
        if super::wire::send(&mut stream, &body).is_err() {
            lock.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
            break;
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

#[cfg(unix)]
fn read_loop(mut stream: std::os::unix::net::UnixStream, id: u64) {
    loop {
        let heard = match super::wire::receive(&mut stream) {
            Ok(Some(body)) => match FromHelper::decode(&body) {
                Ok(message) => Heard::Message(message),
                Err(e) => {
                    tracing::warn!(error = %e, "the view helper sent what no helper sends; its window closes");
                    Heard::Gone
                }
            },
            Ok(None) | Err(_) => Heard::Gone,
        };
        let gone = heard == Heard::Gone;
        let _ = slint::invoke_from_event_loop(move || route(id, heard));
        if gone {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_view_socket_in_the_session_directory_is_taken() {
        let dir = Path::new("/run/user/1000/yantrik");
        let bad = |p: &str| acceptable(Path::new(p), dir).unwrap_err();
        assert!(bad("/run/user/1000/yantrik/app-notes.sock").contains("view-"), "an app's own socket");
        assert!(bad("/tmp/view-notes-1.sock").contains("socket directory"));
        assert!(bad("/run/user/1000/yantrik/view-notes-1.txt").contains("view-"));
        assert!(bad("/run/user/1000/yantrik/view-no tes.sock").contains("view-"));
        assert!(bad("/run/user/1000/yantrik/../view-notes-1.sock").contains("view-"));
        // Named right but not there: refused for what it is.
        assert!(bad("/run/user/1000/yantrik/view-notes-1.sock").contains("view helper"));
    }

    #[test]
    fn a_frame_waits_in_one_slot_and_requests_queue_behind_none() {
        let link = Link { id: 0, outbox: Arc::new((Mutex::new(Outbox::default()), Condvar::new())) };
        let frame = || ToHelper::Frame { width: 1, height: 1, x: 0, y: 0, w: 1, h: 1, rgb: vec![1, 2, 3] };
        assert!(link.send(frame()));
        assert!(!link.send(frame()), "a second frame while the first waits is refused");
        assert!(link.send(ToHelper::Raise) && link.send(ToHelper::Drag), "requests are never refused");
        let outbox = link.outbox.clone();
        drop(link);
        let out = outbox.0.lock().unwrap();
        assert!(out.closed);
        assert_eq!(out.control.back(), Some(&ToHelper::Close.encode()), "the helper is told the window closed");
    }
}
