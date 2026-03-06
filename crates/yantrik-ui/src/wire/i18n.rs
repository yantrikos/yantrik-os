//! i18n wiring — push translations from I18n to Slint's Tr global.
//!
//! Called once at startup. Can be re-called on locale switch.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::i18n::I18n;
use crate::{App, Tr};

/// Push all translations to the Slint Tr global.
pub fn wire(ui: &App, ctx: &AppContext) {
    push_translations(ui, &ctx.i18n);
}

/// Set every Tr property from the i18n store.
pub fn push_translations(ui: &App, i18n: &I18n) {
    let tr = ui.global::<Tr>();

    // ── Common ──
    tr.set_ok(i18n.tr("common.ok").into());
    tr.set_cancel(i18n.tr("common.cancel").into());
    tr.set_save(i18n.tr("common.save").into());
    tr.set_delete_(i18n.tr("common.delete").into());
    tr.set_close(i18n.tr("common.close").into());
    tr.set_back(i18n.tr("common.back").into());
    tr.set_next(i18n.tr("common.next").into());
    tr.set_skip(i18n.tr("common.skip").into());
    tr.set_done(i18n.tr("common.done").into());
    tr.set_search(i18n.tr("common.search").into());
    tr.set_refresh(i18n.tr("common.refresh").into());
    tr.set_loading(i18n.tr("common.loading").into());
    tr.set_error(i18n.tr("common.error").into());
    tr.set_retry(i18n.tr("common.retry").into());
    tr.set_yes(i18n.tr("common.yes").into());
    tr.set_no(i18n.tr("common.no").into());
    tr.set_apply(i18n.tr("common.apply").into());
    tr.set_settings(i18n.tr("common.settings").into());
    tr.set_none_(i18n.tr("common.none").into());
    tr.set_copy(i18n.tr("common.copy").into());
    tr.set_paste(i18n.tr("common.paste").into());
    tr.set_select_all(i18n.tr("common.select_all").into());

    // ── Desktop ──
    tr.set_desktop_greeting(i18n.tr("desktop.greeting").into());
    tr.set_desktop_lens_placeholder(i18n.tr("desktop.lens_placeholder").into());
    tr.set_desktop_no_results(i18n.tr("desktop.no_results").into());

    // ── Dock ──
    tr.set_dock_terminal(i18n.tr("dock.terminal").into());
    tr.set_dock_browser(i18n.tr("dock.browser").into());
    tr.set_dock_files(i18n.tr("dock.files").into());
    tr.set_dock_notes(i18n.tr("dock.notes").into());
    tr.set_dock_editor(i18n.tr("dock.editor").into());
    tr.set_dock_bond(i18n.tr("dock.bond").into());
    tr.set_dock_memory(i18n.tr("dock.memory").into());
    tr.set_dock_alerts(i18n.tr("dock.alerts").into());
    tr.set_dock_system(i18n.tr("dock.system").into());
    tr.set_dock_media(i18n.tr("dock.media").into());
    tr.set_dock_email(i18n.tr("dock.email").into());
    tr.set_dock_calendar(i18n.tr("dock.calendar").into());
    tr.set_dock_network(i18n.tr("dock.network").into());
    tr.set_dock_apps(i18n.tr("dock.apps").into());
    tr.set_dock_settings(i18n.tr("dock.settings").into());

    // ── Screen Titles ──
    tr.set_title_bond(i18n.tr("bond.title").into());
    tr.set_title_personality(i18n.tr("companion.typing").replace("...", "").into()); // Personality has no i18n key, use literal
    tr.set_title_personality("Personality".into()); // TODO: add personality.title to YAML
    tr.set_title_memory(i18n.tr("dock.memory").into());
    tr.set_title_settings(i18n.tr("settings.title").into());
    tr.set_title_files(i18n.tr("files.title").into());
    tr.set_title_notifications(i18n.tr("notifications.title").into());
    tr.set_title_system(i18n.tr("dock.system").into());
    tr.set_title_image_viewer("Image Viewer".into()); // TODO: add viewer.title to YAML
    tr.set_title_editor(i18n.tr("editor.title").into());
    tr.set_title_now_playing(i18n.tr("music.now_playing").into());
    tr.set_title_terminal(i18n.tr("terminal.title").into());
    tr.set_title_notes(i18n.tr("notes.title").into());
    tr.set_title_about(i18n.tr("about.title").into());
    tr.set_title_email(i18n.tr("email.title").into());
    tr.set_title_calendar(i18n.tr("calendar.title").into());
    tr.set_title_packages(i18n.tr("packages.title").into());
    tr.set_title_network(i18n.tr("network.title").into());
    tr.set_title_weather(i18n.tr("weather.title").into());
    tr.set_title_music(i18n.tr("music.title").into());
    tr.set_title_sysmonitor(i18n.tr("sysmonitor.title").into());
    tr.set_title_downloads(i18n.tr("downloads.title").into());
    tr.set_title_snippets(i18n.tr("snippets.title").into());
    tr.set_title_containers(i18n.tr("containers.title").into());
    tr.set_title_devices(i18n.tr("devices.title").into());
    tr.set_title_permissions(i18n.tr("permissions.title").into());

    // ── Boot ──
    tr.set_boot_loading(i18n.tr("boot.loading").into());
    tr.set_boot_starting(i18n.tr("boot.starting").into());

    // ── Lock Screen ──
    tr.set_lock_enter_pin(i18n.tr("lock.enter_pin").into());
    tr.set_lock_wrong_pin(i18n.tr("lock.wrong_pin").into());

    // ── Terminal ──
    tr.set_terminal_new_tab(i18n.tr("terminal.new_tab").into());
    tr.set_terminal_close_tab(i18n.tr("terminal.close_tab").into());

    // ── Notes ──
    tr.set_notes_new_note(i18n.tr("notes.new_note").into());
    tr.set_notes_no_notes(i18n.tr("notes.no_notes").into());

    // ── Email ──
    tr.set_email_inbox(i18n.tr("email.inbox").into());
    tr.set_email_sent(i18n.tr("email.sent").into());
    tr.set_email_drafts(i18n.tr("email.drafts").into());
    tr.set_email_compose(i18n.tr("email.compose").into());
    tr.set_email_no_emails(i18n.tr("email.no_emails").into());

    // ── Files ──
    tr.set_files_name(i18n.tr("files.name").into());
    tr.set_files_size(i18n.tr("files.size").into());
    tr.set_files_modified(i18n.tr("files.modified").into());
    tr.set_files_empty(i18n.tr("files.empty").into());
    tr.set_files_show_hidden(i18n.tr("files.show_hidden").into());

    // ── Power ──
    tr.set_power_shutdown(i18n.tr("power.shutdown").into());
    tr.set_power_restart(i18n.tr("power.restart").into());
    tr.set_power_sleep(i18n.tr("power.sleep").into());
    tr.set_power_logout(i18n.tr("power.logout").into());

    // ── Companion ──
    tr.set_companion_thinking(i18n.tr("companion.thinking").into());
    tr.set_companion_typing(i18n.tr("companion.typing").into());
    tr.set_companion_error(i18n.tr("companion.error").into());

    // ── Bond ──
    tr.set_bond_score(i18n.tr("bond.score").into());
    tr.set_bond_level(i18n.tr("bond.level").into());
    tr.set_bond_interactions(i18n.tr("bond.interactions").into());
    tr.set_bond_days(i18n.tr("bond.days").into());
    tr.set_bond_streak(i18n.tr("bond.streak").into());

    // ── Notifications ──
    tr.set_notifications_no_notifications(i18n.tr("notifications.no_notifications").into());
    tr.set_notifications_clear_all(i18n.tr("notifications.clear_all").into());

    // ── Downloads ──
    tr.set_downloads_add_url(i18n.tr("downloads.add_url").into());
    tr.set_downloads_active(i18n.tr("downloads.active").into());
    tr.set_downloads_completed(i18n.tr("downloads.completed").into());

    // ── Snippets ──
    tr.set_snippets_new(i18n.tr("snippets.new").into());
    tr.set_snippets_no_snippets(i18n.tr("snippets.no_snippets").into());

    // ── Containers ──
    tr.set_containers_running(i18n.tr("containers.running").into());
    tr.set_containers_stopped(i18n.tr("containers.stopped").into());
    tr.set_containers_images(i18n.tr("containers.images").into());

    // ── Weather ──
    tr.set_weather_humidity(i18n.tr("weather.humidity").into());
    tr.set_weather_wind(i18n.tr("weather.wind").into());

    // ── Music ──
    tr.set_music_library(i18n.tr("music.library").into());
    tr.set_music_now_playing(i18n.tr("music.now_playing").into());
    tr.set_music_no_tracks(i18n.tr("music.no_tracks").into());

    // ── Packages ──
    tr.set_packages_install(i18n.tr("packages.install").into());
    tr.set_packages_remove(i18n.tr("packages.remove").into());
    tr.set_packages_search(i18n.tr("packages.search").into());

    tracing::info!(locale = %i18n.locale(), "Translations pushed to Slint UI");
}
