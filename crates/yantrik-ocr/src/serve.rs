//! The reader kept running between looks: `yantrik-ocr --serve`.
//!
//! Started for each look, the reader loads its models, reads everything and forgets it all. Kept
//! running, it loads them once and remembers what each line said (`memory`), so a look at a
//! screen that changed a little costs a little.
//!
//! One request per line on standard input, one answer per line on standard output:
//!
//! ```text
//! {"image": "/run/user/1000/yantrik-read-1.ppm", "space": "desktop", "max_lines": 300}
//! → {"width":1280,"height":800,"seconds":0.4,"lines":[…],"clipped":false,"cost":{…}}
//! {"forget": true}
//! → {"forgot": true}
//! ```
//!
//! A request that cannot be answered gets `{"error": "…"}` and the reader carries on.
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

/// The display a look is of when the request does not say: its memory is shared with nothing
/// that names one.
const UNNAMED: &str = "";

/// One line of input, understood.
#[derive(Debug, PartialEq)]
pub enum Request {
    Look { image: PathBuf, space: String, max_lines: usize },
    Forget,
}

/// Understand one line of input.
pub fn request(line: &str) -> Result<Request, String> {
    let json: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("not a request: {e}"))?;
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
    let last_asked = Arc::new(Mutex::new(Instant::now()));
    let busy = Arc::new(AtomicBool::new(false));
    {
        let (last_asked, busy) = (Arc::clone(&last_asked), Arc::clone(&busy));
        // The main thread is blocked in a read of standard input, which has no time limit of
        // its own. This thread is the limit.
        let spawned = std::thread::Builder::new().name("idle".into()).spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let quiet_for = last_asked.lock().map_or(Duration::ZERO, |at| at.elapsed());
            if !busy.load(Ordering::Acquire) && quiet_for >= idle {
                std::process::exit(0);
            }
        });
        if let Err(e) = spawned {
            // Without the limit the reader would stay for as long as its input stays open.
            eprintln!("yantrik-ocr: no idle timer ({e}); not staying");
            return;
        }
    }

    let mut memory = Memory::default();
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        busy.store(true, Ordering::Release);
        let reply = request(&line).and_then(|request| answer(&reader, &mut memory, request)).unwrap_or_else(|why| serde_json::json!({ "error": why }));
        if let Ok(mut at) = last_asked.lock() {
            *at = Instant::now();
        }
        busy.store(false, Ordering::Release);
        let mut out = stdout.lock();
        if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
            // Nobody is reading the answers any more.
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
