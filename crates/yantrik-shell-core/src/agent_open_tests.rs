use super::*;

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
    assert!(may_open_file(Handler::Desktop("yantrik-terminal.desktop"), &installed()).is_err());
    assert!(may_open_file(Handler::Desktop("yantrik-notes.desktop"), &installed()).is_ok());
    assert!(may_open_file(Handler::Desktop("yantrik-text-editor"), &installed()).is_ok());
    assert!(may_open_file(Handler::TextEditor, &[]).is_ok());
    assert!(may_open_file(Handler::ImageViewer, &[]).is_ok());
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
    assert!(may_launch("notes", &installed()).is_ok());
    assert!(may_launch("yantrik-notes", &installed()).is_ok());
    assert!(may_launch("terminal", &installed()).is_ok());
    assert!(may_launch("files", &installed()).is_ok(), "a screen of the shell has no entry");
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
    // Our name over someone else's program does not pass.
    for exec in ["mpv --player-operation-mode=pseudo-gui %f", "/usr/bin/env mpv %f", "sh -c mpv"] {
        let wrapped = vec![entry("yantrik-music-player", exec)];
        assert!(may_open_file(Handler::Desktop("yantrik-music-player"), &wrapped).is_err(), "{exec}");
    }
    assert!(may_open_file(Handler::Desktop("mpv.desktop"), &[entry("mpv", "mpv %f")]).is_err());
}
