//! What was written, and by whom.
//!
//! `crates/yantrik-os/src/files.rs` already watches directories with inotify, and inotify has one
//! shape of blindness that cannot be worked around: it reports that a file changed and never who
//! changed it. "Something wrote `report.odt`" is a fact with nowhere to go. "LibreOffice saved
//! `report.odt`" is a thing a person would say.
//!
//! fanotify hands over the acting pid with every event, which is the whole reason to prefer it and
//! most of the reason this service is privileged.
//!
//! # Why there are two groups
//!
//! The first version of this file watched `FAN_CLOSE_WRITE` and called it a save. That is wrong,
//! and wrong in the case that matters most. `FAN_CLOSE_WRITE` means *a writable descriptor was
//! closed* — nothing more. Every serious editor saves by writing a scratch file and renaming it
//! over the document, because a rename is atomic and a partial write is a lost afternoon. So vim
//! saving `report.odt` closes `.report.odt.swpx`, and the rename that actually replaces the
//! document is a directory-entry change that group never sees.
//!
//! It is worse than reporting the wrong name, because it is not even reliably wrong. fanotify
//! hands over a *descriptor*, and the path comes from reading `/proc/self/fd/N` afterwards — which
//! resolves to whatever that inode is called at the moment we look. Drain the queue before the
//! editor's rename lands and we report `.report.odt.swpx`; drain it after and we report
//! `report.odt`, correctly and entirely by luck. Which one happens is a race with another process.
//! Measured here the rename usually won — which is precisely how a bug like this survives a probe,
//! especially one that only tested `echo probe > note.txt`, the single shape of save that writes
//! in place and is not representative of anything.
//!
//! With both groups the outcome stops depending on the race. Lose it, and the descriptor group
//! reports the scratch file — a [`Kind::Wrote`] — while the rename supplies the save. Win it, and
//! the descriptor group already names the document, so the rename is the same save described
//! twice, which the bus coalesces. Either way: exactly one save, correctly named and attributed.
//!
//! Directory-entry events (`FAN_MOVED_TO` and friends) are not deliverable as descriptors — the
//! event is about a *name in a directory*, not about an open file — so the kernel requires a group
//! initialised with `FAN_REPORT_DFID_NAME`, which reports a file handle for the parent directory
//! plus the entry name. That is a different event layout and cannot share a group with the
//! descriptor-carrying one. Hence two:
//!
//! | group | mask | tells us |
//! |---|---|---|
//! | descriptor | `FAN_CLOSE_WRITE`, `FAN_OPEN_EXEC` | a file was written or run, with a path |
//! | dirent | `FAN_MOVED_TO` | a name in a watched directory now points at something else |
//!
//! # Raw facts and inferred ones are kept apart
//!
//! [`Kind::Wrote`] is what the kernel said. [`Kind::Saved`] is what we think it meant, and carries
//! [`SaveShape`] so a reader can see which observation established it. Nothing is dropped to make
//! the inference tidy: a scratch-file close is still recorded, it simply does not claim to be a
//! save.
//!
//! # Resolving a directory handle without privilege
//!
//! `open_by_handle_at` would turn the kernel's handle back into a path, and needs
//! `CAP_DAC_READ_SEARCH` — which this service gives away deliberately and permanently at startup.
//! So the map is built the other way round and in advance: while still privileged, every directory
//! we mark is put through `name_to_handle_at`, and the resulting handle is remembered against its
//! path. At event time the lookup is a hash of bytes the kernel handed us. Same shape as the
//! descriptors themselves — do the privileged part first, keep only the result.
//!
//! # Two choices worth defending
//!
//! **A close, not a modify.** `FAN_MODIFY` would fire on every write and drown everything else in
//! the ring.
//!
//! **Directory marks, not a filesystem mark.** `FAN_MARK_FILESYSTEM` would catch everything
//! including subdirectories created later, at the cost of being told about every write on the
//! disk and filtering afterwards — the daemon would *receive* events about `~/.ssh` and choose
//! not to report them. Marking only the directories in scope means those events are never
//! delivered at all. The scope becomes structural rather than a promise, which is the same reason
//! Landlock is here.
//!
//! The cost is real and stated plainly: a directory created after start is not watched until the
//! service restarts.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::bus::Bus;
use crate::observation::{Actor, Kind, SaveShape};
use crate::scope::Resolved;

const FAN_CLOEXEC: libc::c_uint = 0x0000_0001;
const FAN_CLASS_NOTIF: libc::c_uint = 0x0000_0000;
/// `FAN_REPORT_DIR_FID | FAN_REPORT_NAME`. Linux 5.9 and later.
const FAN_REPORT_DFID_NAME: libc::c_uint = 0x0000_0400 | 0x0000_0800;
const FAN_MARK_ADD: libc::c_uint = 0x0000_0001;
const FAN_MARK_ONLYDIR: libc::c_uint = 0x0000_0008;
const FAN_EVENT_ON_CHILD: u64 = 0x0800_0000;
const FAN_CLOSE_WRITE: u64 = 0x0000_0008;
const FAN_OPEN_EXEC: u64 = 0x0000_1000;
/// A name in a marked directory started pointing at something else — the second half of a save.
const FAN_MOVED_TO: u64 = 0x0000_0080;

/// `struct fanotify_event_metadata` is 24 bytes: event_len, vers, reserved, metadata_len, mask,
/// fd, pid.
const METADATA_LEN: usize = 24;

/// `FAN_EVENT_INFO_TYPE_DFID_NAME`: parent directory handle followed by the entry name.
const INFO_TYPE_DFID_NAME: u8 = 2;

/// `struct fanotify_event_info_header`: info_type, pad, len.
const INFO_HEADER_LEN: usize = 4;

/// Largest handle the kernel will hand back. `MAX_HANDLE_SZ` in the uapi headers.
const MAX_HANDLE_SZ: usize = 128;

/// How deep to walk a watched directory when placing marks.
///
/// Deep enough for a project tree, shallow enough that pointing the scope at something enormous
/// cannot take minutes at startup.
const MAX_DEPTH: usize = 8;

/// Ceiling on marks. Each is a kernel object; an unbounded walk of a large tree would be a way to
/// exhaust kernel memory by editing a config file.
const MAX_MARKS: usize = 4096;

// ── Identifying a directory the way the kernel does ─────────────────

/// A marked directory, keyed as it will arrive in an event.
///
/// `fsid` comes from `statfs`, the handle from `name_to_handle_at`; together they are what the
/// kernel puts in a `FAN_REPORT_DFID_NAME` record. Comparing raw bytes rather than paths is the
/// point — a path can be renamed out from under us, and the handle cannot.
#[derive(PartialEq, Eq, Hash, Debug, Clone)]
struct DirKey {
    fsid: [u8; 8],
    handle_type: i32,
    handle: Vec<u8>,
}

/// Newtype so the map can cross a module boundary without exposing the layout.
#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub struct DirId(DirKey);

/// Directory handles to paths, for the dirent group.
pub type DirMap = HashMap<DirId, PathBuf>;

/// What [`init_and_mark`] managed to open, and what it could not.
pub struct Watches {
    /// The `FAN_CLOSE_WRITE` / `FAN_OPEN_EXEC` group.
    pub files: libc::c_int,
    /// The `FAN_MOVED_TO` group, when the kernel supports `FAN_REPORT_DFID_NAME`.
    pub renames: Option<libc::c_int>,
    /// Directory handle to path, for resolving rename events.
    pub dirs: DirMap,
    /// How many marks were placed on the descriptor group.
    pub marks: usize,
    /// Why renames are not being watched, when they are not. Reported as a `SourceFailed`
    /// observation rather than logged: a service that has half gone blind must not look like one
    /// where nothing is happening.
    pub renames_unavailable: Option<String>,
}

/// Open both descriptors and place every mark, while still privileged.
///
/// `fanotify_init`, `fanotify_mark` and `name_to_handle_at` all want privilege we are about to
/// give away, so all of it happens once on the main thread before [`crate::caps::drop_all`]. What
/// remains afterwards is a pair of descriptors the kernel writes events to and a map of bytes —
/// which is why this service can hand its privileges back and keep watching.
pub fn init_and_mark(scope: &Resolved) -> Result<Watches, String> {
    if scope.watch.is_empty() {
        return Err("nothing is in scope".into());
    }
    let files = init(FAN_CLASS_NOTIF)?;

    let mut marked = 0usize;
    for root in &scope.watch {
        marked += mark_tree(files, root, scope, &mut 0, &mut MarkKind::Files);
    }
    if marked == 0 {
        // Every directory refused. That is a failure, not a quiet success with nothing to watch.
        // SAFETY: closing our own fd on the failure path.
        unsafe { libc::close(files) };
        return Err("no directory in scope could be marked".into());
    }

    // The rename group is a separate, later kernel feature. Losing it costs us atomic saves, not
    // everything, so it fails on its own terms rather than taking the source down.
    let (renames, dirs, renames_unavailable) = match init(FAN_CLASS_NOTIF | FAN_REPORT_DFID_NAME) {
        Ok(fd) => {
            let mut dirs = DirMap::new();
            let mut unnameable = 0usize;
            let mut placed = 0usize;
            {
                let mut kind = MarkKind::Renames { dirs: &mut dirs, unnameable: &mut unnameable };
                for root in &scope.watch {
                    placed += mark_tree(fd, root, scope, &mut 0, &mut kind);
                }
            }
            if placed == 0 || dirs.is_empty() {
                // SAFETY: our own fd, on the failure path.
                unsafe { libc::close(fd) };
                (
                    None,
                    DirMap::new(),
                    Some(rename_gap(0, unnameable)),
                )
            } else {
                // Up, but perhaps not everywhere. A partial gap is still a gap.
                (Some(fd), dirs, (unnameable > 0).then(|| rename_gap(placed, unnameable)))
            }
        }
        Err(e) => (
            None,
            DirMap::new(),
            // Worth stating in full: on a kernel before 5.9 this is EINVAL and no amount of
            // permission fixes it, and a reader deserves to know that saves will arrive as
            // scratch writes rather than wondering why LibreOffice looks idle.
            Some(format!("{e}; atomic-rename saves will not be seen")),
        ),
    };

    Ok(Watches { files, renames, dirs, marks: marked, renames_unavailable })
}

// ── The descriptor group: writes and execs ──────────────────────────

pub fn run(bus: Bus, scope: &Resolved, fd: libc::c_int) {
    // SAFETY: `fd` came from `init_and_mark` and ownership moves here.
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    let mut buf = [0u8; 8192];
    let mut names = ActorNames::default();

    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                bus.push(
                    Kind::SourceFailed { source: "files".into(), reason: e.to_string() },
                    None,
                );
                tracing::warn!(error = %e, "fanotify stopped");
                return;
            }
        };

        let mut offset = 0usize;
        while offset + METADATA_LEN <= n {
            let event_len =
                u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
            if event_len < METADATA_LEN || offset + event_len > n {
                break;
            }
            let mask = u64::from_ne_bytes(buf[offset + 8..offset + 16].try_into().unwrap());
            let event_fd = i32::from_ne_bytes(buf[offset + 16..offset + 20].try_into().unwrap());
            let pid = i32::from_ne_bytes(buf[offset + 20..offset + 24].try_into().unwrap());
            offset += event_len;

            if event_fd < 0 {
                continue;
            }
            let path = resolve(event_fd);
            // The descriptor the kernel opened for us, closed as soon as the path is read. We
            // never read through it: seeing that a file was saved is the capability; reading what
            // was in it is the one Landlock takes away.
            // SAFETY: `event_fd` came from the kernel in this event and is not used again.
            unsafe { libc::close(event_fd) };

            let Some(path) = path else { continue };
            if !scope.allows(&path) {
                // Should be rare — the marks are already scoped — but a `never` path nested
                // inside a watched one lands here. Counted, never described.
                bus.note_out_of_scope();
                continue;
            }

            let actor = Some(names.of(pid));
            let text = path.to_string_lossy().to_string();
            if mask & FAN_OPEN_EXEC != 0 {
                bus.push(Kind::Executed { path: text }, actor);
            } else if mask & FAN_CLOSE_WRITE != 0 {
                // The whole inference, in one branch. A close on a document is a save; a close on
                // an editor's scratch file is the *machinery* of a save, and the rename that
                // follows is the event. Recorded either way — only the claim changes.
                if is_scratch(&path) {
                    bus.push(Kind::Wrote { path: text }, actor);
                } else {
                    bus.push(Kind::Saved { path: text, how: SaveShape::ClosedWrite }, actor);
                }
            }
        }
    }
}

// ── The dirent group: renames into place ────────────────────────────

pub fn run_renames(bus: Bus, scope: &Resolved, fd: libc::c_int, dirs: DirMap) {
    // SAFETY: `fd` came from `init_and_mark` and ownership moves here.
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    // Handles are variable-length and a busy directory can pack many events into one read; 8 KiB
    // matches the other group, and the kernel simply returns fewer events per read if they are
    // large.
    let mut buf = [0u8; 8192];
    let mut names = ActorNames::default();

    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                bus.push(
                    Kind::SourceFailed { source: "file-renames".into(), reason: e.to_string() },
                    None,
                );
                tracing::warn!(error = %e, "fanotify rename group stopped");
                return;
            }
        };

        for event in DirentEvents::new(&buf[..n]) {
            if event.mask & FAN_MOVED_TO == 0 {
                continue;
            }
            let Some(dir) = dirs.get(&event.dir) else {
                // A handle we never marked: a directory created after startup, or one whose
                // handle we could not take. Counted rather than guessed at — joining a name onto
                // a directory we cannot name would be an invented path.
                bus.note_out_of_scope();
                continue;
            };
            let path = dir.join(&event.name);
            if !scope.allows(&path) {
                bus.note_out_of_scope();
                continue;
            }
            bus.push(
                Kind::Saved { path: path.to_string_lossy().to_string(), how: SaveShape::Replaced },
                Some(names.of(event.pid)),
            );
        }
    }
}

/// One directory-entry event, already parsed.
struct DirentEvent {
    mask: u64,
    pid: i32,
    dir: DirId,
    name: String,
}

/// Walks the events in one `read` of a `FAN_REPORT_DFID_NAME` group.
///
/// Split out from the loop so it can be tested against bytes laid out by hand — the layout is the
/// part of this file most likely to be wrong, and the only part that cannot be checked by reading
/// it.
struct DirentEvents<'a> {
    buf: &'a [u8],
    offset: usize,
}

impl<'a> DirentEvents<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, offset: 0 }
    }
}

impl Iterator for DirentEvents<'_> {
    type Item = DirentEvent;

    fn next(&mut self) -> Option<DirentEvent> {
        while self.offset + METADATA_LEN <= self.buf.len() {
            let base = self.offset;
            let event_len = u32::from_ne_bytes(self.buf[base..base + 4].try_into().ok()?) as usize;
            // `metadata_len` is what the kernel wrote, which is not necessarily our idea of the
            // struct size: a newer kernel may have grown it, and the info records start after it.
            let metadata_len =
                u16::from_ne_bytes(self.buf[base + 6..base + 8].try_into().ok()?) as usize;
            if event_len < METADATA_LEN || metadata_len < METADATA_LEN || metadata_len > event_len {
                return None;
            }
            if base + event_len > self.buf.len() {
                return None;
            }
            let mask = u64::from_ne_bytes(self.buf[base + 8..base + 16].try_into().ok()?);
            let pid = i32::from_ne_bytes(self.buf[base + 20..base + 24].try_into().ok()?);
            self.offset = base + event_len;

            if let Some((dir, name)) =
                parse_dfid_name(&self.buf[base + metadata_len..base + event_len])
            {
                return Some(DirentEvent { mask, pid, dir, name });
            }
            // An event with no directory record — `FAN_Q_OVERFLOW`, or an info type we did not
            // ask for. Skipped rather than aborting the whole read.
        }
        None
    }
}

/// Pull the parent-directory handle and entry name out of an event's info records.
fn parse_dfid_name(mut records: &[u8]) -> Option<(DirId, String)> {
    while records.len() >= INFO_HEADER_LEN {
        let info_type = records[0];
        let len = u16::from_ne_bytes(records[2..4].try_into().ok()?) as usize;
        if len < INFO_HEADER_LEN || len > records.len() {
            return None;
        }
        if info_type == INFO_TYPE_DFID_NAME {
            let body = &records[INFO_HEADER_LEN..len];
            // fsid(8) + handle_bytes(4) + handle_type(4), then the handle, then the name.
            if body.len() < 16 {
                return None;
            }
            let fsid: [u8; 8] = body[0..8].try_into().ok()?;
            let handle_bytes = u32::from_ne_bytes(body[8..12].try_into().ok()?) as usize;
            let handle_type = i32::from_ne_bytes(body[12..16].try_into().ok()?);
            if body.len() < 16 + handle_bytes {
                return None;
            }
            let handle = body[16..16 + handle_bytes].to_vec();
            let rest = &body[16 + handle_bytes..];
            let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
            let name = String::from_utf8_lossy(&rest[..end]).to_string();
            if name.is_empty() || name == "." {
                return None;
            }
            return Some((DirId(DirKey { fsid, handle_type, handle }), name));
        }
        records = &records[len..];
    }
    None
}

// ── Naming the actor ────────────────────────────────────────────────

/// `/proc/<pid>/comm`, remembered.
///
/// Reading it costs a syscall, and the same handful of processes cause almost every event.
#[derive(Default)]
struct ActorNames {
    seen: HashMap<i32, String>,
}

impl ActorNames {
    fn of(&mut self, pid: i32) -> Actor {
        if self.seen.len() > 512 {
            self.seen.clear();
        }
        let name = self
            .seen
            .entry(pid)
            .or_insert_with(|| {
                std::fs::read_to_string(format!("/proc/{pid}/comm"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            })
            .clone();
        Actor { pid, name, parent: None }
    }
}

// ── Telling a document from an editor's scratch file ────────────────

/// Whether this path is an editor's working file rather than a document.
///
/// This is a guess, and it is confined to one decision: whether a `FAN_CLOSE_WRITE` is allowed to
/// call itself a save. It never causes anything to be dropped — a scratch write is still recorded
/// as [`Kind::Wrote`] — so being wrong costs a mislabelled observation, not a blind spot.
///
/// It does not reliably catch the write half of an atomic save, and nothing relies on it to: by
/// the time the path is resolved the scratch file has usually been renamed away already, so what
/// arrives here is the document's own name. What it does catch for certain are the working files
/// an editor genuinely leaves behind — vim's persistent `.swp`, LibreOffice's `.~lock.` — which
/// are closed and never renamed, and would otherwise each be announced as somebody saving a
/// document that does not exist.
///
/// The patterns are the real ones, taken from what the editors on this desktop actually write.
fn is_scratch(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };

    // Emacs autosave: `#report.odt#`. Checked first because it is the only rule that is neither a
    // prefix nor a suffix on its own.
    if name.len() > 2 && name.starts_with('#') && name.ends_with('#') {
        return true;
    }
    // Emacs and gedit backups: `report.odt~`.
    if name.ends_with('~') {
        return true;
    }
    // LibreOffice lock files, and GIO's write-then-rename scratch: `.~lock.report.odt#`,
    // `.goutputstream-A1B2C3`.
    if name.starts_with(".~lock.") || name.starts_with(".goutputstream-") {
        return true;
    }
    // vim: `.report.odt.swp`, `.swo`, `.swx`, `.swpx`. kate: `.notes.md.kate-swp`.
    let lower = name.to_ascii_lowercase();
    const SUFFIXES: [&str; 10] = [
        ".swp", ".swo", ".swx", ".swpx", ".kate-swp", ".tmp", ".part", ".partial", ".crdownload",
        ".lock",
    ];
    SUFFIXES.iter().any(|s| lower.ends_with(s))
}

// ── Placing the marks ───────────────────────────────────────────────

/// Which group a walk is marking for, and where to record handles if it is the rename group.
enum MarkKind<'a> {
    Files,
    Renames {
        dirs: &'a mut DirMap,
        /// Directories the kernel would not name. Counted rather than shrugged at: on a
        /// filesystem with no `export_operations` — 9p, some network mounts, an overlay without
        /// a lower fh — `name_to_handle_at` returns `EOPNOTSUPP` for every directory on it, and
        /// the rename group then covers none of them. A silent partial blindness is the failure
        /// this service exists not to have.
        unnameable: &'a mut usize,
    },
}

fn init(flags: libc::c_uint) -> Result<libc::c_int, String> {
    // SAFETY: constant arguments; returns a descriptor or -1.
    let fd = unsafe {
        libc::fanotify_init(FAN_CLOEXEC | flags, (libc::O_RDONLY | libc::O_CLOEXEC) as u32)
    };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(match err.raw_os_error() {
            // Worth naming exactly: this is the capability that makes the service privileged, and
            // "operation not permitted" on its own sends people looking at file modes.
            Some(libc::EPERM) => "fanotify_init: needs CAP_SYS_ADMIN".to_string(),
            // What a kernel before 5.9 says about FAN_REPORT_DFID_NAME.
            Some(libc::EINVAL) if flags & FAN_REPORT_DFID_NAME != 0 => {
                "fanotify_init: this kernel has no FAN_REPORT_DFID_NAME (needs 5.9)".to_string()
            }
            _ => format!("fanotify_init: {err}"),
        });
    }
    Ok(fd)
}

/// How to say what the rename group can and cannot see.
///
/// Written out rather than inlined because it is the sentence a person reads when saves stop
/// looking like saves, and it has to name the cause precisely enough to act on.
fn rename_gap(covered: usize, unnameable: usize) -> String {
    if covered == 0 {
        return format!(
            "no directory could be marked for renames ({unnameable} could not be named by the \
             kernel; a filesystem with no file-handle support, such as 9p or a network mount); \
             atomic-rename saves will not be seen"
        );
    }
    format!(
        "{unnameable} of {} watched directories cannot be named by the kernel; saves made by \
         renaming into those directories will not be seen",
        covered + unnameable
    )
}

/// Mark `dir` and its subdirectories, returning how many marks were placed.
fn mark_tree(
    fd: libc::c_int,
    dir: &Path,
    scope: &Resolved,
    depth: &mut usize,
    kind: &mut MarkKind<'_>,
) -> usize {
    if *depth > MAX_DEPTH {
        return 0;
    }
    if !scope.allows(dir) && !scope.watch.iter().any(|w| w == dir) {
        return 0;
    }

    let mut placed = 0usize;
    match kind {
        MarkKind::Files => {
            if mark_one(fd, dir, FAN_CLOSE_WRITE | FAN_OPEN_EXEC | FAN_EVENT_ON_CHILD).is_ok() {
                placed = 1;
            }
        }
        MarkKind::Renames { dirs, unnameable } => {
            // Directory-entry events on a directory mark already report its entries, so
            // FAN_EVENT_ON_CHILD is neither needed nor meaningful here.
            //
            // The handle is taken before the mark rather than after: a directory we cannot name in
            // the form events will arrive in would produce events we could only count.
            match dir_key(dir) {
                Some(key) => {
                    if mark_one(fd, dir, FAN_MOVED_TO).is_ok() {
                        dirs.insert(key, dir.to_path_buf());
                        placed = 1;
                    }
                }
                None => **unnameable += 1,
            }
        }
    }

    let Ok(entries) = std::fs::read_dir(dir) else { return placed };
    for entry in entries.flatten() {
        if placed >= MAX_MARKS {
            tracing::warn!(limit = MAX_MARKS, "fanotify mark limit reached; deeper paths unwatched");
            break;
        }
        let path = entry.path();
        // `is_dir` follows symlinks; a link out of the scope would silently widen it, and a link
        // back into it would mark the same tree twice.
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        *depth += 1;
        placed += mark_tree(fd, &path, scope, depth, kind);
        *depth -= 1;
    }
    placed
}

fn mark_one(fd: libc::c_int, dir: &Path, mask: u64) -> Result<(), ()> {
    let Ok(c_dir) = CString::new(dir.as_os_str().as_bytes()) else { return Err(()) };
    // SAFETY: `c_dir` is NUL-terminated and outlives the call; AT_FDCWD with an absolute path.
    let rc = unsafe {
        libc::fanotify_mark(fd, FAN_MARK_ADD | FAN_MARK_ONLYDIR, mask, libc::AT_FDCWD, c_dir.as_ptr())
    };
    if rc < 0 {
        Err(())
    } else {
        Ok(())
    }
}

/// `libc::fsid_t` is two 32-bit words, and the kernel writes the same eight bytes into an event.
/// If that ever stops being true the copy below would read the wrong thing, so it is a build
/// failure rather than a runtime surprise.
const _: () = assert!(std::mem::size_of::<libc::fsid_t>() == 8);

/// The kernel's own name for a directory: filesystem id plus an opaque handle.
///
/// Wants `CAP_DAC_READ_SEARCH` on some filesystems, which is why this runs at startup and never
/// again.
fn dir_key(dir: &Path) -> Option<DirId> {
    let c_dir = CString::new(dir.as_os_str().as_bytes()).ok()?;

    // SAFETY: `statfs` writes a POD struct; `c_dir` is NUL-terminated and outlives the call.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c_dir.as_ptr(), &mut stat) } < 0 {
        return None;
    }
    let mut fsid = [0u8; 8];
    // SAFETY: `f_fsid` is eight bytes of POD (asserted above). Its fields are private in `libc`,
    // and the kernel compares these bytes rather than any interpretation of them.
    unsafe {
        std::ptr::copy_nonoverlapping(
            std::ptr::addr_of!(stat.f_fsid) as *const u8,
            fsid.as_mut_ptr(),
            8,
        );
    }

    #[repr(C)]
    struct HandleBuf {
        handle_bytes: libc::c_uint,
        handle_type: libc::c_int,
        f_handle: [u8; MAX_HANDLE_SZ],
    }
    let mut handle = HandleBuf {
        handle_bytes: MAX_HANDLE_SZ as libc::c_uint,
        handle_type: 0,
        f_handle: [0; MAX_HANDLE_SZ],
    };
    let mut mount_id: libc::c_int = 0;

    // SAFETY: `handle` is laid out as `struct file_handle` with its capacity declared in
    // `handle_bytes`; the kernel writes at most that many bytes into `f_handle`.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_name_to_handle_at,
            libc::AT_FDCWD,
            c_dir.as_ptr(),
            std::ptr::addr_of_mut!(handle) as *mut libc::file_handle,
            std::ptr::addr_of_mut!(mount_id),
            0,
        )
    };
    if rc < 0 {
        return None;
    }
    let len = (handle.handle_bytes as usize).min(MAX_HANDLE_SZ);
    Some(DirId(DirKey {
        fsid,
        handle_type: handle.handle_type,
        handle: handle.f_handle[..len].to_vec(),
    }))
}

/// The path behind a descriptor the kernel handed us.
fn resolve(fd: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{fd}")).ok().filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metadata_layout_matches_the_kernels() {
        // event_len(4) + vers(1) + reserved(1) + metadata_len(2) + mask(8) + fd(4) + pid(4).
        assert_eq!(METADATA_LEN, 24);
        // info_type(1) + pad(1) + len(2).
        assert_eq!(INFO_HEADER_LEN, 4);
    }

    #[test]
    fn we_ask_for_saves_and_execs_and_nothing_else() {
        let mask = FAN_CLOSE_WRITE | FAN_OPEN_EXEC | FAN_EVENT_ON_CHILD;
        // FAN_MODIFY would fire on every write and bury everything else in the ring.
        const FAN_MODIFY: u64 = 0x0000_0002;
        const FAN_OPEN: u64 = 0x0000_0020;
        assert_eq!(mask & FAN_MODIFY, 0, "watching writes would be a flood, not a signal");
        assert_eq!(mask & FAN_OPEN, 0, "every open of every file is not perception");
    }

    #[test]
    fn a_descriptor_that_resolves_to_nothing_is_dropped() {
        // Descriptor 9999 is not open in the test process.
        assert!(resolve(9999).is_none());
    }

    #[test]
    fn an_editors_working_file_is_not_a_document() {
        // Every one of these is what an editor on this desktop actually writes on the way to
        // saving. Reporting any of them as "saved" is the bug this file was rewritten for.
        for scratch in [
            "/home/p/.report.odt.swp",
            "/home/p/.report.odt.swpx",
            "/home/p/.notes.md.kate-swp",
            "/home/p/report.odt~",
            "/home/p/#report.odt#",
            "/home/p/.~lock.report.odt#",
            "/home/p/.goutputstream-A1B2C3",
            "/home/p/lu84h2j.tmp",
            "/home/p/big.iso.part",
            "/home/p/song.mp3.crdownload",
            "/home/p/repo/.git/index.lock",
        ] {
            assert!(is_scratch(Path::new(scratch)), "{scratch} should not be called a save");
        }
    }

    #[test]
    fn a_real_document_is_not_mistaken_for_scratch() {
        // The other half: over-matching here would turn in-place saves into `Wrote` and lose the
        // one case the old code got right.
        for document in [
            "/home/p/report.odt",
            "/home/p/notes.md",
            "/home/p/src/main.rs",
            "/home/p/Cargo.toml",
            "/home/p/swap-meet.txt",
            "/home/p/tmp-notes.md",
            "/home/p/#hashtag.md",
        ] {
            assert!(!is_scratch(Path::new(document)), "{document} is a document");
        }
    }

    /// Lay out one `FAN_REPORT_DFID_NAME` event the way the kernel does.
    fn dirent_event(mask: u64, pid: i32, fsid: [u8; 8], handle: &[u8], name: &str) -> Vec<u8> {
        let mut info = Vec::new();
        info.extend_from_slice(&fsid);
        info.extend_from_slice(&(handle.len() as u32).to_ne_bytes());
        info.extend_from_slice(&1i32.to_ne_bytes()); // handle_type
        info.extend_from_slice(handle);
        info.extend_from_slice(name.as_bytes());
        info.push(0);
        // The kernel pads records out to 8 bytes; parsing must survive the padding.
        while (INFO_HEADER_LEN + info.len()) % 8 != 0 {
            info.push(0);
        }

        let record_len = INFO_HEADER_LEN + info.len();
        let event_len = METADATA_LEN + record_len;

        let mut out = Vec::new();
        out.extend_from_slice(&(event_len as u32).to_ne_bytes());
        out.push(3); // vers
        out.push(0); // reserved
        out.extend_from_slice(&(METADATA_LEN as u16).to_ne_bytes());
        out.extend_from_slice(&mask.to_ne_bytes());
        out.extend_from_slice(&(-1i32).to_ne_bytes()); // FAN_NOFD
        out.extend_from_slice(&pid.to_ne_bytes());
        out.push(INFO_TYPE_DFID_NAME);
        out.push(0); // pad
        out.extend_from_slice(&(record_len as u16).to_ne_bytes());
        out.extend_from_slice(&info);
        out
    }

    #[test]
    fn a_rename_event_yields_a_directory_and_a_name() {
        let bytes =
            dirent_event(FAN_MOVED_TO, 4242, [7; 8], &[1, 2, 3, 4, 5, 6, 7, 8], "report.odt");
        let events: Vec<_> = DirentEvents::new(&bytes).collect();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "report.odt");
        assert_eq!(events[0].pid, 4242);
        assert_eq!(events[0].mask & FAN_MOVED_TO, FAN_MOVED_TO);
        assert_eq!(events[0].dir.0.fsid, [7; 8]);
        assert_eq!(events[0].dir.0.handle, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn two_events_in_one_read_are_both_seen() {
        // A save storm arrives in a single `read`; stopping after the first would lose the rest.
        let mut bytes = dirent_event(FAN_MOVED_TO, 1, [1; 8], &[9; 12], "a.txt");
        bytes.extend(dirent_event(FAN_MOVED_TO, 2, [1; 8], &[9; 12], "b.txt"));

        let names: Vec<String> = DirentEvents::new(&bytes).map(|e| e.name).collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn the_same_directory_produces_the_same_key() {
        // What makes the map work: the handle the kernel puts in an event must hash equal to the
        // one taken at mark time. Bytes in, bytes out — no path comparison anywhere.
        let bytes = dirent_event(FAN_MOVED_TO, 1, [5; 8], &[3; 16], "x");
        let from_event = DirentEvents::new(&bytes).next().unwrap().dir;

        let mut map = DirMap::new();
        map.insert(
            DirId(DirKey { fsid: [5; 8], handle_type: 1, handle: vec![3; 16] }),
            PathBuf::from("/home/p/docs"),
        );
        assert_eq!(map.get(&from_event), Some(&PathBuf::from("/home/p/docs")));
    }

    #[test]
    fn a_truncated_event_stops_the_walk_rather_than_reading_past_it() {
        let full = dirent_event(FAN_MOVED_TO, 1, [1; 8], &[2; 8], "cut.txt");
        for cut in [4, 12, METADATA_LEN, full.len() - 1] {
            let events: Vec<_> = DirentEvents::new(&full[..cut]).collect();
            assert!(events.is_empty(), "a {cut}-byte buffer should yield nothing, not garbage");
        }
    }

    #[test]
    fn an_event_carrying_no_directory_record_is_skipped_not_fatal() {
        // FAN_Q_OVERFLOW arrives with no info records at all.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(METADATA_LEN as u32).to_ne_bytes());
        bytes.push(3);
        bytes.push(0);
        bytes.extend_from_slice(&(METADATA_LEN as u16).to_ne_bytes());
        bytes.extend_from_slice(&0u64.to_ne_bytes());
        bytes.extend_from_slice(&(-1i32).to_ne_bytes());
        bytes.extend_from_slice(&0i32.to_ne_bytes());
        // …followed by a real one, which must still be found.
        bytes.extend(dirent_event(FAN_MOVED_TO, 9, [1; 8], &[4; 8], "after.txt"));

        let names: Vec<String> = DirentEvents::new(&bytes).map(|e| e.name).collect();
        assert_eq!(names, vec!["after.txt"]);
    }

    #[test]
    fn a_directory_on_an_ordinary_filesystem_can_be_named() {
        // Not a layout test: it proves `name_to_handle_at` actually works, which is the
        // assumption the whole rename group rests on. Run against the temp directory rather than
        // the source tree, because the source tree is often a 9p share during development and 9p
        // has no file handles at all — see the sibling test.
        let dir = std::env::temp_dir();
        let key = dir_key(&dir);
        assert!(key.is_some(), "name_to_handle_at failed on {dir:?}; renames would be unresolvable");
        assert_eq!(key.clone(), dir_key(&dir), "the same directory must key the same every time");
        assert!(!key.unwrap().0.handle.is_empty());
    }

    #[test]
    fn a_filesystem_without_handles_is_reported_rather_than_half_watched() {
        // The failure mode found while writing this: on 9p, `name_to_handle_at` returns
        // EOPNOTSUPP for every directory, the rename group covers nothing, and without this the
        // service would look identical to a desktop where nobody saves anything.
        let total_loss = rename_gap(0, 12);
        assert!(total_loss.contains("atomic-rename saves will not be seen"));
        assert!(total_loss.contains("12"));

        let partial = rename_gap(30, 4);
        assert!(partial.contains("4 of 34"), "a partial gap must state its size: {partial}");
        assert!(!partial.contains("no directory"), "half-blind is not blind: {partial}");
    }
}
