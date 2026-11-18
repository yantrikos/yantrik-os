//! The reader kept running between looks: `yantrik-ocr --serve`.
//!
//! Started for each look, the reader loads its models, reads everything and forgets it all. Kept
//! running, it loads them once and remembers what each line said (`memory`), so a look at a
//! screen that changed a little costs a little.
//!
//! One request per line on standard input, one answer per line on standard output:
//!
//! ```text
//! {"id": 7, "image": "/run/user/1000/yantrik-read-1.ppm", "space": "desktop", "max_lines": 300}
//! → {"id":7,"width":1280,"height":800,"seconds":0.4,"lines":[…],"clipped":false,"cost":{…}}
//! {"id": 8, "forget": true}
//! → {"id":8,"forgot":true}
//! ```
//!
//! A request that cannot be answered gets `{"error": "…"}` and the reader carries on. A request's
//! `id`, when it has one, comes back in its answer, so the asker can tell an answer from a line
//! that is not one (anything else the process might print) and from a late one.
//!
//! # It does not outstay its welcome
//!
//! The reader holds a few hundred MB and what the last looks said. It ends when its input
//! closes (the shell went away, or wants it gone), and by itself after [`IDLE`] without a look.
//! `forget` drops what it remembers without ending it. It never reads an image it was not sent.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::memory::Memory;
use crate::read::Reader;
use crate::reading;

/// How long the reader stays without being asked anything.
pub const IDLE: Duration = Duration::from_secs(120);

/// How often the idle timer looks at the clock.
const TICK: Duration = Duration::from_millis(250);

/// The display a look is of when the request does not say: its memory is shared with nothing
/// that names one.
const UNNAMED: &str = "";

/// One line of input, understood.
#[derive(Debug, PartialEq)]
pub enum Request {
    Look { image: PathBuf, space: String, max_lines: usize },
    /// Drop everything remembered, without ending the reader. The shell ends the process
    /// instead; this is for tools and the bench, which want a cold memory and warm models.
    Forget,
}

/// A line of input: the number the asker gave it, if any, and what it asks. Answers carry the
/// same `id`, so an answer is never taken for the answer to another request.
#[derive(Debug, PartialEq)]
pub struct Asked {
    pub id: Option<u64>,
    pub request: Result<Request, String>,
}

/// Understand one line of input.
pub fn asked(line: &str) -> Asked {
    let json: serde_json::Value = match serde_json::from_str(line) {
        Ok(json) => json,
        Err(e) => return Asked { id: None, request: Err(format!("not a request: {e}")) },
    };
    Asked { id: json["id"].as_u64(), request: understand(&json) }
}

fn understand(json: &serde_json::Value) -> Result<Request, String> {
    if json["forget"] == true {
        return Ok(Request::Forget);
    }
    let image = json["image"].as_str().filter(|path| !path.is_empty()).ok_or("a request names an `image` to read, or says `forget`")?;
    Ok(Request::Look {
        image: PathBuf::from(image),
        space: json["space"].as_str().unwrap_or(UNNAMED).to_string(),
        max_lines: json["max_lines"].as_u64().map_or(usize::MAX, |n| n as usize),
    })
}

fn answer(reader: &Reader, memory: &mut Memory, request: Request) -> Result<serde_json::Value, String> {
    match request {
        Request::Forget => {
            memory.forget();
            Ok(serde_json::json!({ "forgot": true }))
        }
        Request::Look { image, space, max_lines } => {
            let rgb = image::open(&image).map_err(|e| format!("cannot read {}: {e}", image.display()))?.into_rgb8();
            let (width, height) = rgb.dimensions();
            reading::look(reader, memory, &space, rgb.as_raw(), width, height).map(|reading| reading.to_json(max_lines))
        }
    }
}

/// Answer requests until the input closes or nothing has been asked for `idle`.
pub fn serve(reader: Reader, idle: Duration) {
    let mut memory = Memory::default();
    run(std::io::stdin().lock(), std::io::stdout().lock(), idle, || std::process::exit(0), |request| answer(&reader, &mut memory, request));
}

/// The loop of [`serve`], with the input, the output, the answering and the leaving given.
fn run(input: impl BufRead, mut output: impl Write, idle: Duration, leave: impl Fn() + Send + 'static, mut answer: impl FnMut(Request) -> Result<serde_json::Value, String>) {
    let last_asked = Arc::new(Mutex::new(Instant::now()));
    let busy = Arc::new(AtomicBool::new(false));
    {
        let (last_asked, busy) = (Arc::clone(&last_asked), Arc::clone(&busy));
        // The main thread is blocked in a read of its input, which has no time limit of its
        // own. This thread is the limit.
        let spawned = std::thread::Builder::new().name("idle".into()).spawn(move || loop {
            std::thread::sleep(TICK);
            let quiet_for = last_asked.lock().map_or(Duration::ZERO, |at| at.elapsed());
            if !busy.load(Ordering::Acquire) && quiet_for >= idle {
                leave();
                return;
            }
        });
        if let Err(e) = spawned {
            // Without the limit the reader would stay for as long as its input stays open.
            eprintln!("yantrik-ocr: no idle timer ({e}); not staying");
            return;
        }
    }

    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        busy.store(true, Ordering::Release);
        let Asked { id, request } = asked(&line);
        let mut reply = request.and_then(&mut answer).unwrap_or_else(|why| serde_json::json!({ "error": why }));
        if let (Some(id), Some(fields)) = (id, reply.as_object_mut()) {
            fields.insert("id".into(), id.into());
        }
        if let Ok(mut at) = last_asked.lock() {
            *at = Instant::now();
        }
        busy.store(false, Ordering::Release);
        if writeln!(output, "{reply}").and_then(|()| output.flush()).is_err() {
            // Nobody is reading the answers any more.
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc;

    fn request(line: &str) -> Result<Request, String> {
        asked(line).request
    }

    #[test]
    fn a_look_names_its_image_and_may_name_its_display_and_a_limit() {
        let look = request(r#"{"image": "/run/user/1000/yantrik-read-1.ppm", "space": "desktop", "max_lines": 300}"#);
        assert_eq!(look, Ok(Request::Look { image: "/run/user/1000/yantrik-read-1.ppm".into(), space: "desktop".into(), max_lines: 300 }));
        let bare = request(r#"{"image": "/tmp/x.png"}"#);
        assert_eq!(bare, Ok(Request::Look { image: "/tmp/x.png".into(), space: String::new(), max_lines: usize::MAX }));
    }

    #[test]
    fn forgetting_is_asked_for_by_name() {
        assert_eq!(request(r#"{"forget": true}"#), Ok(Request::Forget));
        assert!(request(r#"{"forget": false}"#).is_err(), "not forgetting, and no image: nothing to do");
    }

    #[test]
    fn a_line_that_is_not_a_request_is_refused_not_guessed_at() {
        for line in ["", "read the screen", "{}", r#"{"image": ""}"#, r#"{"image": 7}"#] {
            assert!(request(line).is_err(), "{line:?}");
        }
    }

    #[test]
    fn a_request_may_carry_an_id_and_it_is_not_part_of_what_is_asked() {
        assert_eq!(asked(r#"{"id": 12, "forget": true}"#), Asked { id: Some(12), request: Ok(Request::Forget) });
        assert_eq!(asked(r#"{"id": 12}"#).id, Some(12), "a request that cannot be answered still says whose it was");
        assert_eq!(asked(r#"{"forget": true}"#).id, None);
        assert_eq!(asked(r#"{"id": "twelve", "forget": true}"#).id, None);
    }

    fn replies(input: &str) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        let answer = |request| match request {
            Request::Forget => Ok(serde_json::json!({ "forgot": true })),
            Request::Look { space, .. } => Err(format!("no models in a test, for {space}")),
        };
        run(Cursor::new(input.to_string()), &mut out, Duration::from_secs(3600), || {}, answer);
        String::from_utf8_lossy(&out).lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
    }

    #[test]
    fn every_answer_carries_the_id_of_its_request() {
        let got = replies("{\"id\": 1, \"forget\": true}\n\n{\"id\": 2, \"image\": \"/x\", \"space\": \"s\"}\nnot json\n{\"forget\": true}\n");
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], serde_json::json!({ "id": 1, "forgot": true }));
        assert_eq!(got[1]["id"], 2, "an error is the answer to its request too");
        assert!(got[1]["error"].is_string());
        assert!(got[2]["id"].is_null() && got[2]["error"].is_string());
        assert_eq!(got[3], serde_json::json!({ "forgot": true }), "no id asked, none given");
    }

    /// A reader's input that stays open until its sender is dropped.
    struct Open(mpsc::Receiver<u8>);

    impl std::io::Read for Open {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match (buf.first_mut(), self.0.recv()) {
                (Some(slot), Ok(byte)) => {
                    *slot = byte;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }
    }

    #[test]
    fn the_reader_leaves_by_itself_after_the_idle_time_with_its_input_still_open() {
        let (keep_open, input) = mpsc::channel();
        let (left, hear) = mpsc::channel();
        let serving = std::thread::spawn(move || {
            run(std::io::BufReader::new(Open(input)), std::io::sink(), Duration::from_millis(300), move || {
                let _ = left.send(());
            }, |_| Ok(serde_json::json!({})));
        });
        assert!(hear.recv_timeout(Duration::from_secs(5)).is_ok(), "nothing asked for 300 ms: it leaves");
        drop(keep_open);
        assert!(serving.join().is_ok());
    }

    #[test]
    fn a_reader_that_is_asked_does_not_leave() {
        let (keep_open, input) = mpsc::channel();
        let (left, hear) = mpsc::channel();
        let serving = std::thread::spawn(move || {
            run(std::io::BufReader::new(Open(input)), std::io::sink(), Duration::from_millis(1500), move || {
                let _ = left.send(());
            }, |_| Ok(serde_json::json!({})));
        });
        // One request a little under the idle time apart, three times: the clock starts again.
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(900));
            assert!(hear.try_recv().is_err(), "left while being asked");
            for byte in b"{\"forget\": true}\n" {
                assert!(keep_open.send(*byte).is_ok());
            }
        }
        drop(keep_open);
        assert!(serving.join().is_ok());
    }
}
