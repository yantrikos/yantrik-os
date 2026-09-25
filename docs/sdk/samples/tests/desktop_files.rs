//! The `.desktop` files the guide shows, read the way the shell reads them.
//!
//! `docs/sdk/found-while-closed.md` says what each key does; this holds the files an author copies
//! (the two templates) and the ones the guide works through (Blender, LibreOffice) to what the
//! shell's own parser — `yantrik_shell_core::apps::parse_desktop_text`, the one its catalogue
//! scans with — makes of them. And every `.desktop` file any page quotes must declare a surface.

use std::path::{Path, PathBuf};

use yantrik_shell_core::apps::{parse_desktop_text, DesktopEntry};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// A `.desktop` file in this repository, parsed as the shell parses it.
fn entry(relative: &str) -> DesktopEntry {
    let path = repo().join(relative);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let stem = path.file_stem().unwrap().to_string_lossy().to_string();
    parse_desktop_text(&stem, &text).unwrap_or_else(|| panic!("{relative} is not an entry the launcher lists"))
}

#[test]
fn both_templates_declare_the_surface_they_serve() {
    for file in ["templates/rust-surface/my-surface.desktop", "templates/python-surface/my-surface.desktop"] {
        let e = entry(file);
        assert_eq!(e.surface.as_deref(), Some("my-surface"), "{file}");
        assert_eq!(e.aliases, vec!["my-tasks".to_string()], "{file}");
        assert!(!e.purpose.is_empty(), "{file}");
        assert_eq!(e.exec, "my-surface", "{file}");
        assert_eq!(e.adapter, None, "{file}: a program that serves its own surface names no adapter");
    }
}

#[test]
fn libreoffice_names_the_adapter_that_serves_it() {
    let e = entry("adapters/libreoffice/yantrik-libreoffice.desktop");
    assert_eq!(e.surface.as_deref(), Some("libreoffice"));
    assert_eq!(e.aliases, vec!["writer".to_string(), "calc".to_string()]);
    assert_eq!(e.adapter.as_deref(), Some("yantrik-libreoffice-adapter"));
    // `%U` is kept as the entry wrote it: the shell substitutes the file where the code
    // stands when it runs the command, and removes the code when there is no file (#304).
    assert_eq!(e.exec, "yantrik-libreoffice %U");
    // The wrapper the Exec runs ships with the adapter and is always installed, so only
    // TryExec — naming the app it wraps — hides the entry from a machine with no LibreOffice
    // (#214).
    assert_eq!(e.try_exec.as_deref(), Some("soffice"));
}

#[test]
fn blender_hosts_its_own_surface_and_names_no_adapter() {
    let e = entry("apps/desktop-files/yantrik-blender.desktop");
    assert_eq!(e.surface.as_deref(), Some("blender"));
    assert_eq!(e.adapter, None);
    assert!(e.exec.contains("--python"), "the add-on is loaded by the command itself: {}", e.exec);
    // A machine without Blender must not be offered the tile — or the name in any listing a
    // mind reads (#214).
    assert_eq!(e.try_exec.as_deref(), Some("blender"));
}

#[test]
fn every_desktop_file_the_guide_quotes_declares_a_surface() {
    let guide = repo().join("docs/sdk");
    let mut quoted = Vec::new();
    for page in std::fs::read_dir(&guide).unwrap().flatten() {
        let text = std::fs::read_to_string(page.path()).unwrap_or_default();
        for line in text.lines() {
            let line = line.trim();
            if let Some(file) = line.strip_prefix("<!-- from: ").and_then(|l| l.strip_suffix(" -->")) {
                if file.ends_with(".desktop") {
                    quoted.push(file.to_string());
                }
            }
        }
    }
    assert!(quoted.len() >= 3, "the guide quotes {} .desktop files: {quoted:?}", quoted.len());
    for file in quoted {
        assert!(entry(&file).surface.is_some(), "{file} declares no surface");
    }
}
