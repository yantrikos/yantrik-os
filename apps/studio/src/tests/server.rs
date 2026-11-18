//! A ComfyUI server, and an OpenAI-compatible images endpoint, in this process.
//!
//! One server for both because both are HTTP and the question the second one answers (whether an
//! API key can be used without ever being repeated) needs a service that receives it. Real
//! sockets and real HTTP, to the same `ureq` client the app ships: a hand-written trait object
//! standing in for `Backend` would test this app's idea of the protocol; this tests the protocol.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::backend;

/// What the fake server saw. Everything a test asserts about the protocol is recorded here rather
/// than inferred from what came back.
#[derive(Default)]
pub(crate) struct Recorded {
    pub(crate) next: usize,
    /// prompt id → the graph that was posted with it.
    pub(crate) graphs: HashMap<String, Value>,
    /// prompt id → how many times `/history` has been asked about it.
    pub(crate) polls: HashMap<String, usize>,
    /// "subfolder/filename" → prompt id, so `/view` can render the graph it was asked for.
    pub(crate) views: HashMap<String, String>,
    pub(crate) viewed: Vec<String>,
    pub(crate) prompts: Vec<Value>,
    pub(crate) interrupts: usize,
    pub(crate) authorizations: Vec<String>,
    pub(crate) image_requests: Vec<Value>,
}

/// How the fake server misbehaves, for the tests that are about failure.
///
/// `pub(crate)` because `engine`'s own tests drive a ComfyUI generation through it too, and a
/// second fake server answering the same routes a second way would be two things to keep in step.
#[derive(Clone, Default)]
pub(crate) struct Manners {
    /// Answer `/prompt` with this status and body instead of taking the graph.
    pub(crate) refuse: Option<(u16, String)>,
    /// Never report a render as finished, so the only way out is the cancel flag.
    pub(crate) never_finish: bool,
    /// Answer an image request with this URL to fetch, as some hosted services do.
    pub(crate) offer_url: Option<String>,
    /// Answer an image request with these bytes inline, whatever they are.
    pub(crate) inline_bytes: Option<Vec<u8>>,
}

/// A ComfyUI server, and an OpenAI-compatible images endpoint, in this process.
///
/// One server for both because both are HTTP and the question the second one answers — whether an
/// API key can be used without ever being repeated — needs a service that receives it.
///
/// Real sockets and real HTTP, to the same `ureq` client the app ships. A hand-written trait object
/// standing in for `Backend` would test this app's idea of the protocol; this tests the protocol.
pub(crate) struct Server {
    pub(crate) base_url: String,
    shared: Arc<Mutex<Recorded>>,
    stop: Arc<AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(manners: Manners) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        // Non-blocking so the accept loop can notice it is being shut down. A test that leaked a
        // thread blocked in `accept` would leak a socket with it, and the port would stay taken for
        // the rest of the run.
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Mutex::new(Recorded::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let accept = {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let shared = shared.clone();
                            let manners = manners.clone();
                            // One thread per connection rather than a pool: these are tests, and a
                            // connection that is served inline would deadlock the moment a handler
                            // waited on the client that is waiting on it.
                            std::thread::spawn(move || serve(stream, &shared, &manners));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(_) => return,
                    }
                }
            })
        };
        Server { base_url: format!("http://127.0.0.1:{port}"), shared, stop, accept: Some(accept) }
    }

    pub(crate) fn config(&self, kind: &str) -> String {
        format!(r#"{{"backend":{{"kind":"{kind}","base_url":"{}"}}}}"#, self.base_url)
    }

    pub(crate) fn with<T>(&self, read: impl FnOnce(&Recorded) -> T) -> T {
        read(&self.shared.lock().unwrap())
    }

    pub(crate) fn last_graph(&self) -> Value {
        self.with(|recorded| {
            recorded
                .graphs
                .iter()
                // Job ids are "prompt-1", "prompt-2", … so (length, text) is numeric order, and a
                // HashMap's iteration order is not something to build an expectation on.
                .max_by_key(|(id, _)| (id.len(), (*id).clone()))
                .map(|(_, graph)| graph.clone())
                .expect("no graph was posted to the server")
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

/// Serve one connection until the client closes it.
///
/// Every reply says `Connection: close`, and the backend builds a fresh `ureq` agent per call, so in
/// practice this is one request per connection. The loop is here because "in practice" is not a
/// contract, and a server that hung up on a client that reused a connection would look exactly like
/// a bug in the app.
fn serve(mut stream: TcpStream, shared: &Arc<Mutex<Recorded>>, manners: &Manners) {
    let peer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(peer);
    loop {
        let mut request_line = String::new();
        match reader.read_line(&mut request_line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(_) => return,
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let target = parts.next().unwrap_or_default().to_string();
        let mut length = 0usize;
        let mut authorization = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => return,
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
                if name.eq_ignore_ascii_case("authorization") {
                    // Recorded verbatim, and asserted on by exactly one test: this is the string
                    // that must never appear anywhere the app writes or says.
                    authorization = value.trim().to_string();
                }
            }
        }
        let mut body = vec![0u8; length];
        if length > 0 && reader.read_exact(&mut body).is_err() {
            return;
        }
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let (status, kind, payload) = route(&method, &target, body, authorization, shared, manners);
        let reason = match status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Status",
        };
        let mut head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .into_bytes();
        head.extend_from_slice(&payload);
        if stream.write_all(&head).is_err() || stream.flush().is_err() {
            return;
        }
    }
}

fn route(
    method: &str,
    target: &str,
    body: Value,
    authorization: String,
    shared: &Arc<Mutex<Recorded>>,
    manners: &Manners,
) -> (u16, &'static str, Vec<u8>) {
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query.to_string()),
        None => (target, String::new()),
    };
    let json = |status: u16, value: Value| (status, "application/json", value.to_string().into_bytes());

    match (method, path) {
        ("POST", "/prompt") => {
            if let Some((status, text)) = &manners.refuse {
                return (*status, "application/json", text.as_bytes().to_vec());
            }
            let Some(graph) = body.get("prompt") else {
                return json(400, json!({ "error": { "message": "no graph was posted" } }));
            };
            let mut recorded = shared.lock().unwrap();
            recorded.next += 1;
            let id = format!("prompt-{}", recorded.next);
            recorded.graphs.insert(id.clone(), graph.clone());
            recorded.prompts.push(body.clone());
            drop(recorded);
            json(200, json!({ "prompt_id": id, "number": 1, "node_errors": [] }))
        }
        ("GET", path) if path.starts_with("/history/") => {
            let id = path.trim_start_matches("/history/").to_string();
            let mut recorded = shared.lock().unwrap();
            let polls = recorded.polls.entry(id.clone()).or_insert(0);
            *polls += 1;
            let first = *polls == 1;
            // The entry appears as soon as the graph is queued and grows its `outputs` when the
            // render finishes, so the first poll answers with an entry that has none. That is the
            // "still working" branch, and a server that answered with the picture straight away
            // would never exercise it.
            if first && !manners.never_finish {
                let graph = recorded.graphs.get(&id).cloned().unwrap_or(Value::Null);
                drop(recorded);
                let (width, height) = latent_size(&graph);
                let key = format!("Studio/{}/Studio_{:05}_.png", today(), width * height % 1000);
                shared.lock().unwrap().views.insert(key.clone(), id.clone());
                return json(
                    200,
                    json!({ id: { "status": { "status_str": "running", "completed": false } } }),
                );
            }
            if manners.never_finish {
                drop(recorded);
                return json(200, json!({ id: { "status": { "status_str": "running", "completed": false } } }));
            }
            let key = recorded
                .views
                .iter()
                .find(|(_, owner)| *owner == &id)
                .map(|(key, _)| key.clone())
                .unwrap_or_else(|| format!("Studio/{}/Studio_00001_.png", today()));
            drop(recorded);
            json(
                200,
                json!({
                    id: {
                        "status": { "status_str": "success", "completed": true },
                        "outputs": {
                            "9": {
                                "images": [{
                                    "filename": key.rsplit('/').next().unwrap_or_default(),
                                    "subfolder": key.rsplit_once('/').map(|(head, _)| head).unwrap_or_default(),
                                    "type": "output",
                                }]
                            }
                        }
                    }
                }),
            )
        }
        ("GET", "/offered") => {
            shared.lock().unwrap().viewed.push(target.to_string());
            (200, "image/png", backend::draw("offered", 1, 16, 16))
        }
        ("GET", "/view") => {
            let mut filename = String::new();
            let mut subfolder = String::new();
            for pair in query.split('&') {
                if let Some((name, value)) = pair.split_once('=') {
                    match name {
                        "filename" => filename = unescape(value),
                        "subfolder" => subfolder = unescape(value),
                        _ => {}
                    }
                }
            }
            let mut recorded = shared.lock().unwrap();
            recorded.viewed.push(target.to_string());
            let key = format!("{subfolder}/{filename}");
            let Some(graph) = recorded.views.get(&key).and_then(|id| recorded.graphs.get(id)).cloned() else {
                drop(recorded);
                return json(404, json!({ "error": { "message": format!("no file called {key}") } }));
            };
            drop(recorded);
            // Rendered from the graph that was actually posted: the prompt is taken by following the
            // sampler's own `positive` link, exactly the link `fill` wrote the prompt through. A
            // graph that wired the negative prompt into the positive input would draw a different
            // picture, and the test that compares these bytes to the ones the backend drew would
            // fail — which is the point of rendering rather than returning a fixed image.
            let (prompt, seed) = ask_of(&graph);
            let (width, height) = latent_size(&graph);
            (200, "image/png", backend::draw(&prompt, seed, width.max(8), height.max(8)))
        }
        ("POST", "/interrupt") => {
            shared.lock().unwrap().interrupts += 1;
            json(200, json!({}))
        }
        ("GET", "/system_stats") => json(
            200,
            json!({ "system": { "comfyui_version": "0.3.0-fake" }, "devices": [] }),
        ),
        ("POST", "/images/generations") => {
            let mut recorded = shared.lock().unwrap();
            recorded.authorizations.push(authorization.clone());
            recorded.image_requests.push(body.clone());
            drop(recorded);
            let prompt = body["prompt"].as_str().unwrap_or_default().to_string();
            let size = body["size"].as_str().unwrap_or("1024x1024").to_string();
            let (width, height) = size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
                .unwrap_or((1024, 1024));
            use base64::Engine as _;
            if let Some(offered) = &manners.offer_url {
                return json(200, json!({ "data": [{ "url": offered }] }));
            }
            if let Some(bytes) = &manners.inline_bytes {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                return json(200, json!({ "data": [{ "b64_json": encoded }] }));
            }
            let encoded = base64::engine::general_purpose::STANDARD.encode(backend::draw(&prompt, 1, width, height));
            json(200, json!({ "created": 1_780_000_000u64, "data": [{ "b64_json": encoded }] }))
        }
        _ => json(404, json!({ "error": { "message": format!("the fake server has no {method} {path}") } })),
    }
}

pub(crate) fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Undo the percent-encoding this app writes by hand, so the query it built can be read back.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or_default();
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub(crate) fn node_of<'a>(graph: &'a Value, class: &str) -> Option<&'a Value> {
    graph
        .as_object()?
        .values()
        .find(|node| node["class_type"].as_str() == Some(class))
}

/// The text a link points at. A link in the API format is `["6", 0]`; a literal is a string.
pub(crate) fn text_of_link(graph: &Value, link: &Value) -> String {
    if let Some(text) = link.as_str() {
        return text.to_string();
    }
    let id = link.as_array().and_then(|pair| pair.first()).and_then(|id| id.as_str()).unwrap_or_default();
    graph[id]["inputs"]["text"].as_str().unwrap_or_default().to_string()
}

/// What a posted graph asks for, read the way a server would read it.
pub(crate) fn ask_of(graph: &Value) -> (String, u64) {
    let sampler = node_of(graph, "KSampler").or_else(|| node_of(graph, "KSamplerAdvanced"));
    let prompt = sampler
        .and_then(|node| node["inputs"].get("positive"))
        .map(|link| text_of_link(graph, link))
        .unwrap_or_default();
    let seed = sampler
        .and_then(|node| node["inputs"].get("seed").or_else(|| node["inputs"].get("noise_seed")))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    (prompt, seed)
}

pub(crate) fn latent_size(graph: &Value) -> (u32, u32) {
    let latent = node_of(graph, "EmptyLatentImage");
    let read = |key: &str| {
        latent
            .and_then(|node| node["inputs"].get(key))
            .and_then(|value| value.as_u64())
            .unwrap_or(64) as u32
    };
    (read("width"), read("height"))
}

