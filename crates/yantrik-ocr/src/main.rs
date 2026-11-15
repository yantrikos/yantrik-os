//! `yantrik-ocr <image>`: the text on a screen capture, as JSON.
//!
//! The desktop reads most windows without looking at them: our own apps describe themselves, and
//! GTK, Qt, Chromium and Firefox publish an accessibility tree. What is left publishes nothing —
//! a terminal, a game, a canvas, an X11 app without a bridge — and `yos screen` used to say only
//! that it could not see inside. This is the eyes for those: the shell captures the display, runs
//! this on it, and hands the lines to whoever asked, with where each one is.
//!
//! Pure Rust (ocrs on rten), with its two models (12 MB) shipped beside the desktop in
//! `share/ocr`, so it needs no network, no GPU and no system package. A separate process rather
//! than a library in the shell: it holds a few hundred MB while it reads, and a crash in it must
//! not take the desktop with it.
//!
//! ```text
//! yantrik-ocr [--models DIR] [--max-lines N] <image.png|image.ppm>
//! → {"width":1280,"height":800,"seconds":1.7,"lines":[{"text":"…","box":[x,y,w,h]}, …],
//!    "clipped":false,"cost":{…}}
//!
//! yantrik-ocr [--models DIR] [--idle-secs N] --serve
//!   one request per line in, one answer per line out, until the input closes (see `serve`)
//! ```
//!
//! Run for one image it reads every line. Kept running with `--serve` it remembers what each
//! line said and reads only what changed (`memory`).

mod memory;
mod read;
mod reading;
mod serve;
mod settle;

use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "usage: yantrik-ocr [--models DIR] [--max-lines N] <image.png|image.ppm>\n       yantrik-ocr [--models DIR] [--idle-secs N] --serve";

/// Where the models are: `--models`, else `share/ocr` beside this binary's `bin/`, else the
/// installed desktop's.
fn models_dir(given: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = given {
        return dir;
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(Path::parent).map(|root| root.join("share/ocr")));
    match beside {
        Some(dir) if dir.join("text-detection.rten").exists() => dir,
        _ => PathBuf::from("/opt/yantrik/share/ocr"),
    }
}

/// What the command line asked for.
#[derive(Debug, PartialEq)]
struct Asked {
    models: Option<PathBuf>,
    what: What,
}

#[derive(Debug, PartialEq)]
enum What {
    One { image: PathBuf, max_lines: usize },
    Serve { idle: Duration },
}

fn asked(args: impl Iterator<Item = String>) -> Result<Asked, String> {
    let mut args = args;
    let (mut models, mut image, mut max_lines, mut serve, mut idle) = (None, None, usize::MAX, false, serve::IDLE);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--models" => models = Some(PathBuf::from(args.next().ok_or("--models needs a directory")?)),
            "--max-lines" => max_lines = args.next().and_then(|n| n.parse().ok()).ok_or("--max-lines needs a number")?,
            "--idle-secs" => idle = Duration::from_secs(args.next().and_then(|n| n.parse().ok()).ok_or("--idle-secs needs a number")?),
            "--serve" => serve = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ if image.is_none() => image = Some(PathBuf::from(arg)),
            other => return Err(format!("one image at a time; `{other}` is a second")),
        }
    }
    let what = match (serve, image) {
        (true, None) => What::Serve { idle },
        (true, Some(image)) => return Err(format!("--serve takes its images as requests, not `{}` on the command line", image.display())),
        (false, Some(image)) => What::One { image, max_lines },
        (false, None) => return Err(USAGE.into()),
    };
    Ok(Asked { models, what })
}

fn run() -> Result<Option<serde_json::Value>, String> {
    let asked = asked(std::env::args().skip(1))?;
    let started = std::time::Instant::now();
    let reader = read::Reader::load(&models_dir(asked.models))?;
    match asked.what {
        What::Serve { idle } => {
            serve::serve(reader, idle);
            Ok(None)
        }
        What::One { image, max_lines } => {
            let rgb = image::open(&image).map_err(|e| format!("cannot read {}: {e}", image.display()))?.into_rgb8();
            let (width, height) = rgb.dimensions();
            // One look with a memory nothing is in: every line is read.
            let mut reading = reading::look(&reader, &mut memory::Memory::default(), "", rgb.as_raw(), width, height)?;
            // As it always was: from the start of the process, the models' loading included.
            reading.seconds = started.elapsed().as_secs_f64();
            Ok(Some(reading.to_json(max_lines)))
        }
    }
}

fn main() {
    match run() {
        Ok(Some(out)) => println!("{out}"),
        Ok(None) => {}
        Err(e) => {
            eprintln!("yantrik-ocr: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> impl Iterator<Item = String> + '_ {
        line.split_whitespace().map(str::to_string)
    }

    #[test]
    fn one_image_is_read_once_as_before() {
        let one = asked(args("--max-lines 300 /run/user/1000/shot.ppm"));
        assert_eq!(one, Ok(Asked { models: None, what: What::One { image: "/run/user/1000/shot.ppm".into(), max_lines: 300 } }));
        assert!(asked(args("a.png b.png")).is_err(), "one image at a time");
        assert!(asked(args("")).is_err(), "nothing to read");
    }

    #[test]
    fn serving_takes_no_image_and_may_be_told_how_long_to_stay() {
        assert_eq!(asked(args("--serve")), Ok(Asked { models: None, what: What::Serve { idle: serve::IDLE } }));
        let short = asked(args("--models /opt/x --idle-secs 5 --serve"));
        assert_eq!(short, Ok(Asked { models: Some("/opt/x".into()), what: What::Serve { idle: Duration::from_secs(5) } }));
        assert!(asked(args("--serve shot.ppm")).is_err(), "an image on the command line is not a request");
        assert!(asked(args("--idle-secs soon --serve")).is_err());
    }
}
