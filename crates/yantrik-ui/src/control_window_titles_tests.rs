use super::*;
use crate::windows::window_entry;

const TITLES: [&str; 8] = [
    "tokens.json (~/backup) - gedit", "tokens.json + (~/backup) - NVIM", "*tokens.json - Kate", "/home/ann/backup/id_ed25519 - nano",
    "server.pem.bak — Text Editor", "[~/.ssh/id_rsa] - vim", "\"credentials.json\" - Mousepad", ".tokens.json.swp - less",
];

/// Review R3-6: a mind reading `describe shell` gets the app's name, and a flag, for a title that
/// names a secret file.
#[test]
fn a_title_with_a_secret_name_is_replaced_for_a_mind() {
    for title in TITLES {
        let window = window_entry("gedit", title);
        let shown = row(&window, true);
        assert_eq!(shown["title_hidden"], true, "{title}");
        assert_eq!(shown["title"], app_display_name(&window.app_id), "{title}");
        assert_eq!(shown["app"], window.app_id.as_str(), "{title}");
        assert!(!shown.to_string().to_lowercase().contains("tokens") && !shown.to_string().contains("id_"), "{title}: {shown}");
    }
}

/// The person's own view is whole, and so is a title without a secret name: ordinary file names,
/// public keys and words that only contain a secret's name.
#[test]
fn the_person_and_ordinary_titles_see_the_title_whole() {
    for title in TITLES {
        let shown = row(&window_entry("gedit", title), false);
        assert_eq!(shown["title"], title);
        assert!(shown.get("title_hidden").is_none(), "{title}");
    }
    for title in ["notes.md (~/Documents) - gedit", "id_rsa.pub - gedit", "tokens-notes.txt - Kate", "My tokens list - Text Editor", "keynote.txt - nano"] {
        let shown = row(&window_entry("gedit", title), true);
        assert_eq!(shown["title"], title);
        assert!(shown.get("title_hidden").is_none(), "{title}: {shown}");
    }
}

/// `in_front` is the same title; it is hidden the same way, by the name of the app that has it.
#[test]
fn the_front_window_title_is_hidden_like_its_row() {
    let windows = vec![window_entry("gedit", "tokens.json (~/backup) - gedit"), window_entry("gedit", "notes.md - gedit")];
    let front = Some(windows[0].title.clone());
    assert_eq!(in_front(front.clone(), &windows, true), Some(app_display_name(&windows[0].app_id)));
    assert_eq!(in_front(front.clone(), &[], true).as_deref(), Some(A_WINDOW));
    assert_eq!(in_front(front.clone(), &windows, false), front);
    assert_eq!(in_front(Some("notes.md - gedit".to_string()), &windows, true).as_deref(), Some("notes.md - gedit"));
    assert_eq!(in_front(None, &windows, true), None);
}

/// Review M4: quotes of every kind, ellipses, full stops, dashes and the other ways a title sets a
/// file's name beside other text, and a path into a protected place.
#[test]
fn a_title_that_sets_the_name_in_curly_quotes_or_a_sentence_is_hidden() {
    for title in [
        "“tokens.json” Properties", "Save changes to “tokens.json”?", "‘tokens.json’ - less", "«tokens.json»", "tokens.json… - Text Editor",
        "Opening tokens.json.", "tokens.json–Mousepad", "deploy-2026-11-15T10:30:00.pem - gedit", "config (~/.aws) - VIM", "/home/ann/.ssh/known_hosts - nano",
        "Delete “server (conflicted copy 2026-11-15 101530).pem”?", "backup:id_ed25519 - gedit",
        // Review R5-M3: names with spaces in them.
        "tokens (copy 1).json - gedit", "tokens 2.json - gedit", "tokens (1).json - Text Editor", "tokens - Copy.json - Kate",
        "“tokens (conflicted copy 2026-11-15 101530).json” Properties", "Login Data - DB Browser for SQLite", "Local State - gedit",
    ] {
        let shown = row(&window_entry("gedit", title), true);
        assert_eq!(shown["title_hidden"], true, "{title}");
        assert!(!shown.to_string().contains("tokens") && !shown.to_string().contains(".aws") && !shown.to_string().contains("pem"), "{title}: {shown}");
        assert_eq!(row(&window_entry("gedit", title), false)["title"], title, "the person sees it whole: {title}");
    }
    for title in ["“notes.md” Properties", "Opening notes.md.", "~/Documents - Files", "/home/ann/Documents/notes.md - nano", "id_rsa.pub… - gedit", "Budget – 2026 — Calc",
        "notes (copy 1).md - gedit", "Chapter 2.md - gedit", "Data Science (1).pdf - Viewer",
    ] {
        assert!(row(&window_entry("gedit", title), true).get("title_hidden").is_none(), "{title}");
    }
}

/// Review M4, the switcher: a cell's title, and its name when it is the title, are hidden from a
/// mind and whole for the person; and `described` asks who is calling.
#[test]
fn a_switcher_cell_hides_a_secret_title_from_a_mind() {
    let name = "tokens.json - gedit";
    let hidden = cell(name, name, "gedit", true);
    assert_eq!(hidden["title_hidden"], true);
    assert!(!hidden.to_string().contains("tokens"), "{hidden}");
    assert_eq!(hidden["title"], app_display_name("gedit"));
    assert_eq!(cell("gedit", name, "gedit", true)["title_hidden"], true);
    let whole = cell(name, name, "gedit", false);
    assert_eq!((whole["name"].as_str(), whole["title"].as_str()), (Some(name), Some(name)));
    assert!(whole.get("title_hidden").is_none());
    assert!(cell("Calendar", "notes.md - gedit", "gedit", true).get("title_hidden").is_none());
    let switcher = include_str!("control_switcher.rs");
    assert!(
        switcher.contains("crate::control_window_titles::cell(&c.app_name, &c.title, &c.app_id, crate::control_files_mind::a_mind_is_calling())"),
        "`described` asks who is calling"
    );
}

/// Review L6: a notification's title for a mind, and the front window's wiring.
#[test]
fn a_notification_title_that_names_a_secret_is_hidden_from_a_mind() {
    assert_eq!(notice_title("Download complete: tokens.json", true), ("a notification".to_string(), true));
    assert_eq!(notice_title("Download complete: tokens.json", false), ("Download complete: tokens.json".to_string(), false));
    assert_eq!(notice_title("Download complete: notes.md", true), ("Download complete: notes.md".to_string(), false));
    assert!(include_str!("notifications.rs").contains("notice_title(&n.title, crate::control_files_mind::a_mind_is_calling())"));
}

/// Review R5-M2: a dock button of an app with no name of its own is labelled by its window's title;
/// a mind is given the app's name instead, and `describe shell` asks who is calling.
#[test]
fn a_dock_label_that_is_a_secret_title_is_hidden_from_a_mind() {
    assert_eq!(dock_label("tokens.json - foo", "foo", true), (app_display_name("foo"), true));
    assert_eq!(dock_label("tokens.json - foo", "foo", false), ("tokens.json - foo".to_string(), false));
    assert_eq!(dock_label("notes.md - foo", "foo", true), ("notes.md - foo".to_string(), false));
    let hidden = crate::wire::dock_bar::describe_value(
        &[crate::wire::dock_bar::DescribeRow {
            app: "foo".into(), label: app_display_name("foo"), label_hidden: true, pinned: false, running: true, windows: 1, focused: true,
        }],
        4, 0, false,
    );
    assert_eq!(hidden["buttons"][0]["label_hidden"], true);
    let dock = include_str!("wire/dock_bar.rs");
    assert!(
        dock.contains("dock_label(&b.label, &b.app_id, mind)") && dock.contains("let mind = crate::control_files_mind::a_mind_is_calling();"),
        "`for_describe` asks who is calling"
    );
}
