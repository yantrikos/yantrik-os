//! email-service's account file, `~/.config/yantrik/email.json`, holds the account password in
//! clear text. It was, until the framework moved app settings to `yantrik/apps/<id>/`, also
//! where the framework looked for Email's settings: read and parsed at every start. Here it sits
//! where the service keeps it, with a sentinel for a password, and Email starts, works and shuts
//! down beside it: the sentinel is never in what Email shows or says, never in the log, and the
//! file is never written, moved or set aside.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, Once};

use yantrik_app::{App as _, Paths};

use super::{quiet, start_at, summary};
use crate::backend::fake::FakeMail;
use crate::Email;

const SENTINEL: &str = "SENTINEL-EMAIL-PASSWORD-7f3a";

/// Everything this test binary logs, at every level, from every thread.
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
        // Another test of this binary may have set one first; then this one sees nothing, and
        // the assertion on the log below would pass vacuously, so it is checked to be ours.
        assert!(tracing::subscriber::set_global_default(subscriber).is_ok(), "the log is captured");
    });
}

fn logged() -> String {
    String::from_utf8_lossy(&LOG.lock().unwrap_or_else(|p| p.into_inner())).to_string()
}

/// `<config>/yantrik/email.json` under the test's own root: where the service keeps it.
fn account_file(paths: &Paths) -> PathBuf {
    let apps = paths.config_file().ancestors().nth(3).expect("the settings sit three folders under the shared config folder");
    apps.join("email.json")
}

#[test]
fn the_mail_services_account_file_is_never_read_logged_or_written() {
    capture_the_log();
    let paths = Paths::temporary(Email::ID);
    let account = account_file(&paths);
    std::fs::create_dir_all(account.parent().unwrap()).unwrap();
    let content = format!(
        r#"[{{"id":"default","email":"p@example.org","password":"{SENTINEL}","imap_server":"imap.example.org","oauth_token":"{SENTINEL}"}}]"#
    );
    let mut file = std::fs::File::create(&account).unwrap();
    file.write_all(content.as_bytes()).unwrap();
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&account, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(paths.former_config_file("email.json"), None, "never an app's to read");
    assert_ne!(paths.config_file(), account.as_path(), "Email's settings are not the account file");

    let (app, _) = start_at(paths.clone(), &FakeMail::ready(), yantrik_app::mind::fake::FakeMind::new());
    quiet(&app);
    let shown = format!("{} {}", app.describe(), summary(&app));
    assert!(!shown.contains(SENTINEL), "nothing of the account file reaches the model");
    app.ui(|ui| ui.invoke_sync_emails());
    quiet(&app);
    app.run_writes();
    let _ = app.shutdown();
    drop(app);

    assert_eq!(std::fs::read_to_string(&account).unwrap(), content, "never written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&account).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let beside: Vec<String> = std::fs::read_dir(account.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(beside.iter().all(|n| n == "email.json" || n == "apps"), "nothing set aside or made beside it: {beside:?}");
    assert!(!paths.imported_record().exists(), "Email names no former file; the old place is not looked at");
    let log = logged();
    assert!(!log.contains(SENTINEL), "the password never reaches the log");
    assert!(!log.contains("not in a shape") && !log.contains("not one this app wrote"), "the file is not even looked at");
}
