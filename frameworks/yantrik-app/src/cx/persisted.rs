//! The core's side of the settings (design/app-framework-2026-10-21.md, §6): a save asked for
//! by a command goes to the lane; what the lane and the housekeeping thread report comes back
//! as an event the app may answer. The store is the authority on what is committed; nothing
//! here touches the model.

use std::sync::Arc;

use std::time::Duration;

use super::{Core, Inbound};
use crate::app::{App, Event};
use crate::drafts;
use crate::persist::{Op, PersistKey, Report, Write, Written, SUPERSEDED};
use crate::queue::Post;
use crate::settings::{Health, Reloaded};
use crate::watch::{Notice, Seen};

/// Delivers a finished write to the core's queue.
pub(super) struct PostWritten<A: App>(pub(super) Post<Inbound<A>>);

impl<A: App> Report for PostWritten<A> {
    fn written(&self, written: Written) {
        let _ = self.0.post(Inbound::Written(written));
    }
}

/// Delivers what the housekeeping thread saw to the core's queue.
struct PostSeen<A: App>(Post<Inbound<A>>);

impl<A: App> Notice for PostSeen<A> {
    fn seen(&self, seen: Seen) {
        let _ = self.0.post(Inbound::Seen(seen));
    }
}

impl<A: App> Core<A> {
    /// `fx.save_settings`: the value is desired now; the write follows, in order.
    pub(super) fn save_settings(&self, value: A::Settings, replace_unusable: bool) {
        let held_back = {
            let mut store = self.settings.borrow_mut();
            if replace_unusable {
                store.take_over();
            }
            store.desire(value);
            match store.health() {
                Health::Fine => None,
                Health::Recovery(e) => Some(e.to_string()),
            }
        };
        match held_back {
            None => self.flush_settings(),
            Some(why) => self.event(Event::SettingsNotSaved(format!(
                "{why}; the change is kept until the file is replaced"
            ))),
        }
    }

    /// A conflict resolved the app's way (or the default: its own value stands).
    pub(super) fn resolve_settings(&self, value: A::Settings) {
        self.settings.borrow_mut().resolve(value);
        self.flush_settings();
    }

    /// Hands the lane whatever the store owes the disk: a save, a migration's rewrite, a
    /// conflict's resolution. Nothing when it is settled or in recovery.
    pub fn flush_settings(&self) {
        let owed = self.settings.borrow_mut().bytes_to_write();
        let Some((revision, bytes)) = owed else {
            return;
        };
        let path = self.settings.borrow().path().to_path_buf();
        self.lane.borrow_mut().write(Write {
            key: PersistKey::Settings,
            revision,
            path,
            op: Op::Put(bytes),
            delay: Duration::ZERO,
        });
    }

    /// `fx.save_draft` and `fx.checkpoint_draft`: the next revision of the draft, to the lane.
    pub(super) fn save_draft(&self, name: String, bytes: Vec<u8>, delay: Duration) {
        self.draft_op(name, Op::Put(bytes), delay);
    }

    /// `fx.discard_draft`: the file removed, ordered behind any save of it.
    pub(super) fn discard_draft(&self, name: String) {
        self.draft_op(name, Op::Remove, Duration::ZERO);
    }

    fn draft_op(&self, name: String, op: Op, delay: Duration) {
        if !drafts::valid_name(&name) {
            self.event(Event::DraftNotSaved {
                name,
                why: "not a draft name (letters, digits, dashes and underscores, up to 64)".into(),
            });
            return;
        }
        let revision = {
            let mut drafts = self.drafts.borrow_mut();
            let revision = drafts.entry(name.clone()).or_insert(0);
            *revision += 1;
            *revision
        };
        let path = drafts::path_in(self.paths.state_dir(), &name);
        self.lane.borrow_mut().write(Write {
            key: PersistKey::Draft(name),
            revision,
            path,
            op,
            delay,
        });
    }

    /// The lane finished a write.
    pub(super) fn written(&self, written: Written) {
        match (written.key, written.result) {
            (PersistKey::Settings, Ok(())) => {
                let settled = {
                    let mut store = self.settings.borrow_mut();
                    store.committed_at(written.revision);
                    !store.is_pending()
                };
                if settled {
                    self.event(Event::SettingsSaved);
                }
            }
            // A newer write took its place; its outcome is the one that counts.
            (_, Err(e)) if e == SUPERSEDED => {}
            (PersistKey::Settings, Err(e)) => {
                tracing::error!(app = self.id, error = %e, "the settings were not saved");
                self.event(Event::SettingsNotSaved(format!(
                    "the settings could not be written: {e}"
                )));
            }
            (PersistKey::Draft(name), Ok(())) => {
                // A discard that landed says nothing: `DraftSaved` means a draft on disk.
                let latest = self.drafts.borrow().get(&name).copied() == Some(written.revision);
                if latest && !written.removed {
                    self.event(Event::DraftSaved(name));
                }
            }
            (PersistKey::Draft(name), Err(why)) => {
                tracing::error!(app = self.id, draft = %name, error = %why, "a draft was not saved");
                self.event(Event::DraftNotSaved { name, why });
            }
        }
    }

    /// The housekeeping thread saw a change.
    pub(super) fn seen(&self, seen: Seen) {
        match seen {
            Seen::Theme(theme) => self.apply_theme(&theme),
            Seen::Reach(reach) => self.event(Event::ReachChanged(reach)),
            // A look taken before the app re-armed the watch is older than what the app knows.
            Seen::File { key, seen, arming } => {
                if self.watched.is_current(&key, arming) {
                    self.event(Event::FileChanged { key, seen });
                }
            }
            Seen::SettingsFile => {
                let reloaded = self.settings.borrow_mut().reload();
                match reloaded {
                    Reloaded::Unchanged => {}
                    Reloaded::Adopted(value) => self.event(Event::SettingsChangedOutside(value)),
                    Reloaded::Conflict(c) => self.event(Event::SettingsConflict {
                        outside: c.outside,
                        mine: c.mine,
                    }),
                    Reloaded::Unusable(e) => {
                        tracing::warn!(app = self.id, "{e}; the settings in memory stand");
                        crate::problems::record_failure(self.id, "settings", &e.to_string());
                    }
                }
            }
        }
    }

    /// The committed settings: what is on disk, by the store's account.
    pub fn committed_settings(&self) -> A::Settings {
        self.settings.borrow().committed().clone()
    }

    /// Whether a save has not reached the disk yet.
    pub fn settings_pending(&self) -> bool {
        self.settings.borrow().is_pending()
    }

    /// Where the settings live, for the housekeeping thread and for tests.
    pub fn settings_path(&self) -> std::path::PathBuf {
        self.settings.borrow().path().to_path_buf()
    }

    /// Writes still in the lane.
    pub fn writes_waiting(&self) -> usize {
        self.lane.borrow().waiting()
    }

    /// The lane's report endpoint is the core's own queue; this is the same for the watcher.
    pub fn notice(&self) -> Arc<dyn crate::watch::Notice> {
        Arc::new(PostSeen(self.post()))
    }
}
