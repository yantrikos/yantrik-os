//! A web server on the loopback for the tests to download from: files by path, `Range` honoured
//! as a real server does (`206` with `Content-Range`), and an optional hold part way through a
//! file, so a test can close the app while a download is genuinely half written.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How a file is served.
#[derive(Clone)]
pub struct File {
    pub body: Vec<u8>,
    /// Send this many bytes, then wait for [`Server::release`] before the rest.
    pub hold_at: Option<usize>,
    /// Answer `302` to this `Location` instead of serving a body.
    pub redirect: Option<String>,
}

impl File {
    pub fn plain(body: Vec<u8>) -> File {
        File { body, hold_at: None, redirect: None }
    }

    pub fn redirect(to: &str) -> File {
        File { body: Vec::new(), hold_at: None, redirect: Some(to.to_string()) }
    }

    pub fn held_at(body: Vec<u8>, at: usize) -> File {
        File { body, hold_at: Some(at), redirect: None }
    }
}

type Gate = Arc<(Mutex<bool>, Condvar)>;

pub struct Server {
    pub port: u16,
    gate: Gate,
    stop: Arc<AtomicBool>,
    /// The `Range` header of each request, `None` where there was none, in the order they came.
    pub ranges: Arc<Mutex<Vec<Option<String>>>>,
}

impl Server {
    /// A server an agent's fetch may reach: its port is listed with the engine's test seam
    /// (`engine/fetch.rs`), which lets nothing else on the loopback through.
    pub fn start(files: Vec<(&str, File)>) -> Server {
        let server = Server::start_unlisted(files);
        crate::engine::listed::allow(server.port);
        server
    }

    /// A server on the loopback that an agent's fetch may not reach: the machine's own services.
    pub fn start_unlisted(files: Vec<(&str, File)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let files: Arc<HashMap<String, File>> = Arc::new(files.into_iter().map(|(p, f)| (p.to_string(), f)).collect());
        let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let (g, s, r) = (gate.clone(), stop.clone(), ranges.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (files, gate, ranges) = (files.clone(), g.clone(), r.clone());
                        std::thread::spawn(move || serve(stream, &files, &gate, &ranges));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Server { port, gate, stop, ranges }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// [`Server::release`], for a thread of its own to call: the test's thread is the UI thread,
    /// and it may be waiting inside a job for the file this lets go.
    pub fn releaser(&self) -> impl Fn() + Send + 'static {
        let gate = self.gate.clone();
        move || {
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
        }
    }

    /// Let every held file go on, now and for every later request.
    pub fn release(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.release();
    }
}

fn serve(mut stream: TcpStream, files: &HashMap<String, File>, gate: &Gate, ranges: &Mutex<Vec<Option<String>>>) {
    let _ = stream.set_nonblocking(false);
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    let range = head
        .lines()
        .find_map(|l| {
            let l = l.to_ascii_lowercase();
            l.strip_prefix("range: bytes=").map(|r| r.trim_end_matches('-').to_string())
        });
    ranges.lock().unwrap().push(range.clone());
    let Some(file) = files.get(&path) else {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    if let Some(to) = &file.redirect {
        let _ = stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
        return;
    }
    let start = range.as_deref().and_then(|r| r.parse::<usize>().ok()).unwrap_or(0).min(file.body.len());
    let (status, extra) = if range.is_some() {
        ("206 Partial Content", format!("Content-Range: bytes {start}-{}/{}\r\n", file.body.len() - 1, file.body.len()))
    } else {
        ("200 OK", String::new())
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        file.body.len() - start
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }
    let mut at = start;
    if let Some(hold) = file.hold_at.filter(|h| *h > start) {
        // In small pieces like the rest: a test that waits for the status word alone can pause
        // before the worker has written anything, if one write hands it the whole held part.
        for piece in file.body[at..hold].chunks(4096) {
            if stream.write_all(piece).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        at = hold;
        let (open, turned) = (&gate.0, &gate.1);
        let mut released = open.lock().unwrap();
        let until = Instant::now() + Duration::from_secs(60);
        while !*released && Instant::now() < until {
            released = turned.wait_timeout(released, Duration::from_millis(50)).unwrap().0;
        }
        drop(released);
        // After the hold, in small pieces: a reader that is told to stop meanwhile has not been
        // handed the whole rest in one read.
        for piece in file.body[at..].chunks(4096) {
            if stream.write_all(piece).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        return;
    }
    let _ = stream.write_all(&file.body[at..]);
}
