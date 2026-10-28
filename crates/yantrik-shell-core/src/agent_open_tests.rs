use super::*;
use crate::framework;

fn entry(app_id: &str, exec: &str) -> DesktopEntry {
    DesktopEntry { app_id: app_id.into(), exec: exec.into(), name: app_id.into(), ..Default::default() }
}

fn installed() -> Vec<DesktopEntry> {
    let mut lo = entry("yantrik-libreoffice", "yantrik-libreoffice %U");
    lo.surface = Some("libreoffice".into());
    lo.aliases = vec!["writer".into(), "calc".into()];
    vec![
        entry("yantrik-notes", "/opt/yantrik/bin/yantrik-notes %f"),
        entry("yantrik-text-editor", "yantrik-text-editor %f"),
        entry("gimp", "gimp %U"),
        entry("yantrik-terminal", "/opt/yantrik/bin/yantrik-terminal"),
        entry("yantrik-blender", "blender --python /x"),
        lo,
    ]
}

#[test]
fn the_browser_is_refused_for_html_svg_and_pdf() {
    // These are what the one MIME rule sends to the browser.
    for name in ["page.html", "page.htm", "logo.svg", "doc.pdf", "page.xhtml"] {
        let err = may_open_file(Handler::Browser, &installed()).expect_err(name);
        assert!(err.contains("Yantrik's own apps") && err.ends_with("Nothing was opened."), "{name}: {err}");
    }
}

#[test]
fn an_office_document_is_refused_by_its_handler() {
    let err = may_open_file(Handler::Desktop("yantrik-libreoffice.desktop"), &installed()).unwrap_err();
    assert!(err.contains("#953"), "{err}");
    let err = may_open_file(Handler::Desktop("libreoffice-calc.desktop"), &[]).unwrap_err();
    assert!(err.contains("#953"), "{err}");
}

#[test]
fn an_unknown_or_missing_handler_is_refused() {
    assert!(may_open_file(Handler::Desktop("mystery.desktop"), &installed()).is_err());
    let err = may_open_file(Handler::Desktop("gone"), &installed()).unwrap_err();
    assert!(err.contains("cannot be told"), "{err}");
}

#[test]
fn an_external_app_is_refused_and_a_yantrik_app_is_not() {
    assert!(may_open_file(Handler::Desktop("gimp.desktop"), &installed()).is_err());
    assert!(may_open_file(Handler::Desktop("yantrik-blender.desktop"), &installed()).is_err());
    // Ours are not refused as foreign: they are refused a window of a Mind's (#978 step 1).
    for err in [
        may_open_file(Handler::Desktop("yantrik-notes.desktop"), &installed()).unwrap_err(),
        may_open_file(Handler::Desktop("yantrik-text-editor"), &installed()).unwrap_err(),
        may_open_file(Handler::Desktop("yantrik-spreadsheet.desktop"), &installed()).unwrap_err(),
        may_open_file(Handler::Desktop("yantrik-studio.desktop"), &installed()).unwrap_err(),
        may_open_file(Handler::Desktop("yantrik-arcade.desktop"), &installed()).unwrap_err(),
        may_open_file(Handler::Desktop("yantrik-presentation.desktop"), &installed()).unwrap_err(),
        // Terminal is a framework app: a file named to it is never typed into a shell.
        may_open_file(Handler::Desktop("yantrik-terminal.desktop"), &installed()).unwrap_err(),
        may_open_file(Handler::TextEditor, &[]).unwrap_err(),
        may_open_file(Handler::ImageViewer, &[]).unwrap_err(),
    ] {
        assert!(err.contains("cannot open in your view"), "{err}");
    }
}

#[test]
fn an_entry_named_like_ours_that_runs_something_else_is_refused() {
    let planted = vec![entry("yantrik-notes", "/tmp/evil %f")];
    assert!(may_open_file(Handler::Desktop("yantrik-notes"), &planted).is_err());
}

#[test]
fn launching_libreoffice_is_refused_by_every_name() {
    for name in ["libreoffice", "writer", "calc", "Writer", "yantrik-libreoffice", "soffice"] {
        let err = may_launch(name, &installed()).expect_err(name);
        assert!(err.contains("#953"), "{name}: {err}");
        assert!(may_launch(name, &[]).is_err(), "{name} with no catalogue");
    }
}

#[test]
fn launching_an_external_app_is_refused_and_ours_opens() {
    assert!(may_launch("gimp", &installed()).unwrap_err().contains("Nothing was launched"));
    assert!(may_launch("yantrik-blender", &installed()).is_err());
    assert!(may_launch("notes", &installed()).unwrap_err().contains("cannot open in your view"));
    assert!(may_launch("terminal", &installed()).unwrap_err().contains("cannot open in your view"));
    assert!(may_launch("browser", &installed()).is_ok());
    assert!(may_launch("files", &installed()).is_ok(), "a screen of the shell has no entry");
}

/// Every app under `apps/` that implements the framework `App` is in the allowlist.
#[test]
fn every_framework_app_is_listed() {
    let apps = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
    let mut found = Vec::new();
    for dir in std::fs::read_dir(&apps).expect("apps/").flatten() {
        let Ok(files) = std::fs::read_dir(dir.path().join("src")) else { continue };
        for file in files.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "rs")) {
            let text = std::fs::read_to_string(file.path()).unwrap_or_default();
            if !text.contains("impl App for") {
                continue;
            }
            for line in text.lines().filter(|l| l.contains("const ID: &'static str")) {
                if let Some(id) = line.split('"').nth(1) {
                    found.push(id.to_string());
                }
            }
        }
    }
    assert!(found.len() >= 10, "the scan found {found:?}");
    for id in found {
        assert!(framework::by_id(&id).is_some(), "framework app `{id}` is not in YANTRIK_APPS");
    }
}

#[test]
fn the_media_player_is_refused_for_playlists_video_and_subtitles() {
    // mpv follows an m3u to local files and draws an srt or ass on screen.
    for name in ["list.m3u", "clip.mp4", "subs.srt", "subs.ass"] {
        let err = may_open_file(Handler::MediaPlayer, &installed()).expect_err(name);
        assert!(err.contains("Yantrik's own apps") && err.ends_with("Nothing was opened."), "{name}: {err}");
    }
}

#[test]
fn no_listed_app_is_mpv_or_a_wrapper_of_an_external_binary() {
    assert!(framework::FRAMEWORK_APPS.iter().all(|a| a.program.starts_with("yantrik-") && !a.program.contains("mpv")));
    // Our name over someone else's program does not pass.
    for exec in ["mpv --player-operation-mode=pseudo-gui %f", "/usr/bin/env mpv %f", "sh -c mpv"] {
        let wrapped = vec![entry("yantrik-music-player", exec)];
        assert!(may_open_file(Handler::Desktop("yantrik-music-player"), &wrapped).is_err(), "{exec}");
    }
    assert!(may_open_file(Handler::Desktop("mpv.desktop"), &[entry("mpv", "mpv %f")]).is_err());
}

fn named(app_id: &str, name: &str) -> DesktopEntry {
    DesktopEntry { app_id: app_id.into(), exec: format!("/opt/yantrik/bin/{app_id}"), name: name.into(), ..Default::default() }
}

/// #978 step 1: no framework app gives a Mind a window, by any name it is asked for and whether
/// the person has it open or not. The sentence is the same word for word.
#[test]
fn a_minds_open_app_of_a_framework_app_is_refused_with_the_next_step() {
    let installed = vec![
        named("yantrik-calendar", "Calendar"),
        named("yantrik-notes", "Notes"),
        named("yantrik-image-viewer", "Image Viewer"),
    ];
    let said = |name: &str| may_launch(name, &installed).unwrap_err();
    for name in ["calendar", "yantrik-calendar", "Calendar"] {
        assert_eq!(said(name), "Calendar cannot open in your view; use its actions and os_describe calendar instead.");
    }
    assert_eq!(said("notes"), "Notes cannot open in your view; use its actions and os_describe notes instead.");
    assert_eq!(
        said("image-viewer"),
        "Image Viewer cannot open in your view; use its actions and os_describe image-viewer instead."
    );
    assert_eq!(
        said("yantrik-image-viewer"),
        "Image Viewer cannot open in your view; use its actions and os_describe image-viewer instead."
    );
}

/// Every app of the framework is refused, with or without a catalogue entry to name it.
#[test]
fn every_framework_app_is_refused_to_a_mind() {
    for a in framework::FRAMEWORK_APPS {
        for name in [a.id, a.program] {
            let err = may_launch(name, &[]).expect_err(name);
            assert!(err.ends_with(&format!("use its actions and os_describe {} instead.", a.id)), "{name}: {err}");
        }
    }
    // What is not a framework app is as it was.
    assert!(may_launch("browser", &[]).is_ok());
    assert!(may_launch("files", &[]).is_ok());
}
