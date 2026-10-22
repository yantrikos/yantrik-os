//! The settings store never reads or writes a file that is not its own, through the real core
//! and lane (`TestApp`): an app named `email`, with settings to save, beside email-service's
//! account file; and a file without the app's mark where the settings go. A sentinel stands for
//! the password, and the whole log of this binary is captured to show it never gets there.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use serde::{Deserialize, Serialize};
use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::refused::Refused;
use yantrik_app::settings::{Settings, SettingsError};
use yantrik_app::{Health, Paths};
use yantrik_testing::TestApp;

slint::slint! {
    export component GuardWin inherits Window {}
}

impl yantrik_app::Window for GuardWin {
    fn make() -> Result<Self, slint::PlatformError> {
        GuardWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

const SENTINEL: &str = "SENTINEL-ACCOUNT-PASSWORD-91c2";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
struct Prefs {
    signature: String,
}

impl Settings for Prefs {
    const SCHEMA: u32 = 1;
    // An app that wrongly names the service's file as its own: refused all the same.
    const FORMER_FILE: Option<&'static str> = Some("email.json");
    fn legacy(raw: serde_json::Value) -> Option<serde_json::Value> {
        Some(raw)
    }
}

#[derive(Default)]
struct Model {
    prefs: Prefs,
    health: Option<Health>,
    not_saved: Vec<String>,
    saved: u32,
}

#[derive(Debug, Clone)]
enum Command {
    Save(String),
    Replace(String),
    NotSaved(String),
    Saved,
}

/// Named as the real Email is, so its old settings path was the service's account file.
struct Mail;

impl App for Mail {
    type Ui = GuardWin;
    type Model = Model;
    type Command = Command;
    type Settings = Prefs;
    const ID: &'static str = "email";

    fn model(boot: &Boot<Prefs>) -> Result<Model, Startup> {
        Ok(Model { prefs: boot.settings.clone(), health: Some(boot.settings_health.clone()), ..Model::default() })
    }

    fn wire(_: &Cx<Self>) {}

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Save(s) => {
                m.prefs.signature = s;
                fx.save_settings(m.prefs.clone());
            }
            Command::Replace(s) => {
                m.prefs.signature = s;
                fx.replace_settings_file(m.prefs.clone());
            }
            Command::NotSaved(why) => m.not_saved.push(why),
            Command::Saved => m.saved += 1,
        }
        Ok(())
    }

    fn event(event: Event<Prefs>, _: &Model) -> Option<Command> {
        match event {
            Event::SettingsNotSaved(why) => Some(Command::NotSaved(why)),
            Event::SettingsSaved => Some(Command::Saved),
            _ => None,
        }
    }

    fn render(_: &GuardWin, _: WindowId, _: &Model, _: &Changed) {}
}

static LOG: Mutex<Vec<u8>> = Mutex::new(Vec::new());

struct Sink;

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        LOG.lock().unwrap_or_else(|p| p.into_inner()).extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_the_log() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(|| Sink)
            .finish();
        assert!(tracing::subscriber::set_global_default(subscriber).is_ok(), "the log is captured");
    });
}

fn logged() -> String {
    String::from_utf8_lossy(&LOG.lock().unwrap_or_else(|p| p.into_inner())).to_string()
}

/// `<config>/yantrik/` under the test's root.
fn shared_config(paths: &Paths) -> PathBuf {
    paths.config_file().ancestors().nth(3).expect("<config>/yantrik/apps/<id>/settings.json").to_path_buf()
}

fn account_list() -> String {
    format!(r#"[{{"id":"default","email":"p@example.org","password":"{SENTINEL}"}}]"#)
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

#[test]
fn settings_saved_by_an_app_named_email_never_touch_the_services_account_file() {
    capture_the_log();
    let paths = Paths::temporary(Mail::ID);
    let account = shared_config(&paths).join("email.json");
    let content = account_list();
    write(&account, &content);

    let app = TestApp::<Mail>::start_at(paths.clone(), Vec::new());
    assert_eq!(app.model(|m| m.health.clone()), Some(Health::Fine), "nothing at its own path: the defaults, fine");
    assert_eq!(app.model(|m| m.prefs.clone()), Prefs::default(), "nothing of the account file is read in");
    assert!(!paths.imported_record().exists(), "the service's file is refused as a former file: not looked at");
    app.run(Command::Save("Regards".into()));
    app.run_writes();
    assert_eq!(app.model(|m| m.saved), 1);
    let _ = app.shutdown();
    drop(app);

    assert_eq!(std::fs::read_to_string(&account).unwrap(), content, "the account file is as it was");
    let mine = std::fs::read_to_string(paths.config_file()).unwrap();
    assert!(mine.contains("Regards") && mine.contains("\"app\": \"email\""), "{mine}");
    assert_eq!(TestApp::<Mail>::start_at(paths.clone(), Vec::new()).model(|m| m.prefs.signature.clone()), "Regards");
    assert!(!logged().contains(SENTINEL), "the password never reaches the log");
}

/// A file without the app's mark where its settings go: never parsed into the model, never
/// written over by a save, its path alone said; the person's "replace" sets it aside, intact.
#[test]
fn a_file_that_is_not_the_apps_is_never_parsed_or_written_over() {
    capture_the_log();
    let paths = Paths::temporary(Mail::ID);
    let foreign = format!(r#"{{"schema":1,"settings":{{"signature":"{SENTINEL}"}}}}"#);
    write(paths.config_file(), &foreign);

    let app = TestApp::<Mail>::start_at(paths.clone(), Vec::new());
    assert_eq!(app.model(|m| m.health.clone()), Some(Health::Recovery(SettingsError::NotOurs)));
    assert_eq!(app.model(|m| m.prefs.clone()), Prefs::default(), "not read past the mark");
    app.run(Command::Save("mine".into()));
    app.run_writes();
    let said = app.model(|m| m.not_saved.clone());
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains("not one this app wrote") && !said[0].contains(SENTINEL), "{said:?}");
    assert_eq!(std::fs::read_to_string(paths.config_file()).unwrap(), foreign, "not written over");

    // The person chooses to replace it: it is set aside beside itself first, as it was.
    app.run(Command::Replace("mine".into()));
    app.run_writes();
    assert_eq!(app.model(|m| m.saved), 1);
    let _ = app.shutdown();
    drop(app);
    let dir = paths.config_file().parent().unwrap();
    let kept: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("settings.json.set-aside-"))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), foreign, "kept intact, never deleted");
    assert!(std::fs::read_to_string(paths.config_file()).unwrap().contains("\"mine\""));
    let log = logged();
    assert!(!log.contains(SENTINEL), "the file's values never reach the log");
    assert!(log.contains(&paths.config_file().display().to_string()), "its path does");
}
