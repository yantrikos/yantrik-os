//! The first lines every app process runs: `--version`, the fullscreen guard, tracing and the
//! panic hook. The runner does this once; `yantrik_app_runtime::init_tracing` is this function.

use crate::problems;

/// Initialize tracing-subscriber with an env filter for the app, after answering `--version`.
pub fn init_tracing(app_name: &str) {
    init_tracing_for(app_name, None);
}

/// [`init_tracing`] for an app whose crate is not named after its id: the runner passes the
/// crate the app's type lives in (`yantrik_container_manager` for the app `containers`), so
/// its own lines are shown whatever the two are called.
pub fn init_tracing_for(app_name: &str, crate_target: Option<&str>) {
    // `--version`, answered here because this is the one line every app binary runs first.
    //
    // A machine used to give three answers about what it was, one of them a hardcoded
    // `Yantrik Terminal v0.1.0` printed into the terminal's first pane. Sixteen app mains each
    // reporting their own `CARGO_PKG_VERSION` is the same defect with more places to forget, so
    // the flag is handled once, from the string yantrik-version resolves. Same reasoning as the
    // SLINT_FULLSCREEN line below: the shared line is the only place a rule holds everywhere.
    yantrik_version::handle_version_flag(app_name);

    // An app is not the OS.
    //
    // The session exports SLINT_FULLSCREEN=1 because the shell IS the desktop and must not be a
    // window on something else. Children inherit the environment, so any launch path that
    // forgets to strip it hands an app a fullscreen window with no titlebar, no taskbar and no
    // way to close it. That happened: the Apps grid spawned a bare Command, and Notes opened
    // over the whole screen with nothing to press.
    //
    // The launcher strips it, and now so does this. Belt and braces, because the cost of the
    // belt failing is a window the user cannot get out of, and the cost of this line is
    // nothing: no app in this OS ever wants to start fullscreen.
    std::env::remove_var("SLINT_FULLSCREEN");

    // A panic becomes a problem record before it becomes a stack trace on stderr. The record
    // is local and scrubbed; nothing sends it. See `problems`.
    problems::install_panic_hook(app_name);

    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    let mut filter = tracing_subscriber::EnvFilter::from_default_env();
    // The framework's own lines (instance guard, theme) must be visible too, or an app that
    // exits at once because another instance holds the slot says nothing.
    let mut defaults = app_directives(app_name, crate_target);
    defaults.extend([
        "yantrik_app_runtime=info".to_string(),
        "yantrik_app=info".to_string(),
    ]);
    let asked = std::env::var("RUST_LOG").unwrap_or_default();
    for directive in unless_asked(defaults, &asked) {
        // The crate's target comes from `type_name`, whose format is not promised: a target that
        // does not parse costs a hidden line, never a start.
        if let Ok(directive) = directive.parse() {
            filter = filter.add_directive(directive);
        }
    }
    // A second init (a test harness after the app) is not an error worth stopping for.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        // The same lines, kept in a small ring so a problem record can carry the last forty.
        .with(RecentLines)
        .try_init();
}

/// The log targets that are the app's own, at `info`.
///
/// An app on the old runtime names itself by its crate (`yantrik-notes`, whose lines are
/// tagged `yantrik_notes`); an app on the framework is named by its id (`weather`), while its
/// crate is `yantrik-weather` and its lines are tagged `yantrik_weather`. A filter built from the
/// id alone matched nothing the app wrote, so its own lines (a kill that failed, a fetch that
/// fell back) were hidden unless `RUST_LOG` was set. The id's spellings are enabled, and the
/// crate the runner names, which is the only one that is right when the id is not the crate's
/// suffix (`containers` in `yantrik_container_manager`).
fn app_directives(app_name: &str, crate_target: Option<&str>) -> Vec<String> {
    let target = app_name.replace('-', "_");
    let mut targets = vec![target.clone()];
    if !target.starts_with("yantrik_") {
        targets.push(format!("yantrik_{target}"));
    }
    if let Some(own) = crate_target {
        if !targets.iter().any(|t| t == own) {
            targets.push(own.to_string());
        }
    }
    targets.into_iter().map(|t| format!("{t}=info")).collect()
}

/// The crate a type is defined in, as its log target: `yantrik_container_manager` for
/// `yantrik_container_manager::Containers`.
pub(crate) fn crate_of<T>() -> &'static str {
    let name = std::any::type_name::<T>();
    name.split("::").next().unwrap_or(name)
}

/// The defaults whose target `RUST_LOG` does not name. A later directive for the same target
/// replaces an earlier one, so a default added after the environment's would undo
/// `RUST_LOG=yantrik_weather=debug`; the person's own setting wins.
fn unless_asked(defaults: Vec<String>, rust_log: &str) -> Vec<String> {
    let named = named_targets(rust_log);
    defaults
        .into_iter()
        .filter(|d| {
            d.split_once('=')
                .is_some_and(|(target, _)| !named.contains(&target))
        })
        .collect()
}

/// The targets `RUST_LOG` sets a level for, read as the filter reads it (tracing-subscriber's
/// `parse_lossy`): split on commas, nothing trimmed, and a piece that does not parse ignored, as
/// the filter ignores it. A bare target means its trace level, so it names the target; a bare
/// level names none. A span-scoped piece (`target[span]=level`) is kept by the filter apart
/// from the target's own level, so it does not name the target either.
fn named_targets(rust_log: &str) -> Vec<&str> {
    use tracing_subscriber::filter::{Directive, LevelFilter};
    rust_log
        .split(',')
        .filter(|piece| !piece.is_empty() && !piece.contains('['))
        .filter(|piece| piece.parse::<Directive>().is_ok())
        .filter(|piece| piece.parse::<LevelFilter>().is_err())
        .map(|piece| piece.split_once('=').map_or(piece, |(target, _)| target))
        .collect()
}

#[cfg(test)]
mod directive_tests {
    use super::{app_directives, unless_asked};

    #[test]
    fn an_apps_own_lines_are_shown_whether_it_is_named_by_crate_or_by_id() {
        assert_eq!(
            app_directives("yantrik-notes", None),
            vec!["yantrik_notes=info"]
        );
        assert_eq!(
            app_directives("weather", Some("yantrik_weather")),
            vec!["weather=info", "yantrik_weather=info"]
        );
        assert_eq!(
            app_directives("system-monitor", Some("yantrik_system_monitor")),
            vec!["system_monitor=info", "yantrik_system_monitor=info"]
        );
        // An id that is not the crate's suffix: only the crate the runner names matches.
        let containers = app_directives("containers", Some("yantrik_container_manager"));
        assert!(containers.contains(&"yantrik_container_manager=info".to_string()));
        for d in containers {
            assert!(
                d.parse::<tracing_subscriber::filter::Directive>().is_ok(),
                "{d}"
            );
        }
    }

    #[test]
    fn rust_log_wins_for_a_target_it_names() {
        let defaults = vec![
            "yantrik_weather=info".to_string(),
            "yantrik_app=info".to_string(),
        ];
        assert_eq!(unless_asked(defaults.clone(), ""), defaults);
        assert_eq!(
            unless_asked(defaults.clone(), "yantrik_weather=debug,hyper=warn"),
            vec!["yantrik_app=info"]
        );
        assert_eq!(
            unless_asked(defaults.clone(), "yantrik_weather"),
            vec!["yantrik_app=info"],
            "a bare target is its trace level"
        );
        assert_eq!(
            unless_asked(defaults.clone(), "debug"),
            defaults,
            "a bare level names no target"
        );
    }

    #[test]
    fn a_piece_the_filter_ignores_names_nothing() {
        let defaults = vec!["yantrik_weather=info".to_string()];
        // The filter ignores each of these, so the default must stand.
        for ignored in [
            "hyper=warn, yantrik_weather=debug",
            "yantrik_weather=debug ",
            "yantrik_weather=bogus",
        ] {
            assert_eq!(
                unless_asked(defaults.clone(), ignored),
                defaults,
                "{ignored:?}"
            );
        }
        // A span-scoped directive is kept apart from the target's own level.
        assert_eq!(
            unless_asked(defaults.clone(), "yantrik_weather[fetch]=debug"),
            defaults
        );
    }

    #[test]
    fn the_crate_of_an_app_is_where_its_type_lives() {
        struct Here;
        assert_eq!(super::crate_of::<Here>(), "yantrik_app");
    }
}

/// A tracing layer that remembers the most recent log lines of this process, for the
/// `log_tail` of a problem record. It formats nothing to a sink; `problems::note_log_line` holds
/// a bounded ring and that is all.
struct RecentLines;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RecentLines {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        struct Line(String);
        impl tracing::field::Visit for Line {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                if field.name() == "message" {
                    let _ = write!(self.0, "{value:?}");
                } else {
                    let _ = write!(self.0, " {}={:?}", field.name(), value);
                }
            }
        }
        let mut line = Line(String::new());
        event.record(&mut line);
        let meta = event.metadata();
        problems::note_log_line(&format!("{} {}: {}", meta.level(), meta.target(), line.0));
    }
}

/// The Wayland app id of an app's windows: the stem of its `.desktop` file, which is its program
/// (`yantrik-container-manager` for the app whose `App::ID` is `containers`).
///
/// Our windows told the compositor no app id at all, so labwc's rules and a taskbar knew a window
/// only by its title (#877). With one, a single labwc rule (`identifier="yantrik-*"`,
/// config/labwc/rc.xml) reaches every app's windows and none of anybody else's, which is how they
/// get the compositor's shadow, and a taskbar finds the app's name and icon by the id.
///
/// The program comes from the framework's list of its apps (`FRAMEWORK_APPS`), which a test in
/// `yantrik-shell-core` holds to the apps and their desktop files. An id that list does not know
/// (an app under test) is `yantrik-<id>`.
pub fn window_app_id(id: &str) -> String {
    yantrik_ipc_contracts::framework_apps::by_id(id)
        .map(|app| app.program.to_string())
        .unwrap_or_else(|| format!("yantrik-{id}"))
}

/// Give every window this process shows from here on its app id ([`window_app_id`]).
///
/// Called after a window is made and before it is shown, and that order matters both ways. Slint
/// keeps the id on its platform, and `set_xdg_app_id` refuses when there is none yet: it does not
/// start one, and the first component made is what does. Called before the first window, it
/// returned `NoPlatform` and every window went up with no app id (VM 542, 12 Nov 2026). And the
/// compositor is told the id when the surface is created, which Slint does at the first show.
///
/// A refusal is worth a warning: with no app id the compositor's rule for our windows does not
/// reach this one.
pub fn name_windows(id: &str) {
    if let Err(e) = slint::set_xdg_app_id(window_app_id(id)) {
        tracing::warn!(app = id, error = %e, "This app's windows have no app id");
    }
}

/// The software renderer when the session asks for it, chosen before any component exists. One
/// implementation of the five-line switch seven apps carried.
pub fn choose_backend() {
    let software = std::env::var("LIBGL_ALWAYS_SOFTWARE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let backend = std::env::var("SLINT_BACKEND").unwrap_or_default();
    if software && (backend.is_empty() || backend == "winit") {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }
}

#[cfg(test)]
mod app_id_tests {
    use super::window_app_id;
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Every `App::ID` under `apps/`, read from the source the way the shell's guard reads it.
    fn app_ids() -> Vec<String> {
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(root().join("apps")).expect("apps/") {
            let lib = entry.expect("entry").path().join("src/lib.rs");
            let Ok(text) = std::fs::read_to_string(&lib) else { continue };
            for line in text.lines() {
                if let Some(rest) = line.trim().strip_prefix("const ID: &'static str = \"") {
                    ids.push(rest.split('"').next().expect("closing quote").to_string());
                }
            }
        }
        ids.sort();
        ids
    }

    /// The id is the stem of the app's `.desktop` file, which is what a taskbar looks the app's
    /// name and icon up by (#877), for every app and not only the ones whose id is the program.
    #[test]
    fn every_apps_window_id_has_its_desktop_file() {
        let ids = app_ids();
        assert!(ids.len() >= 19, "found only {ids:?}");
        for id in ids {
            let app_id = window_app_id(&id);
            let file = root().join("apps/desktop-files").join(format!("{app_id}.desktop"));
            assert!(file.exists(), "`{id}` declares `{app_id}`, which has no {}", file.display());
        }
    }

    /// The one labwc rule that gives our windows their border and shadow, and that it reaches the
    /// id of every app: the pattern is the rule's glob, so a renamed id or a changed rule fails.
    #[test]
    fn the_labwc_rule_reaches_every_apps_window() {
        let rc = include_str!("../../../config/labwc/rc.xml");
        let rule = rc
            .split("<windowRule identifier=\"yantrik-*\">")
            .nth(1)
            .and_then(|after| after.split("</windowRule>").next())
            .expect("config/labwc/rc.xml has the yantrik-* window rule");
        assert!(rule.contains(r#"<action name="SetDecorations" decorations="border" forceSSD="yes" />"#), "{rule}");
        assert!(!rule.contains("serverDecoration"), "it places a window 27px low: {rule}");
        let prefix = "yantrik-";
        for id in app_ids() {
            assert!(window_app_id(&id).starts_with(prefix), "`{id}` is not matched by yantrik-*");
        }
    }

    /// The Mind desk's rule in the nested compositor names the id the framework gives the desk:
    /// the one place that sets it is `made_window`.
    #[test]
    fn the_mind_desks_rule_names_its_window_id() {
        let rc = include_str!("../../../config/labwc-mind/rc.xml");
        let id = window_app_id("mind-desk");
        assert_eq!(id, "yantrik-mind-desk");
        assert!(rc.contains(&format!(r#"<windowRule identifier="{id}""#)), "labwc-mind/rc.xml");
    }
}
