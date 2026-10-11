//! Exercise the real window, callbacks and PTYs with Slint's native software
//! renderer. Only the window-system event queue and clipboard are substituted.
use super::*;
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Clipboard, EventLoopProxy, Platform, WindowAdapter, WindowEvent,
};
use std::sync::Arc;
use std::{collections::VecDeque, sync::Mutex, time::Instant};
mod whose;
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
            "Native Editor condition timed out"
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
fn shot_dir() -> PathBuf {
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

fn fixture(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "yantrik-editor-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn atomic_save_conflicts_links_permissions_and_utf8() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = fixture("atomic");
    let path = dir.join("original.txt");
    std::fs::write(&path, "hello 🦀\r\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut d = Document::open(&path).unwrap();
    d.text.push_str("second\r\n");
    let d = d.save(&path).unwrap();
    assert!(!d.dirty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), d.text);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    std::fs::write(&path, "external edit").unwrap();
    assert!(d.save(&path).unwrap_err().contains("changed on disk"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external edit");
    assert!(Document::blank().save(&path).is_err());
    let link = dir.join("linked.txt");
    symlink(&path, &link).unwrap();
    assert!(Document::blank().save(&link).is_err());
    let d = Document::open(&path).unwrap();
    std::fs::hard_link(&path, dir.join("hard.txt")).unwrap();
    assert!(d.save(&path).is_err());
    let bad = dir.join("binary");
    std::fs::write(&bad, [0xff, 0xfe]).unwrap();
    assert!(Document::open(&bad).is_err());
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}
#[test]
fn bounded_inputs_and_unicode_search() {
    assert!(document::validate("binary\u{1}control").is_err());
    let text = "a".repeat(20000);
    let ranges = document::matches(&text, "a", true);
    assert_eq!(ranges.len(), 20000);
    assert_eq!(
        document::replace(&text, &ranges, "b").unwrap(),
        "b".repeat(20000)
    );
    assert!(document::replace(&text, &ranges, &"x".repeat(1024)).is_err());
    assert!(document::validate(&"a".repeat(document::MAX_BYTES + 1)).is_err());
    assert!(document::validate(&"\n".repeat(20001)).is_err());
    let text = "İ α Kelvin K kelvin 🦀";
    let ranges = document::matches(text, "k", false);
    assert_eq!(
        ranges.iter().map(|&(a, z)| &text[a..z]).collect::<Vec<_>>(),
        ["K", "K", "k"]
    );
    assert_eq!(document::matches("one ONE", "one", true), [(0, 3)]);
    let dir = fixture("bounded");
    let fifo = dir.join("pipe");
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    unsafe {
        libc::mkfifo(path.as_ptr(), 0o600);
    }
    assert!(Document::open(&fifo).is_err());
    let huge = dir.join("large");
    let f = std::fs::File::create(&huge).unwrap();
    f.set_len(2 * 1024 * 1024 * 1024).unwrap();
    assert!(Document::open(&huge).is_err());
}
#[test]
fn recovery_preserves_drafts_without_touching_originals() {
    let dir = fixture("recovery");
    let original = dir.join("a.txt");
    std::fs::write(&original, "disk").unwrap();
    let mut d = Document::open(&original).unwrap();
    d.text = "draft 🦀".into();
    let path = dir.join("recovery/drafts.json");
    document::checkpoint(&path, &[d, Document::blank()]).unwrap();
    let recovered = document::recover(&path).unwrap();
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].dirty());
    assert_eq!(recovered[0].text, "draft 🦀");
    assert_eq!(std::fs::read_to_string(original).unwrap(), "disk");
    let mut escaped = Document::blank();
    escaped.baseline = "\"".repeat(document::MAX_BYTES);
    escaped.text = "\\".repeat(document::MAX_BYTES);
    document::checkpoint(&path, &vec![escaped; 8]).unwrap();
    assert_eq!(document::recover(&path).unwrap().len(), 8);
    document::checkpoint(&path, &[]).unwrap();
    assert!(document::recover(&path).unwrap().is_empty());
}
// ── A file moved while it was open (#86) ────────────────────────────────────
//
// What Files does with cut and paste: `rename` carries the file out from under the tab, and the
// pair that stranded the work was `save` refusing because the original was gone and Save As
// refusing because the file was already sitting at the path it had been moved to. Both
// refusals were true; between them there was no way to keep the edit.

#[test]
fn a_file_moved_while_open_can_still_be_saved_to_where_it_went() {
    let dir = std::fs::canonicalize(fixture("moved")).unwrap();
    let from = dir.join("notes.txt");
    std::fs::write(&from, "first\n").unwrap();
    let archive = dir.join("archive");
    std::fs::create_dir(&archive).unwrap();
    let to = archive.join("notes.txt");

    let mut d = Document::open(&from).unwrap();
    d.edit("second\n".into());
    std::fs::rename(&from, &to).unwrap(); // what Files does

    // Saving to the old path refuses — but the refusal names where the bytes went, and the way
    // through is a Save As onto that path rather than a dead end.
    let refused = d.save(&from).unwrap_err();
    assert!(
        refused.contains(&to.display().to_string()),
        "the refusal has to name where the file went: {refused}"
    );
    assert!(refused.contains("Save As"), "refusal: {refused}");
    assert!(refused.contains("draft is intact"), "refusal: {refused}");

    // And that Save As is not refused as a clobber: the file at `to` is this tab's own, moved,
    // so writing into it is what a plain Save would have done.
    let saved = d
        .save(&to)
        .expect("the tab's own file, moved, is not somebody else's file");
    assert_eq!(saved.path.as_deref(), Some(to.as_path()));
    assert!(!saved.dirty());
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "second\n");
}

#[test]
fn save_as_onto_an_existing_path_refuses_without_overwrite_and_succeeds_with_it() {
    let dir = std::fs::canonicalize(fixture("clobber")).unwrap();
    let occupied = dir.join("occupied.txt");
    std::fs::write(&occupied, "somebody else's\n").unwrap();

    let mut d = Document::blank();
    d.edit("fresh\n".into());

    let refused = d.save(&occupied).unwrap_err();
    assert!(refused.contains("already exists"), "refusal: {refused}");
    assert!(
        refused.contains("nothing was overwritten"),
        "refusal: {refused}"
    );
    assert!(
        refused.contains("overwrite=true"),
        "the refusal has to name the way through it: {refused}"
    );
    assert_eq!(
        std::fs::read_to_string(&occupied).unwrap(),
        "somebody else's\n",
        "a refused save changes nothing on disk"
    );

    let saved = d
        .save_over(&occupied)
        .expect("`overwrite` is the caller having read that refusal and answered it");
    assert!(!saved.dirty());
    assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "fresh\n");
}

#[test]
fn a_file_really_deleted_under_the_draft_is_written_back_with_permission() {
    let dir = std::fs::canonicalize(fixture("deleted")).unwrap();
    let path = dir.join("draft.txt");
    std::fs::write(&path, "kept\n").unwrap();
    let d = Document::open(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    // Nothing with this name and these bytes is anywhere near, so the search comes back empty
    // and the refusal says the file is gone rather than guessing where it went.
    let refused = d.save(&path).unwrap_err();
    assert!(refused.contains("moved or deleted"), "refusal: {refused}");
    assert!(refused.contains("draft is intact"), "refusal: {refused}");
    assert!(refused.contains("overwrite=true"), "refusal: {refused}");

    d.save_over(&path)
        .expect("writing the draft back where it came from");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept\n");
}

#[test]
fn real_editor_keyboard_tabs_search_save_close_and_recovery() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let queue = Queue::default();
    slint::platform::set_platform(Box::new(NativeTest {
        window: window.clone(),
        queue: queue.clone(),
        clipboard: Arc::new(Mutex::new(String::new())),
    }))
    .unwrap();
    let ui = TextEditorApp::new().unwrap();
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(1100, 760));
    let dir = fixture("ui");
    let recovery = dir.join("state/drafts.json");
    let s = wire(&ui, recovery.clone(), false);
    tick(&queue, &window);
    ui.invoke_focus_editor();
    key(&window, "Hello 🦀\nKelvin K kelvin\n");
    assert!(s.borrow().docs[0].dirty());
    assert_eq!(ui.get_line_count(), 3);
    ui.invoke_action("new".into());
    key(&window, "second draft");
    ui.invoke_action("undo".into());
    assert_eq!(ui.get_content(), "");
    ui.invoke_action("redo".into());
    assert_eq!(ui.get_content(), "second draft");
    ui.invoke_select_tab(0);
    window.dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Control.into(),
    });
    key(&window, "z");
    window.dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Control.into(),
    });
    assert_eq!(ui.get_content(), "");
    ui.invoke_action("redo".into());
    assert!(ui.get_content().starts_with("Hello"));
    ui.set_query("k".into());
    ui.invoke_search();
    assert_eq!(ui.get_match_count(), 3);
    ui.set_replacement("X".into());
    ui.invoke_action("replace-all".into());
    assert!(ui.get_content().contains("Xelvin X Xelvin"));
    ui.invoke_close_tab(0);
    assert_eq!(ui.get_dialog(), 3);
    ui.invoke_action("cancel".into());
    assert_eq!(s.borrow().docs.len(), 2);
    ui.invoke_action("save-as".into());
    ui.set_dialog_path(dir.join("saved.txt").display().to_string().into());
    ui.invoke_action("confirm".into());
    wait(&queue, &window, || !ui.get_busy());
    assert!(!s.borrow().docs[0].dirty());
    key(&window, "new edit");
    std::fs::write(dir.join("saved.txt"), "external").unwrap();
    ui.invoke_action("save".into());
    wait(&queue, &window, || !ui.get_busy());
    assert!(ui.get_notice().contains("changed on disk"));
    assert!(s.borrow().docs[0].dirty());
    ui.invoke_action("save-as".into());
    ui.set_dialog_path(dir.join("missing/no.txt").display().to_string().into());
    ui.invoke_action("confirm".into());
    wait(&queue, &window, || !ui.get_busy());
    assert_eq!(ui.get_dialog(), 2);
    assert!(s.borrow().docs[0].dirty());
    ui.invoke_action("cancel".into());
    wait(&queue, &window, || {
        ui.get_recovery_status() == "Draft recovery up to date"
    });
    assert_eq!(document::recover(&recovery).unwrap().len(), 2);
    // There is no cap at eight any more: small tabs keep opening, the strip shrinks them and
    // then scrolls, and the active one is scrolled into sight (rendered below, not just counted).
    for _ in 0..22 {
        ui.invoke_action("new".into());
    }
    assert_eq!(s.borrow().docs.len(), 24, "{}", ui.get_notice());
    assert_eq!(ui.get_active_tab(), 23);
    tick(&queue, &window);
    save(&window, "editor-many-tabs.png");
    ui.invoke_select_tab(0);
    assert!(ui.get_content().contains("Xelvin"));
    tick(&queue, &window);
    for _ in 0..16 {
        ui.invoke_close_tab(2);
    }
    assert_eq!(s.borrow().docs.len(), 8);
    // Real pointer/keyboard view with a small source file, rendered in both themes.
    let demo = dir.join("hello.rs");
    std::fs::write(&demo,"// A small idea, ready to grow.\n\nfn main() {\n    let message = \"Hello, Yantrik\";\n    println!(\"{}\", message);\n}\n").unwrap();
    for _ in 0..6 {
        ui.invoke_close_tab(2);
    }
    open(&ui, &s, demo);
    wait(&queue, &window, || !ui.get_busy());
    tick(&queue, &window);
    save(&window, "editor-dark.png");
    ui.global::<ThemeMode>().set_dark(false);
    window.set_size(slint::PhysicalSize::new(800, 600));
    tick(&queue, &window);
    save(&window, "editor-light.png");
    // Defocus the native caret: idle workbench must not continually request frames.
    window.dispatch_event(WindowEvent::WindowActiveChanged(false));
    let settle = std::time::Instant::now();
    while settle.elapsed() < Duration::from_millis(1200) {
        tick(&queue, &window);
        std::thread::sleep(Duration::from_millis(20));
    }
    let before = std::time::Instant::now();
    let mut redraws = 0;
    while before.elapsed() < Duration::from_millis(1000) {
        redraws += tick(&queue, &window) as usize;
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        redraws, 0,
        "inactive editor should not keep requesting frames"
    );

    // ── What the mind is shown, and what it is told afterwards ──
    //
    // These run inside this test rather than as their own `#[test]` because a process may
    // install exactly one Slint platform — i-slint-core's `EVENTLOOP_PROXY` is a process-wide
    // `OnceCell` — and every check below needs a real window to read the surface off.
    let published = surface(&ui, &s);
    every_action_says_what_it_does(&published);
    read_pages_a_tab_another_tab_and_a_file(&ui, &s, &published, &dir);
    the_editor_answers_with_what_it_wrote(&ui, &s, &published, &dir);
    an_agent_names_only_files_in_the_home(&ui, &s, &published, &dir);
    a_missing_required_argument_is_refused_by_name(&ui, &s, &published);
    append_adds_to_the_end_and_takes_nothing_away(&s, &published);
    a_document_too_big_to_draw_is_windowed_read_only_and_kept_whole(
        &ui,
        &s,
        &queue,
        &window,
        &published,
        &dir,
    );
    whose::tabs_know_who_opened_them(&ui, &s, &published, &dir);

    let mut b = s.borrow_mut();
    b.recovery_timer.stop();
    let _ = b.jobs.send(Job::Shutdown(b.docs.clone()));
    if let Some(w) = b.worker.take() {
        w.join().unwrap();
    }
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
        .unwrap_or_else(|| panic!("the editor does not publish `{name}`"));
    handler(&args)
}

/// The check that would have caught `"Editor: replace-all"`.
///
/// Sixteen actions were built in two loops, all sixteen described as `Editor: <name>`, with a
/// single argument named `path` or `text` and no description at all — `select_tab` took a "text"
/// that had to be a number. Since d73760d `yos describe` prints each action's description and
/// each argument's type, required flag and description, so this is verbatim what a model reads.
fn every_action_says_what_it_does(published: &[(Action, Handler)]) {
    assert!(
        published.len() >= 16,
        "the surface lost actions: {} published",
        published.len()
    );
    for (spec, _) in published {
        assert_ne!(
            spec.description,
            format!("Editor: {}", spec.name),
            "`{}` still carries the placeholder the old loop generated",
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
            assert!(
                p.required || p.description.contains("Leave it out"),
                "`{}` has an optional `{}`; every argument on this surface is needed, and one \
                 that may be left out has to say what leaving it out means",
                spec.name,
                p.name
            );
        }
        assert!(
            ["safe", "standard", "sensitive", "dangerous"].contains(&spec.permission),
            "`{}` is graded `{}`, which is not a grade this OS defines",
            spec.name,
            spec.permission
        );
    }
    // The two that throw away unsaved text.
    for name in ["discard", "set_content"] {
        let (spec, _) = published.iter().find(|(s, _)| s.name == name).unwrap();
        assert_eq!(
            spec.permission, "sensitive",
            "`{name}` destroys unsaved work and cannot be graded `{}`",
            spec.permission
        );
    }
    // A pure read is `safe`: a mind keeps about 4,000 characters of a safe read and cuts anything
    // graded higher to 900, which would hide the very text the read was for (VM 520, 4 October).
    let (spec, _) = published.iter().find(|(s, _)| s.name == "read").expect("`read` is published");
    assert_eq!(spec.permission, "safe", "`read` only reads; it must be graded `safe`");
}

/// VM 520, 4 October: a mind asked to read ~/mdg/MDG-spec.md (9,557 bytes) found no action that
/// read a document, and `describe` showed only its first 4,000 characters without saying so.
/// Through the real surface: `describe` and `open` now say the content is cut and name the call,
/// and `read` pages the tab in front, another tab, and a file once `open` has opened it.
fn read_pages_a_tab_another_tab_and_a_file(
    ui: &TextEditorApp,
    s: &State,
    published: &[(Action, Handler)],
    dir: &Path,
) {
    let spec = spec_like(240);
    let in_front = s.borrow().active;
    act_on(published, "new", serde_json::json!({ "text": spec.clone() })).expect("a long tab");
    let shown = view(ui, s);
    assert_eq!(shown.state["content_cut"], true, "{}", shown.state);
    assert!(shown.state["read_with"].as_str().unwrap().starts_with("read from_line "), "{}", shown.state);
    assert!(shown.summary.contains("`read` pages"), "the first line says so too: {}", shown.summary);
    assert_eq!(walk(published, serde_json::json!({})), spec, "the tab in front, read whole");

    // Another tab, read where it is: nothing comes forward.
    let long_tab = s.borrow().active;
    act_on(published, "select_tab", serde_json::json!({ "index": in_front })).expect("back");
    let page = act_on(published, "read", serde_json::json!({ "tab": long_tab })).expect("read a tab");
    assert_eq!(page["modified"], true, "a new tab never saved is a buffer, not a file: {page}");
    assert!(page["how_to_see_the_rest"].as_str().unwrap().contains(&format!("tab {long_tab} and from_line ")), "{page}");
    assert_eq!(walk(published, serde_json::json!({ "tab": long_tab })), spec);
    assert_eq!(s.borrow().active, in_front, "reading a tab does not bring it forward");
    assert!(act_on(published, "read", serde_json::json!({ "tab": 99 })).unwrap_err().contains("no tab 99"));
    act_on(published, "select_tab", serde_json::json!({ "index": long_tab })).expect("forward");
    act_on(published, "close", serde_json::json!({})).expect("close the long tab");
    act_on(published, "discard", serde_json::json!({})).expect("discard the long tab");

    // A file is `open`ed (standard) and then read; `read` itself never takes a path, because a
    // safe read runs unasked and a path would reach every text file in the home.
    let file = dir.join("MDG-spec.md");
    std::fs::write(&file, &spec).unwrap();
    let tabs = s.borrow().docs.len();
    let by_path = serde_json::json!({ "path": file.display().to_string() });
    let refused = act_on(published, "read", by_path.clone()).expect_err("a path is refused");
    assert!(refused.contains("takes no `path`") && refused.contains("`open`"), "{refused}");
    assert_eq!(s.borrow().docs.len(), tabs, "a refused read opens nothing");
    let opened = act_on(published, "open", by_path).expect("open the spec");
    assert_eq!(opened["content_cut"], true, "{opened}");
    let read_with = opened["read_with"].as_str().unwrap().to_string();
    let from: usize = read_with.strip_prefix("read from_line ").expect(&read_with).parse().unwrap();
    let rest = act_on(published, "read", serde_json::json!({ "from_line": from })).expect("the named call");
    assert_eq!(rest["modified"], false, "a page of the file as saved says so: {rest}");
    assert!(spec.lines().nth(from - 1).map_or(false, |l| rest["text"].as_str().unwrap().starts_with(l)), "{rest}");
    assert_eq!(walk(published, serde_json::json!({})), spec, "the opened file, read whole");
    act_on(published, "close", serde_json::json!({})).expect("close the spec");

    // A short tab: describe is whole and says so.
    act_on(published, "new", serde_json::json!({ "text": "short\n" })).expect("a short tab");
    assert_eq!(view(ui, s).state["content_cut"], false);
    assert!(view(ui, s).state["read_with"].is_null());
    act_on(published, "close", serde_json::json!({})).expect("close the short tab");
    if ui.get_dialog() == 3 {
        act_on(published, "discard", serde_json::json!({})).expect("discard the short tab");
    }
}

/// Follow `read`'s `how_to_see_the_rest` from line 1 to the end and give back the text it read.
fn walk(published: &[(Action, Handler)], args: serde_json::Value) -> String {
    let mut pages = Vec::new();
    let mut from = 1;
    for _ in 0..100 {
        let mut call = args.clone();
        call["from_line"] = serde_json::json!(from);
        let page = act_on(published, "read", call).expect("a page");
        assert!(page.to_string().len() < 4_000, "a page a mind cannot keep whole: {page}");
        pages.push(page["text"].as_str().unwrap().to_string());
        let hint = page["how_to_see_the_rest"].as_str().unwrap();
        if hint.is_empty() {
            return pages.join("\n");
        }
        from = page["to_line"].as_u64().unwrap() as usize + 1;
        assert!(hint.ends_with(&format!("from_line {from}")), "the next call is named exactly: {hint}");
    }
    panic!("100 pages and the document never ended");
}

/// A document shaped like the spec from VM 520: about 9.5 KB over `lines` lines of uneven length,
/// with the quotes, backslashes and non-ASCII that JSON makes longer.
fn spec_like(lines: usize) -> String {
    (0..lines)
        .map(|i| match i % 4 {
            0 => format!("## {i}. Section — \"quoted\" \\path\\"),
            1 => format!("- item {i}: {}", "word ".repeat(i % 13 + 6)),
            2 => String::new(),
            _ => format!("{i}\ttabbed ✓ {}", "x".repeat(i % 60)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn read_walks_a_long_document_exactly_once_in_pages_a_mind_keeps_whole() {
    let spec = spec_like(240);
    assert!(spec.len() > 8_000, "long enough to need pages: {}", spec.len());
    let lines_total = spec.split('\n').count();
    let mut from = 1;
    let mut pages = Vec::new();
    loop {
        let page = reading::page(&spec, serde_json::json!("/home/p/mdg/MDG-spec.md"), from, "");
        let size = page.to_string().len();
        assert!(size < 4_000 && size <= reading::READ_ANSWER, "a page a mind cannot keep whole: {size}");
        assert_eq!(page["lines_total"], lines_total);
        assert_eq!(page["from_line"], from);
        let to = page["to_line"].as_u64().unwrap() as usize;
        assert!(to >= from, "a page always carries at least one line: {page}");
        assert!(page.get("line_cut").is_none(), "no line here is longer than a page: {page}");
        pages.push(page["text"].as_str().unwrap().to_string());
        let hint = page["how_to_see_the_rest"].as_str().unwrap();
        if to == lines_total {
            assert_eq!(hint, "", "the last page says nothing is left");
            break;
        }
        assert_eq!(hint, format!("lines {from}–{to} of {lines_total}: call read with from_line {}", to + 1));
        from = to + 1;
    }
    assert!(pages.len() >= 3, "9.5 KB is more than one page: {} pages", pages.len());
    assert_eq!(pages.join("\n"), spec, "every line exactly once, in order");
}

#[test]
fn read_fits_a_short_document_in_one_page_with_nothing_to_follow() {
    let page = reading::page("one\ntwo\n", serde_json::json!(null), 1, "");
    assert_eq!(page["text"], "one\ntwo\n");
    assert_eq!(page["lines_total"], 3, "counted as `describe.lines` counts them");
    assert_eq!((page["from_line"].as_u64(), page["to_line"].as_u64()), (Some(1), Some(3)));
    assert_eq!(page["how_to_see_the_rest"], "");
    assert!(reading::describe_cut("one\ntwo\n").is_none(), "describe holds it whole");
}

#[test]
fn read_cuts_a_line_longer_than_a_page_and_says_so() {
    let text = format!("head\n{}\ntail", "\"".repeat(9_000));
    let first = reading::page(&text, serde_json::json!(null), 1, "");
    assert_eq!(first["text"], "head", "the long line starts its own page");
    assert_eq!(first["how_to_see_the_rest"], "lines 1–1 of 3: call read with from_line 2");
    let long = reading::page(&text, serde_json::json!(null), 2, "");
    assert!(long.to_string().len() < 4_000, "{}", long.to_string().len());
    assert_eq!(long["to_line"], 2, "one line, cut");
    assert!(long["line_cut"].as_str().unwrap().contains("9000 characters long"), "{long}");
    assert!(long["how_to_see_the_rest"].as_str().unwrap().ends_with("from_line 3"), "{long}");
    let past = reading::page(&text, serde_json::json!(null), 9, "");
    assert_eq!(past["text"], "");
    assert!(past["how_to_see_the_rest"].as_str().unwrap().contains("past the end"), "{past}");
}

#[test]
fn read_names_the_whole_next_call_and_describe_names_where_its_content_stops() {
    let spec = spec_like(240);
    let page = reading::page(&spec, serde_json::json!("/home/p/x.md"), 1, "tab 2 and ");
    let hint = page["how_to_see_the_rest"].as_str().unwrap();
    assert!(hint.contains("call read with tab 2 and from_line "), "{hint}");
    assert!(page.to_string().len() < 4_000);
    let cut = reading::describe_cut(&spec).expect("9.5 KB is cut at 4,000 characters");
    let shown = reading::describe_content(&spec);
    assert_eq!(shown.chars().count(), reading::DESCRIBE_CHARS);
    assert_eq!(cut, format!("read from_line {}", shown.split('\n').count()), "the line the cut falls in");
}

/// Every answer reports what was observed: the path, the bytes, the count that changed.
///
/// Each of these used to answer `{"accepted": true}` or `{"accepted": true, "completed": true}`.
fn the_editor_answers_with_what_it_wrote(
    ui: &TextEditorApp,
    s: &State,
    published: &[(Action, Handler)],
    dir: &Path,
) {
    let fresh = act_on(published, "new", serde_json::json!({})).expect("a new tab");
    assert_eq!(fresh["title"], "Untitled", "answer: {fresh}");
    assert_eq!(fresh["path"], serde_json::Value::Null, "answer: {fresh}");
    assert_eq!(fresh["modified"], false, "answer: {fresh}");

    let written = act_on(
        published,
        "set_content",
        serde_json::json!({ "text": "alpha\nbeta\nalpha\n" }),
    )
    .expect("set_content on the new tab");
    assert_eq!(written["lines"], 4, "answer: {written}");
    assert_eq!(written["modified"], true, "answer: {written}");
    assert_eq!(written["on_disk"], false, "nothing has been written yet: {written}");

    let found = act_on(published, "find", serde_json::json!({ "text": "alpha" })).expect("find");
    assert_eq!(found["matches"], 2, "answer: {found}");

    act_on(published, "replace_text", serde_json::json!({ "text": "omega" })).expect("replace_text");
    let replaced = act_on(published, "replace-all", serde_json::json!({})).expect("replace-all");
    assert_eq!(replaced["replaced"], 2, "the answer counts what it changed: {replaced}");
    assert_eq!(s.borrow().docs[s.borrow().active].text, "omega\nbeta\nomega\n");

    // A caller that can rewrite a whole tab has to be able to put it back.
    let undone = act_on(published, "undo", serde_json::json!({})).expect("undo the replace-all");
    assert_eq!(undone["modified"], true, "answer: {undone}");
    assert_eq!(s.borrow().docs[s.borrow().active].text, "alpha\nbeta\nalpha\n");
    act_on(published, "redo", serde_json::json!({})).expect("redo it");
    assert_eq!(s.borrow().docs[s.borrow().active].text, "omega\nbeta\nomega\n");

    // A tab that has never been written anywhere used to open a dialog and answer `accepted`.
    let refused = act_on(published, "save", serde_json::json!({}))
        .expect_err("save on a tab with no file must be refused");
    assert!(
        refused.contains("save_as"),
        "the refusal has to name the action that would work: {refused}"
    );

    let path = dir.join("surface.txt");
    let saved = act_on(
        published,
        "save_as",
        serde_json::json!({ "path": path.display().to_string() }),
    )
    .expect("save_as to a fresh path");
    assert_eq!(saved["path"], path.display().to_string(), "answer: {saved}");
    assert_eq!(saved["matches_disk"], true, "answer: {saved}");
    assert_eq!(saved["modified"], false, "answer: {saved}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("the file is on disk"),
        "omega\nbeta\nomega\n"
    );

    // Fault 3: the summary was the filename and nothing else.
    let summary = view(ui, s).summary;
    assert!(
        summary.contains("surface.txt") && summary.contains("saved"),
        "the summary has to name the file and whether it is saved: {summary:?}"
    );
    assert!(
        summary.contains("tab "),
        "and which tab of how many: {summary:?}"
    );

    // #86, on the surface a mind reads: a file already at the path is refused, the refusal
    // names the way through it, and nothing is overwritten until the caller says so.
    let occupied = dir.join("occupied.txt");
    std::fs::write(&occupied, "somebody else's\n").unwrap();
    let refused = act_on(
        published,
        "save_as",
        serde_json::json!({ "path": occupied.display().to_string() }),
    )
    .expect_err("save_as onto an existing file must be refused without `overwrite`");
    assert!(refused.contains("already exists"), "refusal: {refused}");
    assert!(
        refused.contains("overwrite=true"),
        "the refusal has to name the way through it: {refused}"
    );
    assert_eq!(
        std::fs::read_to_string(&occupied).unwrap(),
        "somebody else's\n",
        "a refused save_as changes nothing on disk"
    );
    let replaced = act_on(
        published,
        "save_as",
        serde_json::json!({ "path": occupied.display().to_string(), "overwrite": true }),
    )
    .expect("`overwrite: true` replaces the file that was there");
    assert_eq!(replaced["path"], occupied.display().to_string(), "answer: {replaced}");
    assert_eq!(replaced["matches_disk"], true, "answer: {replaced}");
    assert_eq!(
        std::fs::read_to_string(&occupied).unwrap(),
        "omega\nbeta\nomega\n"
    );

    // A refusal names what it could not do, and reaches the screen.
    let missing = dir.join("not-here.txt");
    let failed = act_on(
        published,
        "open",
        serde_json::json!({ "path": missing.display().to_string() }),
    )
    .expect_err("opening a file that is not there must be refused");
    assert!(!failed.is_empty(), "the refusal has to say something");
    assert!(
        !ui.get_notice().is_empty(),
        "and it has to reach the window notice too"
    );

    let out_of_range = act_on(published, "select_tab", serde_json::json!({ "index": 99 }))
        .expect_err("there is no tab 99");
    assert!(
        out_of_range.contains("99") && out_of_range.contains("open"),
        "the refusal has to say how many tabs there are: {out_of_range}"
    );
}

/// `set_content` with nothing to put in used to answer `{"accepted": true, "completed": true}`.
fn a_missing_required_argument_is_refused_by_name(
    ui: &TextEditorApp,
    s: &State,
    published: &[(Action, Handler)],
) {
    let (spec, _) = published.iter().find(|(s, _)| s.name == "set_content").unwrap();
    assert!(
        spec.params.iter().any(|p| p.name == "text" && p.required),
        "`text` has to be declared required, or the runtime accepts `set_content` with no text"
    );

    let before = s.borrow().docs[s.borrow().active].text.clone();
    let refusal = act_on(published, "set_content", serde_json::json!({}))
        .expect_err("set_content with no text must be refused");
    assert!(
        refusal.contains("text"),
        "the refusal has to name the argument that was missing: {refusal}"
    );
    assert!(
        ui.get_notice().contains("text"),
        "the refusal has to reach the window notice too: {:?}",
        ui.get_notice()
    );
    assert_eq!(
        s.borrow().docs[s.borrow().active].text,
        before,
        "a refused set_content must not have changed the tab"
    );

    let no_path = act_on(published, "open", serde_json::json!({}))
        .expect_err("open with no path must be refused");
    assert!(
        no_path.contains("path"),
        "the refusal has to name the argument that was missing: {no_path}"
    );
}

/// A Writer's way into the Editor (#253). The shell had an editor of its own with an
/// `editor_append` graded `standard`, and that editor is gone; `set_content` here is `sensitive`
/// and above a Writer's ceiling, so `append` is what that role writes a draft with.
fn append_adds_to_the_end_and_takes_nothing_away(s: &State, published: &[(Action, Handler)]) {
    let (spec, _) = published
        .iter()
        .find(|(spec, _)| spec.name == "append")
        .expect("the editor publishes `append`");
    assert_eq!(
        spec.permission, "standard",
        "`append` takes nothing away, and it is the Writer role's way in under a `standard` ceiling"
    );

    let text = || s.borrow().docs[s.borrow().active].text.clone();
    let before = text();
    let added = act_on(published, "append", serde_json::json!({ "text": "\nMinutes\n" }))
        .expect("append to the active tab");
    assert_eq!(added["modified"], true, "answer: {added}");
    act_on(published, "append", serde_json::json!({ "text": "- budget agreed\n" }))
        .expect("append again");
    assert_eq!(text(), format!("{before}\nMinutes\n- budget agreed\n"), "nothing before it changed");

    act_on(published, "undo", serde_json::json!({})).expect("undo the second append");
    assert_eq!(text(), format!("{before}\nMinutes\n"), "undo takes one append back off");

    let refused = act_on(published, "append", serde_json::json!({ "text": "" }))
        .expect_err("append with nothing to add must be refused");
    assert!(refused.contains("text"), "the refusal names the argument: {refused}");
    assert_eq!(text(), format!("{before}\nMinutes\n"), "a refused append changes nothing");

    // The one-call route to a document with text in it, with no card: `new` with `text`, then
    // `save_as`. Minds running unattended failed file tasks waiting on `set_content`'s card; the
    // Mind's hint keys on `new` taking an argument called `text`, so the name is part of this.
    let (spec, _) = published.iter().find(|(spec, _)| spec.name == "new").unwrap();
    assert_eq!(spec.permission, "standard", "a new tab replaces nothing");
    assert!(
        spec.params.iter().any(|p| p.name == "text" && !p.required),
        "`new` takes an optional `text`"
    );
    let (spec, _) = published.iter().find(|(spec, _)| spec.name == "set_content").unwrap();
    assert!(
        spec.description.contains("`new` with `text`"),
        "the sensitive action names the standard route beside it: {:?}",
        spec.description
    );
    let tabs = s.borrow().docs.len();
    assert!(tabs < 8, "the flow before this left {tabs} tabs open; `new` needs room for one more");
    let fresh = act_on(published, "new", serde_json::json!({ "text": "Agenda\n- one\n" }))
        .expect("a new tab holding text");
    assert_eq!(text(), "Agenda\n- one\n", "the new tab holds the text: {fresh}");
    assert_eq!(fresh["modified"], true, "and it is unsaved until save_as: {fresh}");
    assert_eq!(fresh["path"], serde_json::Value::Null, "answer: {fresh}");
    assert_eq!(s.borrow().docs.len(), tabs + 1, "in a tab of its own");
}

/// #328: the crash at `euclid-0.22.13/src/vector.rs:688`. Slint's software renderer keeps
/// every physical coordinate in an `i16` and casts glyph origins before clipping, so a
/// document of ~2000 lines or a ~3900-character line — well inside the editor's own 1 MiB /
/// 20,000-line input bounds — aborted the window mid-draw, and because tabs are restored at
/// launch, every restart died the same way. The editor now draws a read-only leading window
/// of such a document and keeps every byte: each `tick` below rendered, and before the fix
/// the first one on an oversized document panicked.
fn a_document_too_big_to_draw_is_windowed_read_only_and_kept_whole(
    ui: &TextEditorApp,
    s: &State,
    queue: &Queue,
    window: &MinimalSoftwareWindow,
    published: &[(Action, Handler)],
    dir: &Path,
) {
    let text = || s.borrow().docs[s.borrow().active].text.clone();
    // Room for the three documents below: the checks above may have left eight tabs open.
    while s.borrow().docs.len() > 5 {
        let i = s.borrow().active as i32;
        ui.invoke_close_tab(i);
        if ui.get_dialog() == 3 {
            act_on(published, "discard", serde_json::json!({})).ok();
        }
        tick(queue, window);
    }

    // The document from the report — the file Hermes wrote on the VM — draws whole, unrestricted
    // and editable.
    let repro = include_str!("../repro-328-content.py").to_string();
    act_on(published, "new", serde_json::json!({ "text": repro.clone() }))
        .expect("the repro document opens");
    tick(queue, window);
    tick(queue, window);
    assert!(!ui.get_view_limited(), "the repro document fits in full");
    assert_eq!(ui.get_content().as_str(), repro);
    ui.invoke_focus_editor();
    key(window, "#");
    tick(queue, window);
    assert!(text().starts_with('#'), "the repro document stays editable");
    act_on(published, "undo", serde_json::json!({})).expect("undo the keystroke");
    assert_eq!(text(), repro);

    // One line longer than the renderer's i16 horizontal space: this panicked at vector.rs:688.
    let long = "x".repeat(5_000);
    act_on(published, "new", serde_json::json!({ "text": long.clone() }))
        .expect("the wide document opens");
    tick(queue, window);
    tick(queue, window);
    assert!(ui.get_view_limited(), "the wide document is windowed");
    assert!(
        ui.get_content().len() < long.len(),
        "the view is shorter than the document"
    );
    assert_eq!(text(), long, "the document keeps every character");
    assert!(
        !ui.get_view_status().is_empty(),
        "and the status bar says part is withheld"
    );

    // The windowed view is read-only: a keystroke cannot truncate the document to the view.
    ui.invoke_focus_editor();
    key(window, "y");
    tick(queue, window);
    assert_eq!(text(), long, "the read-only view takes no keystroke");

    // Saving writes the whole document, not the window.
    let path = dir.join("whole.txt");
    act_on(
        published,
        "save_as",
        serde_json::json!({ "path": path.display().to_string() }),
    )
    .expect("the wide document saves");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        long,
        "the file on disk is the whole document"
    );

    // A document taller than the renderer's i16 vertical space.
    let tall = format!("{}MARKER\n", "let value = 1;\n".repeat(19_000));
    act_on(published, "new", serde_json::json!({ "text": tall.clone() }))
        .expect("the tall document opens");
    tick(queue, window);
    tick(queue, window);
    assert!(ui.get_view_limited(), "the tall document is windowed");
    assert_eq!(text(), tall, "the document keeps all 19,001 lines");
    assert!(
        ui.get_content().lines().count() < 1_000,
        "the view is a small window: {} lines",
        ui.get_content().lines().count()
    );

    // A match beyond the window is still counted from the whole document, and selecting it is
    // skipped rather than pointing the TextInput at an offset it does not hold.
    let found = act_on(published, "find", serde_json::json!({ "text": "MARKER" }))
        .expect("find in the tall document");
    assert_eq!(found["matches"], 1, "the match is in the document: {found}");
    tick(queue, window);

    // A bigger font fits fewer lines inside the limit; the window follows without a restart.
    ui.set_font_pixels(22);
    tick(queue, window);
    tick(queue, window);
    let big = ui.get_content().lines().count();
    assert!(big > 0 && big < 400, "22px window is {big} lines");
    ui.set_font_pixels(14);
    tick(queue, window);
    assert!(
        ui.get_content().lines().count() > big,
        "14px window grows back: {} lines",
        ui.get_content().lines().count()
    );

    // What a mind reads is the whole document, and says the person sees a window of it.
    let summary = view(ui, s).summary;
    let tall_lines = tall.bytes().filter(|b| *b == b'\n').count() + 1;
    assert!(
        summary.contains(&format!("{tall_lines} lines")),
        "the summary counts the whole document: {summary:?}"
    );
    assert!(
        summary.contains("Showing the first"),
        "and says what the window withholds: {summary:?}"
    );
}

/// An agent names only files in the person's home, outside its protected places (#443).
///
/// `open ~/.ssh/id_ed25519` then `describe` read a key back, and `save_as ~/.bashrc` with
/// `overwrite` ran code as the person at their next login. The fixtures live in the temp
/// directory, outside the home, which is exactly what an agent is refused and the person is not.
fn an_agent_names_only_files_in_the_home(ui: &TextEditorApp, s: &State, published: &[(Action, Handler)], dir: &Path) {
    use yantrik_app_runtime::control::AgentTokenScope;
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if dir.starts_with(&home) {
        return; // A temp directory inside the home would be allowed, and prove nothing here.
    }
    let tab = s.borrow().docs[s.borrow().active].path.clone().expect("the tab saved above");
    let shown = view(ui, s).state;
    assert!(!shown["content"].as_str().unwrap_or_default().is_empty(), "the person sees the text: {shown}");
    assert!(shown["content_hidden"].is_null(), "{shown}");

    let text_before = s.borrow().docs[s.borrow().active].text.clone();
    ui.set_query("omega".into());
    search(ui, s, false);
    assert!(ui.get_match_count() > 0, "the person's find matches");

    let agent = AgentTokenScope::enter(Some("tok-editor-test".into()));
    let refused = |name: &str, args: serde_json::Value, why: &str| {
        let err = act_on(published, name, args.clone()).expect_err(&format!("{name} {args}"));
        assert!(err.ends_with(why), "{name} {args}: {err}");
    };
    refused("open", serde_json::json!({ "path": "~/.ssh/id_ed25519" }), " is protected");
    refused("open", serde_json::json!({ "path": "/etc/hostname" }), " is outside");
    refused("open", serde_json::json!({ "path": tab.display().to_string() }), " is outside");
    refused("save", serde_json::json!({}), " is outside");

    // The tab in front is the person's, from outside the home: nothing reads or changes its
    // text for an agent. `save_as` would copy it somewhere an agent may read; `find` would
    // answer "does it contain X?" by its count.
    let hidden = "its text is left alone; `select_tab` another or `new` one.";
    refused("save_as", serde_json::json!({ "path": "~/Documents/copied-out.txt" }), hidden);
    // Nor does `read` page it out, by default or by its number, and it reaches no file by path.
    refused("read", serde_json::json!({ "path": "~/.ssh/id_ed25519" }), "then `read` it.");
    let active = s.borrow().active;
    refused(
        "read",
        serde_json::json!({ "tab": active }),
        &format!("Tab {active} holds a file an agent is not shown, and its text is left alone; read another tab or `new` one."),
    );
    for (name, args) in [
        ("read", serde_json::json!({})),
        ("find", serde_json::json!({ "text": "omega" })),
        ("find-next", serde_json::json!({})),
        ("find-prev", serde_json::json!({})),
        ("replace_text", serde_json::json!({ "text": "x" })),
        ("replace", serde_json::json!({})),
        ("replace-all", serde_json::json!({})),
        ("set_content", serde_json::json!({ "text": "x" })),
        ("append", serde_json::json!({ "text": "x" })),
        ("undo", serde_json::json!({})),
        ("redo", serde_json::json!({})),
    ] {
        refused(name, args, hidden);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    assert!(!home.join("Documents/copied-out.txt").exists(), "nothing was copied out");
    assert_eq!(s.borrow().docs[s.borrow().active].text, text_before, "the tab is untouched");
    assert_eq!(s.borrow().docs[s.borrow().active].path.as_ref(), Some(&tab), "the tab stays where it was");

    // Nor is it read back through `describe`: not its text, its name, its path or a match count.
    let seen = view(ui, s);
    assert_eq!(seen.state["content"], "", "{}", seen.state);
    assert!(seen.state["content_hidden"].is_string(), "{}", seen.state);
    assert_eq!(seen.state["find_count"], 0, "{}", seen.state);
    assert_eq!(seen.state["find_query"], "", "{}", seen.state);
    for size in ["lines", "characters", "bytes"] {
        assert!(seen.state[size].is_null(), "{size}: {}", seen.state);
    }
    assert!(seen.summary.contains("not shown to an agent"), "{}", seen.summary);
    assert!(!seen.summary.contains(" line"), "no length in the summary either: {}", seen.summary);
    let listed = seen.state.to_string();
    assert!(!listed.contains("surface.txt") && !listed.contains(&dir.display().to_string()), "{listed}");
    assert!(seen.state["tabs"].as_array().unwrap().iter().any(|t| t["path"] == "(hidden)"), "{listed}");

    // In a tab of its own, the write rule alone decides.
    act_on(published, "new", serde_json::json!({ "text": "echo pwned\n" })).expect("a new tab");
    refused("save_as", serde_json::json!({ "path": "~/.bashrc", "overwrite": true }), " is protected");
    refused("save_as", serde_json::json!({ "path": "~/.config/autostart/x.desktop" }), " is protected");
    // Nor anywhere else programs read their settings and startup: the list can never be whole.
    for hidden_place in ["~/.gitconfig", "~/.local/bin/x", "~/bin/x", "~/.tmux.conf"] {
        refused("save_as", serde_json::json!({ "path": hidden_place }), " is hidden_place");
    }
    refused("save_as", serde_json::json!({ "path": dir.join("agent.txt").display().to_string() }), " is outside");
    assert!(!dir.join("agent.txt").exists(), "nothing was written");
    drop(agent);
    act_on(published, "set_content", serde_json::json!({ "text": "" })).expect("empty the scratch tab");
    act_on(published, "close", serde_json::json!({})).expect("close the scratch tab");
    if ui.get_dialog() == 3 {
        act_on(published, "discard", serde_json::json!({})).expect("discard the scratch tab");
    }
    ui.set_query("".into());
    search(ui, s, false);
}
