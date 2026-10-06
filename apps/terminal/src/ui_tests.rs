//! Exercise the real window, callbacks and PTYs with Slint's native software
//! renderer. Only the window-system event queue and clipboard are substituted.
use super::*;
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Clipboard, EventLoopProxy, Platform, WindowAdapter, WindowEvent,
};
use std::{collections::VecDeque, sync::Mutex, time::Instant};
type Queue = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;
struct Proxy(Queue);
impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), slint::EventLoopError> {
        self.0.lock().unwrap().push_back(event);
        Ok(())
    }
}
struct NativeTest {
    window: Rc<MinimalSoftwareWindow>,
    queue: Queue,
    clipboard: Arc<Mutex<String>>,
}
impl Platform for NativeTest {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.queue.clone())))
    }
    fn set_clipboard_text(&self, text: &str, _: Clipboard) {
        *self.clipboard.lock().unwrap() = text.into();
    }
    fn clipboard_text(&self, _: Clipboard) -> Option<String> {
        Some(self.clipboard.lock().unwrap().clone())
    }
}
fn key(window: &MinimalSoftwareWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}
fn tick(queue: &Queue, window: &MinimalSoftwareWindow) -> bool {
    // Drop the queue lock before callbacks enqueue further events.
    let events: Vec<_> = queue.lock().unwrap().drain(..).collect();
    for event in events {
        event();
    }
    slint::platform::update_timers_and_animations();
    let size = window.size();
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.width, size.height);
    window.draw_if_needed(|renderer| {
        renderer.render(buffer.make_mut_slice(), size.width as usize);
    })
}
fn wait(queue: &Queue, window: &MinimalSoftwareWindow, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !condition() {
        tick(queue, window);
        assert!(
            Instant::now() < deadline,
            "Native Terminal condition timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    tick(queue, window);
}
/// Directory the UI screenshots are written to.
///
/// This used to be `CARGO_MANIFEST_DIR/../../target`, which names the build
/// directory only when the build directory is the default one inside the
/// checkout. CI builds this tree from a git worktree against a shared target
/// directory outside it, so that path pointed at a directory that does not
/// exist, and `File::create(..).unwrap()` panicked with ENOENT before the test
/// had asserted anything. It read like a missing display; it is not one. The
/// window below is a `MinimalSoftwareWindow` behind a substituted
/// `slint::platform::Platform` and never touches X11 or Wayland, so this test
/// runs headless — it just has to put its PNGs somewhere that exists.
///
/// The test binary lives in `<target>/<profile>/deps/`, so its own path names
/// the real build directory wherever cargo put it.
fn shot_dir() -> std::path::PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn save(window: &MinimalSoftwareWindow, name: &str) {
    let size = window.size();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.width, size.height);
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(pixels.make_mut_slice(), size.width as usize);
    });
    let path = shot_dir().join(name);
    let mut png = png::Encoder::new(
        std::fs::File::create(path).unwrap(),
        size.width,
        size.height,
    );
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(pixels.as_bytes())
        .unwrap();
}
#[test]
fn real_window_shell_tabs_search_clipboard_resize_and_idle() {
    std::env::set_var("SHELL", "/bin/sh");
    std::env::set_var("PS1", "yantrik $ ");
    let dir = std::env::temp_dir().join(format!("yantrik-terminal-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("HOME", &dir);
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let queue = Queue::default();
    let clipboard = Arc::new(Mutex::new(String::new()));
    slint::platform::set_platform(Box::new(NativeTest {
        window: window.clone(),
        queue: queue.clone(),
        clipboard: clipboard.clone(),
    }))
    .unwrap();
    let ui = TerminalApp::new().unwrap();
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(1000, 680));
    let state = wire(&ui, false);
    ui.invoke_focus_terminal();
    wait(&queue, &window, || {
        ui.get_screen_text().contains("yantrik $")
    });
    key(
        &window,
        "export YANTRIK_UI_TEST=kept; printf 'SESSION:%s\\n' \"$YANTRIK_UI_TEST\"",
    );
    key(&window, "\n");
    wait(&queue, &window, || {
        ui.get_screen_text().contains("SESSION:kept")
    });
    assert_eq!(state.borrow().tabs.len(), 1);
    assert_eq!(state.borrow().session().unwrap().cwd(), dir);
    let folder = dir.join("folder with spaces; literal");
    std::fs::create_dir(&folder).unwrap();
    state.borrow_mut().new_tab_at(&ui, &folder).unwrap();
    assert_eq!(state.borrow().session().unwrap().cwd(), folder);
    assert_eq!(state.borrow().tabs[0].session.cwd(), dir);
    assert!(state
        .borrow_mut()
        .new_tab_at(&ui, &dir.join("missing"))
        .is_err());
    assert_eq!(state.borrow().tabs.len(), 2);
    state.borrow_mut().close(1, &ui, true);
    assert_eq!(state.borrow().active, 0);
    window.dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Control.into(),
    });
    window.dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Shift.into(),
    });
    key(&window, "t");
    window.dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Shift.into(),
    });
    window.dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Control.into(),
    });
    assert_eq!(state.borrow().tabs.len(), 2);
    assert_eq!(state.borrow().active, 1);
    wait(&queue, &window, || {
        ui.get_screen_text().contains("yantrik $")
    });
    assert!(!ui.get_screen_text().contains("SESSION:kept"));
    ui.invoke_select_tab(0);
    ui.invoke_focus_terminal();
    tick(&queue, &window);
    assert!(ui.get_screen_text().contains("SESSION:kept"));
    ui.invoke_copy_screen();
    assert!(clipboard.lock().unwrap().contains("SESSION:kept"));
    *clipboard.lock().unwrap() = "printf 'PASTE:%s\\n' worked\n".into();
    ui.invoke_paste_clipboard();
    wait(&queue, &window, || {
        ui.get_screen_text().contains("PASTE:worked")
    });
    ui.set_show_search(true);
    tick(&queue, &window);
    key(&window, "SESSION:kept");
    wait(&queue, &window, || ui.get_match_count() > 0);
    assert!(ui.get_match_row() >= 0);
    save(&window, "terminal-search.png");
    let count = ui.get_match_count();
    state
        .borrow()
        .session()
        .unwrap()
        .write(b"printf 'SESSION:%s\\n' kept\r")
        .unwrap();
    wait(&queue, &window, || ui.get_match_count() > count);
    key(&window, slint::platform::Key::Escape);
    assert!(!ui.get_show_search());
    key(&window, "sleep 30");
    key(&window, "\n");
    wait(&queue, &window, || {
        state.borrow().session().unwrap().has_children()
    });
    ui.invoke_close_tab(0);
    assert!(ui.get_confirm_close());
    assert_eq!(state.borrow().tabs.len(), 2);
    tick(&queue, &window);
    save(&window, "terminal-close-confirmation.png");
    key(&window, slint::platform::Key::Escape);
    assert!(!ui.get_confirm_close());
    key(&window, "\x03");
    wait(&queue, &window, || {
        !state.borrow().session().unwrap().has_children()
    });
    key(
        &window,
        "printf '\\033[36mYantrik Terminal\\033[0m\\nPersistent shells. Quiet when idle.\\n'",
    );
    key(&window, "\n");
    wait(&queue, &window, || {
        ui.get_screen_text()
            .contains("Persistent shells. Quiet when idle.")
    });
    save(&window, "terminal-native.png");
    window.set_size(slint::PhysicalSize::new(640, 480));
    let old = state.borrow().session().unwrap().snapshot().size;
    wait(&queue, &window, || {
        state.borrow().session().unwrap().snapshot().size != old
    });
    save(&window, "terminal-native-compact.png");
    ui.global::<ThemeMode>().set_dark(false);
    tick(&queue, &window);
    save(&window, "terminal-native-light.png");
    ui.set_show_assistant(true);
    tick(&queue, &window);
    save(&window, "terminal-assistant-compact.png");
    ui.set_show_assistant(false);
    ui.global::<ThemeMode>().set_dark(true);
    // Settle finite theme/focus feedback. Count only renderer-requested frames.
    for _ in 0..35 {
        tick(&queue, &window);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut redraws = 0;
    for _ in 0..100 {
        if tick(&queue, &window) {
            redraws += 1;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        redraws, 0,
        "An idle real terminal session must not request redraws"
    );
    ui.invoke_action("new".into());
    while state.borrow().tabs.len() < MAX_TABS {
        ui.invoke_action("new".into());
    }
    assert_eq!(state.borrow().tabs.len(), MAX_TABS);
    wait(&queue, &window, || {
        ui.get_screen_text().contains("yantrik $")
    });
    save(&window, "terminal-max-tabs.png");
    key(&window, "exit 7\n");
    wait(&queue, &window, || !ui.get_alive());
    assert_eq!(state.borrow().session().unwrap().snapshot().exit, Some(7));
    ui.invoke_action("restart".into());
    wait(&queue, &window, || {
        ui.get_alive() && ui.get_screen_text().contains("yantrik $")
    });
    key(&window, "sleep 30\n");
    wait(&queue, &window, || {
        state.borrow().session().unwrap().has_children()
    });
    ui.invoke_close_tab((MAX_TABS - 1) as i32);
    assert!(ui.get_confirm_close());
    ui.invoke_action("confirm-close".into());
    assert_eq!(state.borrow().tabs.len(), MAX_TABS - 1);
    assert!(!ui.get_confirm_close());

    // ── What the mind is shown, and what it is told afterwards ──
    //
    // These run inside this test rather than as their own `#[test]` because a process may
    // install exactly one Slint platform — i-slint-core's `EVENTLOOP_PROXY` is a process-wide
    // `OnceCell` — and every check below needs a real window and a real PTY.
    let published = surface(&ui, &state);
    every_action_says_what_it_does(&published);
    the_terminal_answers_with_what_the_shell_did(&ui, &state, &published, &queue, &window, &dir);

    for tab in state.borrow_mut().tabs.drain(..) {
        tab.session.shutdown();
    }
    drop(state);
    drop(ui);
    std::env::set_current_dir(original_dir).unwrap();
    let _ = std::fs::remove_dir(dir);
}

#[test]
fn keyboard_encoding_preserves_shell_controls_and_application_mode() {
    use slint::platform::Key;
    use slint::private_unstable_api::re_exports::KeyEvent;
    let mut e = KeyEvent::default();
    e.text = "c".into();
    e.modifiers.control = true;
    assert_eq!(encode_key(&e, false).unwrap(), b"\x03");
    e.modifiers.control = false;
    e.text = Key::UpArrow.into();
    assert_eq!(encode_key(&e, false).unwrap(), b"\x1b[A");
    assert_eq!(encode_key(&e, true).unwrap(), b"\x1bOA");
    e.modifiers.control = true;
    assert_eq!(encode_key(&e, false).unwrap(), b"\x1b[1;5A");
    e.modifiers.control = false;
    e.text = Key::Tab.into();
    assert_eq!(encode_key(&e, false).unwrap(), b"\t");
    e.text = "日本語".into();
    assert_eq!(encode_key(&e, false).unwrap(), "日本語".as_bytes());
    e.text = Key::Backspace.into();
    assert_eq!(encode_key(&e, false).unwrap(), b"\x7f");
}

#[test]
fn the_tab_limit_is_the_constant_and_the_refusal_names_it() {
    assert_eq!(MAX_TABS, 32);
    assert_eq!(tabs_full(MAX_TABS - 1), None);
    let full = tabs_full(MAX_TABS).expect("the cap should refuse at MAX_TABS");
    assert!(
        full.contains(&MAX_TABS.to_string()),
        "the refusal does not name the real number: {full}"
    );
    assert!(full.contains("Close a tab"), "{full}");
}

#[test]
fn physical_modifiers_never_become_shell_control_bytes() {
    use slint::platform::Key;
    use slint::private_unstable_api::re_exports::KeyEvent;
    let mut event = KeyEvent::default();
    for key in [
        Key::Shift,
        Key::ShiftR,
        Key::Control,
        Key::ControlR,
        Key::Alt,
        Key::AltGr,
        Key::Meta,
        Key::MetaR,
        Key::CapsLock,
    ] {
        event.text = key.into();
        assert!(
            encode_key(&event, false).is_none(),
            "Modifier was forwarded: {key:?}"
        );
    }
    event.modifiers.control = true;
    event.text = "p".into();
    assert_eq!(encode_key(&event, false).unwrap(), b"\x10");
    event.text = "q".into();
    assert_eq!(encode_key(&event, false).unwrap(), b"\x11");
}

// ── The surface a mind reads ────────────────────────────────────────────────
//
// Called from the window test above, because only one Slint platform may exist per process.

/// Run one published action by name, the way `app.act` would.
fn act_on(
    published: &[(Action, Handler)],
    name: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (_, handler) = published
        .iter()
        .find(|(spec, _)| spec.name == name)
        .unwrap_or_else(|| panic!("the terminal does not publish `{name}`"));
    handler(&args)
}

/// Every action says what it does, documents its arguments, and is graded for what it can do.
fn every_action_says_what_it_does(published: &[(Action, Handler)]) {
    assert!(
        published.len() >= 4,
        "the surface lost actions: {} published",
        published.len()
    );
    for (spec, _) in published {
        assert_ne!(
            spec.description,
            format!("Terminal: {}", spec.name),
            "`{}` carries a placeholder description",
            spec.name
        );
        assert!(
            spec.description.len() >= 20,
            "`{}` has nothing a reader who cannot see the screen could use: {:?}",
            spec.name,
            spec.description
        );
        assert!(
            spec.description.split_whitespace().count() >= 5,
            "`{}` is not a sentence: {:?}",
            spec.name,
            spec.description
        );
        for p in &spec.params {
            assert!(
                p.description.len() >= 15,
                "`{}` takes `{}` and says nothing about what goes in it",
                spec.name,
                p.name
            );
            assert!(p.required, "`{}` must need its `{}`", spec.name, p.name);
        }
        // Everything here starts or drives a shell process on this machine.
        assert_eq!(
            spec.permission, "sensitive",
            "`{}` starts or feeds a shell and cannot be graded `{}`",
            spec.name, spec.permission
        );
    }
    // The two that only queue bytes on a PTY have to say the work is not finished when they
    // return, or the envelope around them reports `settled: true` for a command that has not
    // run — which is exactly what `run` did while answering `"completed": false` in its own body.
    for name in ["run", "send_input"] {
        let (spec, _) = published.iter().find(|(s, _)| s.name == name).unwrap();
        assert!(
            spec.deferred,
            "`{name}` only starts the work and must declare it"
        );
        assert_eq!(spec.schema()["settles"], "later");
    }
}

/// The answers report the shell, not the fact that a write was queued.
fn the_terminal_answers_with_what_the_shell_did(
    ui: &TerminalApp,
    state: &State,
    published: &[(Action, Handler)],
    queue: &Queue,
    window: &MinimalSoftwareWindow,
    dir: &std::path::Path,
) {
    let pid = state.borrow().session().unwrap().pid();
    let answer = act_on(
        published,
        "run",
        serde_json::json!({ "command": "printf 'SURFACE:%s\\n' ok" }),
    )
    .expect("run a command in the active shell");

    assert_eq!(answer["sent"], "printf 'SURFACE:%s\\n' ok", "answer: {answer}");
    assert_eq!(answer["shell_pid"], pid, "the answer names the shell it typed into: {answer}");
    assert_eq!(answer["alive"], true, "answer: {answer}");
    assert_eq!(answer["finished"], false, "a run never claims the command finished: {answer}");
    assert_eq!(
        answer["directory"],
        state.borrow().session().unwrap().cwd().display().to_string(),
        "answer: {answer}"
    );
    assert!(
        answer["screen_tail"].as_str().is_some_and(|t| !t.is_empty()),
        "the answer carries what the shell had printed: {answer}"
    );
    assert!(
        answer["output"].as_str().is_some_and(|o| o.contains("SURFACE")),
        "and what appeared after the command was sent: {answer}"
    );

    // And the command really ran — checked against the screen, not the answer.
    wait(queue, window, || ui.get_screen_text().contains("SURFACE:ok"));

    // Fault 3: the summary was the working directory and nothing else.
    let summary = view(ui, &state.borrow()).summary;
    assert!(
        summary.starts_with("Terminal — ") && summary.contains(&dir.display().to_string()),
        "the summary has to name the directory: {summary:?}"
    );
    assert!(
        summary.contains("idle at a prompt") || summary.contains("running:"),
        "and whether anything is running in it: {summary:?}"
    );
    assert!(
        summary.contains("tab "),
        "and which tab of how many: {summary:?}"
    );

    // An empty command used to be `command is empty`, with nothing on screen.
    let refusal = act_on(published, "run", serde_json::json!({ "command": "   " }))
        .expect_err("a blank command must be refused");
    assert!(
        refusal.contains("command"),
        "the refusal has to name the argument: {refusal}"
    );
    assert!(
        ui.get_notice().contains("command"),
        "and reach the window notice too: {:?}",
        ui.get_notice()
    );

    let relative = act_on(
        published,
        "open_directory",
        serde_json::json!({ "directory": "somewhere/relative" }),
    )
    .expect_err("a relative directory must be refused");
    assert!(
        relative.contains("absolute"),
        "the refusal has to say what was wrong with it: {relative}"
    );

    let folder = dir.join("folder with spaces; literal");
    let opened = act_on(
        published,
        "open_directory",
        serde_json::json!({ "directory": folder.display().to_string() }),
    )
    .expect("open a tab in a real folder");
    assert_eq!(opened["directory"], folder.display().to_string(), "answer: {opened}");
    assert_eq!(opened["tabs"], MAX_TABS, "answer: {opened}");
    assert_eq!(opened["alive"], true, "answer: {opened}");
    assert_ne!(opened["shell_pid"], pid, "a new tab is a new shell: {opened}");

    // The tab cap used to `return` in silence while the surface answered `{"tabs": n}`.
    let capped = act_on(published, "new_tab", serde_json::json!({}))
        .expect_err("a tab past the cap must be refused, not silently dropped");
    assert!(
        capped.contains(&MAX_TABS.to_string()),
        "the refusal has to say why: {capped}"
    );
    assert_eq!(state.borrow().tabs.len(), MAX_TABS, "and nothing may have been opened");
    assert!(
        ui.get_notice().contains(&MAX_TABS.to_string()),
        "and the person at the window is told too: {:?}",
        ui.get_notice()
    );
}
