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
//! than a library in the shell: it holds a few hundred MB while it reads, for a few seconds, and
//! a crash in it must not take the desktop with it.
//!
//! ```text
//! yantrik-ocr [--models DIR] [--max-lines N] <image.png|image.ppm>
//! → {"width":1280,"height":800,"seconds":1.7,"lines":[{"text":"…","box":[x,y,w,h]}, …]}
//!   (with "clipped": true when there were more than N lines)
//! ```

mod read;

use std::path::{Path, PathBuf};
use std::time::Instant;

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

fn run() -> Result<serde_json::Value, String> {
    let mut args = std::env::args().skip(1);
    let mut models = None;
    let mut image = None;
    let mut max_lines = usize::MAX;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--models" => models = Some(PathBuf::from(args.next().ok_or("--models needs a directory")?)),
            "--max-lines" => {
                max_lines = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or("--max-lines needs a number")?
            }
            "-h" | "--help" => return Err("usage: yantrik-ocr [--models DIR] <image.png|image.ppm>".into()),
            _ if image.is_none() => image = Some(PathBuf::from(arg)),
            other => return Err(format!("one image at a time; `{other}` is a second")),
        }
    }
    let image = image.ok_or("usage: yantrik-ocr [--models DIR] <image.png|image.ppm>")?;

    let started = Instant::now();
    let rgb = image::open(&image)
        .map_err(|e| format!("cannot read {}: {e}", image.display()))?
        .into_rgb8();
    let (width, height) = rgb.dimensions();
    let reader = read::Reader::load(&models_dir(models))?;
    let mut lines = reader.lines(rgb.as_raw(), width, height)?;
    let clipped = lines.len() > max_lines;
    lines.truncate(max_lines);
    Ok(serde_json::json!({
        "clipped": clipped,
        "width": width,
        "height": height,
        "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        "lines": lines.iter().map(|l| serde_json::json!({
            "text": l.text,
            "box": [l.x, l.y, l.w, l.h],
        })).collect::<Vec<_>>(),
    }))
}

fn main() {
    match run() {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("yantrik-ocr: {e}");
            std::process::exit(1);
        }
    }
}
