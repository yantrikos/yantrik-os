//! Moves the mail accounts an older version kept in `config.yaml` into the mail service and the
//! credential vault (#1022), at start and again whenever the vault opens.
//!
//! What a pass does, and what it promises, is in `yantrik_companion::email::legacy`. This is the
//! timing, which is the same as the saved providers' and the mail service's own migrations: one
//! pass now, one a minute for an hour in case the vault is only slow to start, and one at each
//! opening, for as long as a pass says passwords are left in the file. A shut or failing vault
//! leaves `config.yaml` exactly as it was.
//!
//! When a pass changes the file, the person is told in a notification where the archive of the
//! old file is. That copy holds the old password; it is theirs to delete.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use yantrik_app_runtime::notify;
use yantrik_companion::email::legacy;
use yantrik_companion::email::port::ServiceMail;
use yantrik_foundation::vault::opening::{self, Pass};
use yantrik_ipc_contracts::notifications::Urgency;

const RETRY_EVERY: Duration = Duration::from_secs(60);
const RETRIES: usize = 60;

/// Start the migration of the config the shell was started with. Called once, after the credential
/// door is up: the mail service reaches the vault through it. A shell started without a config
/// file has nothing to migrate.
pub fn start(config: Option<PathBuf>) {
    let Some(config) = config else { return };
    let pass: Pass = Arc::new(move |when| {
        let (left, notice) = legacy::pass(&config, &ServiceMail, when);
        if let Some(text) = notice {
            tell(text);
        }
        left
    });
    if opening::until_done("mail-config-migrate", pass, RETRY_EVERY, RETRIES).is_err() {
        tracing::warn!("no thread to move mail accounts out of config.yaml");
    }
}

fn tell(text: String) {
    notify::send(
        notify::Notification::new("Yantrik", "Mail passwords moved to the vault".to_string())
            .body(text)
            .urgency(Urgency::Normal),
    );
}
