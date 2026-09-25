// Yantrik UI Kit — reusable Slint components.
// Consuming crates access .slint files via DEP_YANTRIK_UI_KIT_SLINT_PATH env var.
//
// Components: AppWindow, AppHeader, YButton, YIconButton, YInput, YDialog, YTabs,
// YSidebar, YListItem, YContextMenu, YEmptyState, LineChart, RadarChart,
// ToastBanner, MessageBubble, Icon.
//
// AppWindow is the window every app is (#256): it draws AppHeader as the app's one title bar.
// AppHeader is the one every screen is required to use; see docs/app-sdk.md. YToolbar and
// AppShell used to sit here and were instantiated by nothing, because each spent Slint's
// single `@children` on a region no app needed. They are gone.

#[cfg(test)]
mod app_header_is_mandatory {
    use std::path::{Path, PathBuf};

    /// Every app we ship, and the screen component that draws it.
    ///
    /// An app binary is a thin `Window` around one of these, so this is where an app's top edge
    /// is decided. Adding an app means adding a line here; that is the point.
    const APP_SCREENS: &[(&str, &str)] = &[
        ("calendar", "calendar.slint"),
        ("container-manager", "container_manager.slint"),
        ("document-editor", "document_editor.slint"),
        ("download-manager", "download_manager.slint"),
        ("email", "email.slint"),
        ("image-viewer", "image_viewer.slint"),
        ("music-player", "music_player.slint"),
        ("network-manager", "network_manager.slint"),
        ("notes", "notes_editor.slint"),
        ("presentation", "presentation.slint"),
        ("snippet-manager", "snippet_manager.slint"),
        ("spreadsheet", "spreadsheet.slint"),
        ("system-monitor", "system_monitor.slint"),
        ("terminal", "terminal.slint"),
        ("weather", "weather.slint"),
    ];

    fn ui_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yantrik-ui-slint/ui")
            .canonicalize()
            .expect("the shell's ui directory sits beside the kit")
    }

    /// The check that makes the header consistent by construction rather than by agreement.
    ///
    /// Sixteen apps once drew their own top bar and the heights came out 28, 32, 36, 44, 48, 56
    /// and 64 — nobody chose that, it is just what happens when sixteen files each decide. A
    /// screen that instantiates AppHeader cannot choose: the height, the padding, the glyph tile
    /// and the button size all belong to the component, and the app supplies only data.
    ///
    /// So this asserts the one thing that cannot be re-derived from the markup: that the app
    /// went through the frame at all.
    #[test]
    fn every_app_we_ship_draws_its_header_with_the_shared_component() {
        let dir = ui_dir();
        let mut missing = Vec::new();

        for (app, screen) in APP_SCREENS {
            let path = dir.join(screen);
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{app}: cannot read {}: {e}", path.display()));

            if !src.contains("AppHeader {") {
                missing.push(format!("{app} ({screen}) draws its own header"));
            }
        }

        assert!(
            missing.is_empty(),
            "these apps do not use the shared header:\n  {}\n\n\
             Use AppHeader from the UI kit. Header content is data — `title`, `icon`, \
             `subtitle` and an `actions` model of AppAction — so that every app's top edge is \
             identical without anyone having to remember a number. See docs/app-sdk.md.",
            missing.join("\n  ")
        );
    }

    /// The header's height is a token, not a literal an app can retype.
    ///
    /// `h-app-header` exists so there is exactly one answer to "how tall is an app's top edge".
    /// If AppHeader ever hardcodes a number instead, the token stops meaning anything and the
    /// drift can start again from one file.
    #[test]
    fn the_header_takes_its_height_from_the_token() {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("slint/app_header.slint"),
        )
        .expect("app_header.slint is part of this crate");

        assert!(
            src.contains("height: Theme.h-app-header;"),
            "AppHeader must take its height from Theme.h-app-header, so the one number that \
             decides how tall an app's top edge is lives in the design tokens"
        );
    }

    /// The components that got us here are not allowed to come back.
    ///
    /// YToolbar and AppShell were each instantiated by nothing for the same reason: a Slint
    /// component has one `@children`, both spent it on a region no app needed, and so every app
    /// went and drew its own header instead. A half-usable shared component is worse than none —
    /// it looks like the problem is solved.
    #[test]
    fn the_shells_that_nobody_could_use_are_still_gone() {
        let kit = Path::new(env!("CARGO_MANIFEST_DIR")).join("slint");
        for dead in ["y_toolbar.slint", "app_shell.slint"] {
            assert!(
                !kit.join(dead).exists(),
                "{dead} is back. If a shared component cannot express what every app needs \
                 (for a header: trailing actions), apps will bypass it and hand-roll their own. \
                 Pass the content as data instead of as children — see AppHeader."
            );
        }
    }
}

// ── What a window costs while nobody is looking at it ───────────────────────
//
// An open Terminal showing nothing but a prompt burned about one and a half cores, for hours,
// on a desktop where nobody had typed anything (#29, and most of #53's idle total). Nothing was
// running in the shell; the screen held `$ ` and a cursor. The cause was four lines of markup —
// the cursor rectangle carried
//
//     animate blink-opacity { duration: 600ms; iteration-count: -1; }
//
// and an animation that never finishes is a window that never stops redrawing. Put back and
// measured from /proc/<pid>/stat deltas, that one property costs 110% of a core at an empty
// prompt; the same binary without it spends zero jiffies in thirty seconds. The compositor pays
// again on top of that, which is why labwc sat at 27% beside it.
//
// No functional test sees this. Every screenshot is right, every key works, every action
// answers — the app is simply expensive. So the check has to read the markup, and it has to
// cover every window rather than the one that was caught, because the mistake is a single
// property that anyone can type again.
//
// The first version of this check read only `apps/*/ui/app.slint`, and the shell walked straight
// through the hole that left. #68: with the Intent Lens CLOSED and a mind thinking on a
// delegated job, `yantrik-ui` sat at 83% of a core for minutes at a time — 75.4% of it on the UI
// thread, with labwc paying 20% more to composite what it produced. The whole of that came from
// one 6px dot on the status bar, which pulses while a mind is working:
//
//     opacity: root.companion-status == "thinking" ? ai-pulse-opacity : 1.0;
//     animate ai-pulse-opacity { duration: 2000ms; }
//     Timer { interval: 2000ms; triggered => { ai-pulse-opacity = … > 0.8 ? 0.5 : 1.0; } }
//
// The timer restarted the animation exactly as often as the animation lasted, so it was never
// not animating, and a window with an animation in flight redraws at the display's rate. On the
// shell that is a full-screen repaint — 96ms of CPU on the software rasteriser — for a dot.
// Measured headless on this markup with the Lens closed: 236 frames a second before, 5 after.
//
// The refusals, in the order a window gets expensive:
//
//   * an `animate` with a negative iteration count — it never finishes;
//   * a `Timer` that is always running, including one that names no `running` at all, since
//     Slint's default is true;
//   * a `Timer` that restarts an `animate` whose duration is as long as the gap between ticks —
//     the dot above;
//   * a `Timer` that repeats faster than 100ms. Below that is the display's frame rate, which
//     means a full repaint of whatever is on screen, and a file scan cannot prove the thing
//     being animated is visible. So the rate is what gets held to: a pulse, a progress bar and a
//     sweep all read fine at 100–250ms, and 60Hz is left to the ambient decoration, which takes
//     its interval from the renderer's own measured budget and is switched off entirely where a
//     frame is expensive (`ambient_interval_ms` in crates/yantrik-ui/src/render_backend.rs). A
//     timer that stops itself on its first tick is not repeating at all, and is allowed.
#[cfg(test)]
mod an_idle_window_stops_drawing {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the kit sits two levels under the checkout")
    }

    /// The directories every app's `build.rs` hands to the Slint compiler, in its search order.
    ///
    /// Two of these hold files of the same name (`app_header.slint` lives in the kit, and
    /// `components/app_header.slint` in the shell's ui directory), so the order is what decides
    /// which file an app is really compiling. It is the order the `with_include_paths` call in
    /// every `apps/*/build.rs` uses.
    fn include_paths(repo: &Path) -> Vec<PathBuf> {
        vec![
            repo.join("crates/yantrik-design-tokens/slint"),
            repo.join("crates/yantrik-ui-kit/slint"),
            repo.join("crates/yantrik-ui-slint/ui"),
        ]
    }

    /// Every `.slint` file one app window compiles: its own `ui/app.slint` and, through the
    /// import graph, the screens and components that file reaches.
    ///
    /// Following the imports is the point. The animation that cost #29 its cores was not in the
    /// terminal's own `app.slint` — it was in a shared screen the terminal imported, and a check
    /// that only read the file named after the app would have walked straight past it.
    fn markup_reachable_from(entry: &Path, includes: &[PathBuf]) -> BTreeSet<PathBuf> {
        let mut seen = BTreeSet::new();
        let mut queue = vec![entry.to_path_buf()];
        while let Some(file) = queue.pop() {
            if !seen.insert(file.clone()) {
                continue;
            }
            let source = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
            for spec in imported_paths(&source) {
                // Slint's own widget library, which has no file in this checkout.
                if spec == "std-widgets.slint" {
                    continue;
                }
                let own = file.parent().expect("a file has a directory").to_path_buf();
                let resolved = std::iter::once(own)
                    .chain(includes.iter().cloned())
                    .map(|base| base.join(&spec))
                    .find(|candidate| candidate.is_file())
                    .unwrap_or_else(|| {
                        panic!(
                            "{} imports {spec}, which is in none of the include paths",
                            file.display()
                        )
                    });
                queue.push(resolved);
            }
        }
        seen
    }

    /// The `.slint` paths one file imports: everything inside the quotes of `from "…"`.
    fn imported_paths(source: &str) -> Vec<String> {
        source
            .split("from \"")
            .skip(1)
            .filter_map(|rest| rest.split_once('"'))
            .map(|(spec, _)| spec.to_string())
            .filter(|spec| spec.ends_with(".slint"))
            .collect()
    }

    /// The source with `//` comments removed, so a commented-out animation is not a finding.
    ///
    /// Line comments only, and applied to the copy this check reads rather than to the one the
    /// imports are resolved from. A `//` inside a string literal would take the rest of that
    /// line with it; no shipped screen has one, and the cost would be a missed finding on one
    /// line rather than a false one.
    fn without_comments(source: &str) -> String {
        source
            .lines()
            .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The fastest a `Timer` may repeat. Below this it is asking for frames at the display's rate.
    const PULSE_FLOOR_MS: f64 = 100.0;

    /// What a `Timer` body gives one of its properties — `running`, `interval` — if it sets it.
    ///
    /// Read by hand rather than with a plain search for the name, because a Timer's `triggered =>`
    /// handler is part of the same body and may well mention something that merely ENDS in the
    /// name (`root.ambient-interval-ms` contains `interval`; a handler may touch `is-running`).
    /// Both neighbours are checked: the character before must not be part of a word, and what
    /// follows must be the `:` that makes it a property and not a longer identifier.
    fn setting(body: &str, name: &str) -> Option<String> {
        let mut at = 0;
        while let Some(offset) = body[at..].find(name).map(|i| at + i) {
            at = offset + name.len();
            let before = body[..offset].chars().next_back();
            let own_word = !before.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_');
            if own_word {
                if let Some(value) = body[at..].trim_start().strip_prefix(':') {
                    return Some(value.split(';').next().unwrap_or("").trim().to_string());
                }
            }
        }
        None
    }

    /// The shortest interval an expression can produce, in milliseconds.
    ///
    /// Slint writes an interval either as a duration literal — `160ms` — or as a number of
    /// milliseconds scaled into one, which is how the ambient layers take the renderer's budget:
    ///
    ///     interval: max(100, root.ambient-interval-ms) * 1ms;
    ///
    /// Both shapes leave the millisecond values in the expression as plain numbers; the `1ms`
    /// that does the scaling is a unit and not one of them, so it is dropped. The `max` is what
    /// makes the floor readable at all, which is the reason to write it that way.
    ///
    /// `None` means the expression holds no number: nothing in it says how fast this can go, and
    /// the caller treats that as a finding rather than as permission.
    fn fastest_interval_ms(expression: &str) -> Option<f64> {
        let bytes = expression.as_bytes();
        let mut fastest: Option<f64> = None;
        let mut at = 0;
        while at < bytes.len() {
            if !bytes[at].is_ascii_digit() {
                at += 1;
                continue;
            }
            // A digit inside an identifier (`phase2`) is not a number.
            let inside_a_name = expression[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphabetic() || c == '_');
            let start = at;
            while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
                at += 1;
            }
            if inside_a_name {
                continue;
            }
            let Ok(value) = expression[start..at].parse::<f64>() else {
                continue;
            };
            // `* 1ms` is the unit the bare numbers are counted in, not an interval of one
            // millisecond.
            if expression[at..].starts_with("ms") && value == 1.0 {
                continue;
            }
            fastest = Some(fastest.map_or(value, |seen: f64| seen.min(value)));
        }
        fastest
    }

    /// The properties a `Timer`'s handler writes to.
    ///
    /// The name only, with any `root.` / `parent.` / `self.` in front of it dropped, because that
    /// is how the `animate` for it will be spelled. Comparisons (`==`, `!=`, `<=`, `>=`) and
    /// Slint's callback arrow (`=>`) are not assignments; `+=` and friends are.
    fn properties_assigned(body: &str) -> BTreeSet<String> {
        let bytes = body.as_bytes();
        let mut found = BTreeSet::new();
        for (at, byte) in bytes.iter().enumerate() {
            if *byte != b'=' {
                continue;
            }
            let previous = at.checked_sub(1).map(|i| bytes[i]).unwrap_or(b' ');
            let next = bytes.get(at + 1).copied().unwrap_or(b' ');
            if next == b'=' || next == b'>' || matches!(previous, b'=' | b'!' | b'<' | b'>') {
                continue;
            }
            let left = body[..at].trim_end_matches(['+', '-', '*', '/']).trim_end();
            let starts = left
                .rfind(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
                .map_or(0, |cut| cut + 1);
            let name = left[starts..].rsplit('.').next().unwrap_or("");
            if !name.is_empty() {
                found.insert(name.to_string());
            }
        }
        found
    }

    /// Every `animate` block in a file: the properties it covers, and how long it says it runs.
    ///
    /// `None` for a duration that is not a plain number (`Theme.dur-normal`) — this cannot follow
    /// a token into another file, and a missed finding is better than an invented one.
    fn animations(source: &str) -> Vec<(BTreeSet<String>, Option<f64>)> {
        let mut found = Vec::new();
        let mut at = 0;
        while let Some(offset) = source[at..].find("animate ").map(|i| at + i) {
            at = offset + "animate ".len();
            let Some(open) = source[at..].find('{').map(|i| at + i) else {
                break;
            };
            let names = source[at..open]
                .split(',')
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect::<BTreeSet<_>>();
            let close = source[open..].find('}').map_or(source.len(), |i| open + i);
            let duration = setting(&source[open..close], "duration")
                .and_then(|value| fastest_interval_ms(&value));
            found.push((names, duration));
            at = close;
        }
        found
    }

    /// Whether a `Timer` turns itself off on its first tick.
    ///
    /// One of these is not an animation and costs one frame: the Lens uses a 30ms one to put the
    /// cursor in the reply box a tick after the panel is laid out, because a field cannot take
    /// focus before it has a size. It is recognised by the handler clearing the very property the
    /// `running` gate reads.
    fn stops_itself(gate: &str, body: &str) -> bool {
        let property = gate.rsplit('.').next().unwrap_or(gate).trim();
        let is_a_name = !property.is_empty()
            && property
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
        is_a_name && body.contains(&format!("{property} = false"))
    }

    /// The body of every `Timer { … }` in one file, with the line it starts on.
    fn timer_bodies(source: &str) -> Vec<(usize, String)> {
        let bytes = source.as_bytes();
        let mut found = Vec::new();
        let mut at = 0;
        while let Some(offset) = source[at..].find("Timer").map(|i| at + i) {
            at = offset + "Timer".len();
            let open = source[at..]
                .find(|c: char| !c.is_whitespace())
                .map(|i| at + i)
                .filter(|&i| bytes[i] == b'{');
            let Some(open) = open else { continue };
            let mut depth = 1usize;
            let mut end = open + 1;
            while end < bytes.len() && depth > 0 {
                match bytes[end] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                end += 1;
            }
            let close = end.saturating_sub(1).max(open + 1);
            found.push((
                source[..offset].matches('\n').count() + 1,
                source[open + 1..close].to_string(),
            ));
            at = end;
        }
        found
    }

    /// Every window this checkout ships: each app, and the shell itself.
    ///
    /// The shell is the one that matters most and was the one left out. It is a single window
    /// holding the desktop, the Lens, the lock screen and the login screen, it is up from boot to
    /// shutdown, and what it spends is what the machine feels like — 83% of a core while a mind
    /// was thinking behind a closed panel (#68).
    fn windows(repo: &Path) -> Vec<(String, PathBuf)> {
        let mut found = vec![(
            "shell".to_string(),
            repo.join("crates/yantrik-ui-slint/ui/app.slint"),
        )];
        let mut apps: Vec<PathBuf> = std::fs::read_dir(repo.join("apps"))
            .expect("the apps we ship live in apps/")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|dir| dir.join("ui/app.slint").is_file())
            .collect();
        apps.sort();
        found.extend(apps.into_iter().map(|app| {
            let name = app
                .file_name()
                .expect("an app has a directory name")
                .to_string_lossy()
                .into_owned();
            (name, app.join("ui/app.slint"))
        }));
        found
    }

    #[test]
    fn no_window_asks_to_be_redrawn_for_ever() {
        let repo = repo();
        let includes = include_paths(&repo);
        let windows = windows(&repo);
        // A path mistake here would make the whole check pass by reading nothing, so it says out
        // loud what it found: the shell, and the app this came from by name.
        assert!(
            windows.iter().any(|(name, _)| name == "terminal")
                && windows
                    .iter()
                    .any(|(name, entry)| name == "shell" && entry.is_file()),
            "the windows this check reads are the shell's own ui/app.slint and every app's; \
             under {} it found {} of them and not the pair it expects",
            repo.display(),
            windows.len()
        );

        let mut never_still = Vec::new();
        for (name, entry) in &windows {
            for file in markup_reachable_from(entry, &includes) {
                let source = without_comments(
                    &std::fs::read_to_string(&file)
                        .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display())),
                );
                let shown = file.strip_prefix(&repo).unwrap_or(&file).display().to_string();
                let where_ = |line: usize| format!("{name}: {shown}:{line}");
                for (number, line) in source.lines().enumerate() {
                    let Some((_, count)) = line.split_once("iteration-count") else {
                        continue;
                    };
                    let count = count
                        .trim_start()
                        .trim_start_matches(':')
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim();
                    if count.starts_with('-') {
                        never_still.push(format!(
                            "{} — animate … {{ iteration-count: {count} }}",
                            where_(number + 1)
                        ));
                    }
                }
                for (line, body) in timer_bodies(&source) {
                    let Some(gate) = setting(&body, "running") else {
                        // Slint's Timer runs by default, so saying nothing says true.
                        never_still.push(format!(
                            "{} — Timer with no `running`, which defaults to true",
                            where_(line)
                        ));
                        continue;
                    };
                    if gate == "true" {
                        never_still
                            .push(format!("{} — Timer {{ running: true }}", where_(line)));
                        continue;
                    }
                    // Gated on state, so it ticks only while something is happening. How often
                    // does it ask for a frame while that something is going on?
                    let interval = setting(&body, "interval").unwrap_or_default();
                    let every = fastest_interval_ms(&interval);
                    // One tick and it is done; the rate of a thing that happens once is nothing,
                    // and an animation it starts gets to finish.
                    if stops_itself(&gate, &body) {
                        continue;
                    }
                    // An animation this timer restarts before it has finished never finishes, and
                    // an animation in flight is a window at the display's frame rate. This is what
                    // the status bar's thinking dot was, and it is the one that actually cost #68
                    // its core: a 2000ms `animate` on the opacity, and a 2000ms Timer flipping
                    // that same opacity, so the shell redrew continuously for as long as a mind
                    // was working — with the Lens closed, with nothing else on screen moving.
                    for property in properties_assigned(&body) {
                        for (animated, duration) in animations(&source) {
                            let (Some(every), Some(duration)) = (every, duration) else {
                                continue;
                            };
                            if animated.contains(&property) && duration >= every {
                                never_still.push(format!(
                                    "{} — Timer {{ interval: {every}ms }} restarts `animate \
                                     {property} {{ duration: {duration}ms }}`, which therefore \
                                     never finishes",
                                    where_(line)
                                ));
                            }
                        }
                    }
                    // The ambient decoration — orb, particle field, the breathing lock screen —
                    // takes both its rate and its on/off from the renderer's measured budget,
                    // which is 0 wherever a frame is expensive. That is the one 60Hz in this
                    // tree that was paid for on purpose; see render_backend.rs.
                    if interval.contains("ambient-interval-ms")
                        && gate.contains("ambient-interval-ms")
                    {
                        continue;
                    }
                    match every {
                        Some(ms) if ms >= PULSE_FLOOR_MS => {}
                        Some(ms) => never_still.push(format!(
                            "{} — Timer {{ interval: {ms}ms; running: {gate} }}",
                            where_(line)
                        )),
                        None => never_still.push(format!(
                            "{} — Timer whose interval says no number, so how fast it repeats \
                             cannot be read here: `{interval}`",
                            where_(line)
                        )),
                    }
                }
            }
        }
        never_still.sort();
        never_still.dedup();

        assert!(
            never_still.is_empty(),
            "these windows ask to be redrawn far more often than anything on them changes, so \
             they cost CPU while nobody is looking at them:\n  {}\n\n\
             Each of these holds the window at the display's frame rate: an animation with a \
             negative iteration count; a Timer that is always running; a Timer that repeats \
             faster than {PULSE_FLOOR_MS}ms, for as long as its condition holds — and a condition \
             like `is-thinking` holds for minutes; and a Timer that restarts an animation before \
             that animation can finish, which is the same thing said in two places. The window \
             pays and the compositor pays again to composite every frame: 110% of a core for one \
             such animation on the terminal (#29), and 236 frames a second — 75% of the shell's \
             main thread, 20% more in labwc — while a mind was thinking and nothing on screen was \
             moving but a 6px dot (#68).\n\n\
             Drive the effect from state and at the rate the effect needs. Gate the Timer on \
             everything that has to be true for the thing to be ON SCREEN, not just on the state \
             it reports (`running: root.is-open && root.is-thinking`), and give a pulse, a \
             shimmer or a progress bar 100-250ms a step — nobody can see the difference, and it \
             is a tenth of the frames. An animation has to be shorter than the gap between the \
             ticks that start it, or it never ends. Ambient decoration is the exception, and \
             takes its rate and its on/off from the renderer's budget (`ambient-interval-ms`), \
             which is zero where a frame is expensive. A window that is doing nothing has to ask \
             for no frames at all — apps/terminal's \
             `real_window_shell_tabs_search_clipboard_resize_and_idle` asserts exactly that for \
             one app, by counting the frames the renderer requests.",
            never_still.join("\n  ")
        );
    }
}

#[cfg(test)]
mod every_app_is_an_app_window {
    use std::path::Path;

    /// The apps still drawn inside a plain `Window`, with labwc's title bar over their own
    /// header. This list only ever gets shorter: an app converted to `AppWindow` has to come off
    /// it, and a new app cannot go on it.
    const NOT_YET: &[&str] = &[];

    /// Every app's window is the base app window (#256).
    ///
    /// The ask was one base shell that every app extends, the way every app on macOS is an
    /// NSWindow and every GNOME app an AdwApplicationWindow: the same bar, the same buttons, the
    /// same drag and double-click, because there is only one place any of it is written. An app
    /// that inherits a bare `Window` instead gets labwc's bar over its own header — the two bars
    /// this replaced.
    #[test]
    fn every_app_inherits_the_base_app_window() {
        let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
        let mut wrong = Vec::new();
        let mut seen = 0;
        for entry in std::fs::read_dir(&apps).expect("the apps directory") {
            let dir = entry.expect("an app directory").path();
            let ui = dir.join("ui/app.slint");
            let Ok(src) = std::fs::read_to_string(&ui) else { continue };
            let app = dir.file_name().unwrap().to_string_lossy().to_string();
            seen += 1;
            let base_window = src.contains(" inherits AppWindow {");
            let plain_window = src.contains(" inherits Window {");
            // A bar nobody connected draws its buttons and does nothing when they are pressed.
            let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap_or_default();
            if base_window && !main.contains("window_chrome!(") {
                wrong.push(format!(
                    "{app}: inherits AppWindow, but main never calls window_chrome!, so its \
                     minimise, maximise, close and drag do nothing"
                ));
            }
            match (NOT_YET.contains(&app.as_str()), base_window, plain_window) {
                (false, true, _) | (true, false, true) => {}
                (false, false, _) => wrong.push(format!("{app}: its window does not inherit AppWindow")),
                (true, true, _) => wrong.push(format!(
                    "{app}: inherits AppWindow now, so take it off NOT_YET"
                )),
                (true, false, false) => wrong.push(format!(
                    "{app}: is on NOT_YET but its window inherits neither AppWindow nor Window"
                )),
            }
        }
        assert!(seen >= 10, "read only {seen} apps from {}; the path is wrong", apps.display());
        assert!(
            wrong.is_empty(),
            "every app's window is the base app window:\n  {}\n\n\
             `export component MyApp inherits AppWindow`, with the bar as data (`app-id`, \
             `header-icon`, `header-actions`, `header-action(id)`), the app's own content as the \
             children, `export {{ WindowChrome }} from \"app_window.slint\";` beside the import, \
             and `yantrik_app_runtime::window_chrome!(ui)` once in main. See \
             crates/yantrik-ui-kit/slint/app_window.slint.",
            wrong.join("\n  ")
        );
    }
}

#[cfg(test)]
mod a_test_crate_runs_when_ci_runs {
    use std::path::{Path, PathBuf};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the kit sits two levels under the checkout")
    }

    /// The paths the root workspace names in its `members` list, one per line as written there.
    fn root_members() -> Vec<String> {
        let text = std::fs::read_to_string(repo().join("Cargo.toml")).expect("the root Cargo.toml");
        let mut members = Vec::new();
        let mut inside = false;
        for line in text.lines() {
            let line = line.trim_start();
            if !inside {
                inside = line.starts_with("members = [");
                continue;
            }
            if line.starts_with(']') {
                break;
            }
            if let Some(rest) = line.strip_prefix('"') {
                if let Some(path) = rest.split('"').next() {
                    members.push(path.to_string());
                }
            }
        }
        members
    }

    /// Every `tests/*-core` Rust crate is a member of the root workspace (#85).
    ///
    /// CI's only cargo test step is `cargo test --workspace --locked`. The crates under `tests/`
    /// carried a `[workspace]` table of their own, which makes each one a workspace root that
    /// `--workspace` never reaches: tests/document-core alone hid 66 tests behind it, sixteen
    /// written after #84 found the gap and nobody ran them. #85's own lesson is that the defect
    /// recreates itself — a new core crate written the same way would vanish the same way, and
    /// the absence of a check was the only reason it took an issue to notice.
    ///
    /// `tests/blender-core` answers to the name but holds Python, no crate; the shell-scripts
    /// job runs it, so a directory without a `Cargo.toml` is not this check's to make.
    #[test]
    fn every_core_test_crate_is_a_workspace_member() {
        let members = root_members();
        let mut unseen = Vec::new();
        let mut seen = 0;
        for entry in std::fs::read_dir(repo().join("tests")).expect("the tests directory") {
            let dir = entry.expect("a tests entry").path();
            let Some(name) = dir.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            if !name.ends_with("-core") || !dir.join("Cargo.toml").is_file() {
                continue;
            }
            seen += 1;
            let rel = format!("tests/{name}");
            if !members.iter().any(|m| m == &rel) {
                unseen.push(format!("{rel}: not listed in the root Cargo.toml's members"));
            }
            let text = std::fs::read_to_string(dir.join("Cargo.toml")).expect("a readable Cargo.toml");
            if text
                .lines()
                .any(|l| l.trim_start() == "[workspace]" || l.trim_start().starts_with("[workspace."))
            {
                unseen.push(format!(
                    "{rel}: declares its own [workspace] table, so it is a workspace root no \
                     `--workspace` command reaches"
                ));
            }
        }
        assert!(seen >= 10, "read only {seen} core test crates from tests/; the path is wrong");
        assert!(
            unseen.is_empty(),
            "these test crates never run in CI:\n  {}\n\n\
             A guard test nobody runs is a comment, and this repository's cargo CI runs exactly \
             one command: `cargo test --workspace --locked` (.github/workflows/ci.yml). A crate \
             outside the root workspace is invisible to it — its tests run only when somebody \
             remembers to cd there (#85). Put the directory in the root Cargo.toml's members and \
             delete the crate's `[workspace]` line and its Cargo.lock; then run it from the root \
             once, because a crate nobody tested in years may not compile.",
            unseen.join("\n  ")
        );
    }
}
