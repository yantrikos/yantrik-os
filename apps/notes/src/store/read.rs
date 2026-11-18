//! The store's one way to read a file: no link followed, no FIFO waited on, nothing over a limit.

use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

/// The file's text, `None` when it is absent. A link, a FIFO, a device, a file over `limit` or
/// one that grows past it while it is read is refused with a sentence.
pub fn read(path: &Path, limit: usize) -> Result<Option<String>, String> {
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        // O_NONBLOCK so a FIFO named like a note is refused below instead of hanging the job.
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() || m.len() > limit as u64 {
        return Err("Not a regular text file, or file exceeds the size limit.".into());
    }
    let mut s = String::new();
    (&mut file)
        .take(limit as u64 + 1)
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    if s.len() > limit {
        return Err("File grew beyond its size limit.".into());
    }
    Ok(Some(s))
}
