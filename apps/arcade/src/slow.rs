//! The three commands that reach a browser, behind one seam: the real browser, or under a test a
//! stand-in that answers when the test says. Blocking: from a job only.
//!
//! # A job commits only when it succeeds
//!
//! A run writes its screenshot into a staging folder of its own (`/tmp/yarc-*`, private, swept if
//! the app dies), and nothing into the library while it works. Only a run that finishes, was not
//! stopped because the app is closing, and whose build is still the one it judged, commits its
//! screenshot and verdict, one atomic write each (`Library::commit`). A run that is stopped or
//! fails leaves the library's earlier screenshot and verdict exactly as they were; its staging goes
//! with its folder.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::browser::{self, JobScope};
use crate::command::Verb;
use crate::library::{stamp_of, Library};
use crate::verify::Report;
use crate::{play, verify};

pub trait Slow: Send + Sync {
    /// Runs `verb` (one of play, verify, screenshot) on the built game `slug` whose file is
    /// `html`: the verdict in a sentence, or why it failed.
    fn run(&self, verb: Verb, library: &Library, slug: &str, html: &Path) -> Result<String, String>;
}

/// The verifier and the screenshot taker, as functions, so a test can stand in for the browser
/// and still run the real wiring around it (staging, cancellation, the commit).
pub type Verifier = fn(&Path, Option<&Path>) -> Result<Report, String>;
pub type Shooter = fn(&Path, &Path) -> Result<(), String>;

/// The Browser and the headless verifier.
#[cfg_attr(test, allow(dead_code))]
pub struct Real {
    verify: Verifier,
    shoot: Shooter,
}

#[cfg_attr(test, allow(dead_code))]
impl Real {
    pub fn new() -> Real {
        Real { verify: verify::verify_game, shoot: verify::screenshot_game }
    }

    /// Stand-ins for the browser, for a test of everything around it.
    #[cfg(test)]
    pub fn with(verify: Verifier, shoot: Shooter) -> Real {
        Real { verify, shoot }
    }
}

/// A staging folder that goes when the run ends, however it ends.
struct Staging(std::path::PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        browser::remove_profile(&self.0);
    }
}

impl Slow for Real {
    fn run(&self, verb: Verb, library: &Library, slug: &str, html: &Path) -> Result<String, String> {
        // This run's own handle on its cancellation flag. The browser handle dropping must not be
        // able to clear what is read here: `verify_game` drops its browser before it returns.
        let cancel = Arc::new(AtomicBool::new(false));
        let _scope = JobScope::enter(cancel.clone());
        let stage = Staging(browser::new_tmp(Path::new("/tmp"))?);
        let staged = stage.0.join("screenshot.png");
        // The build this run judges. If the game is built again or removed while the browser is
        // at work, what it found is about a file that is gone, and is dropped.
        let judged = stamp_of(html);
        let stopped = || cancel.load(Ordering::SeqCst);
        let kept = |library: &Library| std::fs::read(&staged).ok().filter(|_| library.game_dir(slug).is_dir());
        match verb {
            Verb::Verify => {
                let ran = (self.verify)(html, Some(&staged));
                if stopped() {
                    return Err(browser::STOPPED.into());
                }
                let report = ran?;
                let value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
                library.commit(slug, &judged, kept(library), Some(&value))?;
                // The verifier's own sentence, so the job, the games list and the banner agree,
                // and so an inconclusive run says it was the machine rather than reading as a
                // verdict on the game. It is still not a pass: the job lands as failed until a
                // machine settles it.
                if report.passed {
                    Ok(format!("{slug}: {}", report.summary_line()))
                } else {
                    Err(format!("{slug}: verification {}", report.summary_line()))
                }
            }
            Verb::Screenshot => {
                let ran = (self.shoot)(html, &staged);
                if stopped() {
                    return Err(browser::STOPPED.into());
                }
                ran?;
                library.commit(slug, &judged, kept(library), None)?;
                Ok(format!("{} \u{2192} {}", slug, library.screenshot_path(slug).display()))
            }
            // If the Browser cannot draw the game, play renders it headless instead and says
            // where the picture is; that picture is staged and committed like any other.
            Verb::Play => {
                let said = play::play(html, &staged);
                if stopped() {
                    return Err(browser::STOPPED.into());
                }
                let said = said?;
                if let Some(bytes) = kept(library) {
                    library.commit(slug, &judged, Some(bytes), None)?;
                }
                Ok(said.replace(&staged.display().to_string(), &library.screenshot_path(slug).display().to_string()))
            }
            other => Err(format!("{} is not a browser command", other.name())),
        }
    }
}

/// What an app started at `paths` reaches its browser through: the real one, or a test's.
#[cfg(not(test))]
pub fn for_app(_: &yantrik_app::Paths) -> std::sync::Arc<dyn Slow> {
    std::sync::Arc::new(Real::new())
}

#[cfg(test)]
pub use crate::tests::slow_for as for_app;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{meadow_run, PIP};

    /// A library with a built game, an earlier job's screenshot and its verdict.
    fn built() -> (Library, std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("arcade-settle-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        let library = Library::at(&dir);
        library.save_character(PIP).unwrap();
        library.save_game(&meadow_run("Meadow Run")).unwrap();
        let (slug, html) = library.build_game("meadow-run").unwrap();
        std::fs::write(library.screenshot_path(&slug), b"OLD PNG").unwrap();
        library.write_verify(&slug, &serde_json::json!({ "earlier": "verdict" })).unwrap();
        (library, dir, html)
    }

    /// A verifier that writes its screenshot, as the real one does at its third gate, and then
    /// goes on as the app closes under it: the browser dropped, the job cancelled.
    fn verify_then_cancelled(_: &Path, shot: Option<&Path>) -> Result<Report, String> {
        std::fs::write(shot.unwrap(), b"NEW PNG").unwrap();
        browser::current().expect("the run is in a job scope").store(true, Ordering::SeqCst);
        // What `verify_game` does as it returns: its browser handle is dropped.
        browser::set_current(None);
        Ok(Report::new("stand-in".into()).finish())
    }

    fn verify_ok(_: &Path, shot: Option<&Path>) -> Result<Report, String> {
        std::fs::write(shot.unwrap(), b"NEW PNG").unwrap();
        Ok(Report::new("stand-in".into()).finish())
    }

    fn shoot_ok(_: &Path, out: &Path) -> Result<(), String> {
        std::fs::write(out, b"NEW PNG").unwrap();
        Ok(())
    }

    /// Review (5th 520 run): a Verify closed after its PNG was written. The earlier screenshot and
    /// verdict are byte for byte what they were, and the new ones are nowhere in the library. The
    /// stand-in drops the thread's flag as the real browser handle does, so the run must hold its
    /// own.
    #[test]
    fn a_verify_cancelled_after_its_shot_commits_nothing_and_the_earlier_files_stand() {
        let (library, dir, html) = built();
        let real = Real::with(verify_then_cancelled, shoot_ok);
        let err = real.run(Verb::Verify, &library, "meadow-run", &html).unwrap_err();
        assert_eq!(err, browser::STOPPED);
        assert_eq!(std::fs::read(library.screenshot_path("meadow-run")).unwrap(), b"OLD PNG");
        assert!(std::fs::read_to_string(library.verify_path("meadow-run")).unwrap().contains("earlier"));
        let in_game: Vec<_> = std::fs::read_dir(library.game_dir("meadow-run")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(in_game.len(), 4, "spec, build, screenshot and verdict only: {in_game:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_verify_that_finishes_commits_its_screenshot_and_verdict() {
        let (library, dir, html) = built();
        let real = Real::with(verify_ok, shoot_ok);
        real.run(Verb::Verify, &library, "meadow-run", &html).unwrap();
        assert_eq!(std::fs::read(library.screenshot_path("meadow-run")).unwrap(), b"NEW PNG");
        assert!(!std::fs::read_to_string(library.verify_path("meadow-run")).unwrap().contains("earlier"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_verify_whose_build_changed_commits_nothing() {
        let (library, dir, html) = built();
        fn verify_then_rebuilt(html: &Path, shot: Option<&Path>) -> Result<Report, String> {
            std::fs::write(shot.unwrap(), b"NEW PNG").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::write(html, b"<html>another build</html>").unwrap();
            Ok(Report::new("stand-in".into()).finish())
        }
        let real = Real::with(verify_then_rebuilt, shoot_ok);
        let err = real.run(Verb::Verify, &library, "meadow-run", &html).unwrap_err();
        assert!(err.contains("built again"), "{err}");
        assert_eq!(std::fs::read(library.screenshot_path("meadow-run")).unwrap(), b"OLD PNG");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_screenshot_cancelled_after_it_was_taken_commits_nothing() {
        let (library, dir, html) = built();
        fn shoot_then_cancelled(_: &Path, out: &Path) -> Result<(), String> {
            std::fs::write(out, b"NEW PNG").unwrap();
            browser::current().unwrap().store(true, Ordering::SeqCst);
            browser::set_current(None);
            Ok(())
        }
        let real = Real::with(verify_ok, shoot_then_cancelled);
        assert_eq!(real.run(Verb::Screenshot, &library, "meadow-run", &html).unwrap_err(), browser::STOPPED);
        assert_eq!(std::fs::read(library.screenshot_path("meadow-run")).unwrap(), b"OLD PNG");
        real.run(Verb::Screenshot, &library, "meadow-run", &html).ok();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The run's staging folder is gone when it ends, whichever way it ends.
    #[test]
    fn the_staging_folder_goes_with_the_run() {
        let (library, dir, html) = built();
        static STAGED: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);
        fn remember(_: &Path, shot: Option<&Path>) -> Result<Report, String> {
            *STAGED.lock().unwrap() = shot.map(|s| s.parent().unwrap().to_path_buf());
            Err("the browser failed".into())
        }
        let real = Real::with(remember, shoot_ok);
        assert!(real.run(Verb::Verify, &library, "meadow-run", &html).is_err());
        let stage = STAGED.lock().unwrap().clone().unwrap();
        assert!(!stage.exists(), "{} was left behind", stage.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
