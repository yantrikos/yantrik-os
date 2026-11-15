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
        // Review R6-L1: a conflict label of eleven words.
        "tokens (Ann Marie de la Cruz's conflicted copy 2026-11-15).json - gedit",
        // Review R6-L2: a public key's name with a copy marker is no public key.
        "client_secret.pub - Copy.json - gedit",
        // Review R6-L3: an editor's copy of a protected file, and a path with a space before its protected part.
        "%home%ann%.kube%config.swp - less", "#!home!ann!.kube!config# - emacs", "/home/ann/My Backups/.kube/config - nvim",
        "config (~/My Stuff/.kube) - NVIM",
        // Review R7-H1: an editor's copy with a space in its path, judged a word at a time.
        "%home%ann%my stuff%.kube%config.swp - less", "#!home!ann!my stuff!.kube!config# - emacs",
        // Review R7-L3: a protected place with ` - ` in its name.
        "/home/ann/.config/Code - OSS/User/History/-1a/x.json - x", "~/.config/Code - OSS/Backups/1 - Code",
    ] {
        let shown = row(&window_entry("gedit", title), true);
        assert_eq!(shown["title_hidden"], true, "{title}");
        assert!(!shown.to_string().contains("tokens") && !shown.to_string().contains(".aws") && !shown.to_string().contains("pem"), "{title}: {shown}");
        assert_eq!(row(&window_entry("gedit", title), false)["title"], title, "the person sees it whole: {title}");
    }
    for title in ["“notes.md” Properties", "Opening notes.md.", "~/Documents - Files", "/home/ann/Documents/notes.md - nano", "id_rsa.pub… - gedit", "Budget – 2026 — Calc",
        "notes (copy 1).md - gedit", "Chapter 2.md - gedit", "Data Science (1).pdf - Viewer", "id_rsa.pub - gedit", "id_rsa.pub (copy 1) - gedit",
        "#!home!ann!documents!notes.md# - emacs", "/home/ann/My Backups/notes.md - nvim", "config (~/My Stuff/Documents) - NVIM",
        "%home%ann%my stuff%notes.md.swp - less", "%a %b - Firefox", "~/.config/Code - OSS/User/settings.json - Code",
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
    assert_eq!(dock_label("tokens.json - foo", "foo", true), ("foo".to_string(), app_display_name("foo"), true));
    assert_eq!(dock_label("tokens.json - foo", "foo", false), ("foo".to_string(), "tokens.json - foo".to_string(), false));
    assert_eq!(dock_label("notes.md - foo", "foo", true), ("foo".to_string(), "notes.md - foo".to_string(), false));
    let hidden = crate::wire::dock_bar::describe_value(
        &[crate::wire::dock_bar::DescribeRow {
            app: "foo".into(), label: app_display_name("foo"), label_hidden: true, pinned: false, running: true, windows: 1, focused: true,
        }],
        4, 0, false,
    );
    assert_eq!(hidden["buttons"][0]["label_hidden"], true);
    let dock = include_str!("wire/dock_bar.rs");
    assert!(
        dock.contains("dock_label(&b.label, &b.app_id, mind)") && dock.contains("                app,\n") && dock.contains("let mind = crate::control_files_mind::a_mind_is_calling();"),
        "`for_describe` asks who is calling"
    );
}

/// Review R6-M1: a window that declares no app id gets one from its title, so the id names the
/// secret as the title did. A mind is given no part of it, in a row, a cell, `in_front` or a dock
/// button; the person's own view is whole.
#[test]
fn an_app_id_derived_from_a_secret_title_is_hidden_from_a_mind() {
    // The last two are secrets by more than their first word (review R7-L1): the id is `tokens` and `local`.
    for title in ["tokens.json - foo", "id_ed25519 - foo", "tokens (copy 1).json - foo", "Local State - foo"] {
        let window = window_entry("", title);
        assert!(title.to_lowercase().starts_with(&window.app_id), "the id is derived from the title: {}", window.app_id);
        let shown = [
            row(&window, true).to_string(),
            cell(title, title, &window.app_id, true).to_string(),
            format!("{:?}", in_front(Some(title.to_string()), std::slice::from_ref(&window), true)),
            format!("{:?}", dock_label(title, &window.app_id, true)),
        ];
        for json in shown {
            assert!(!json.to_lowercase().contains("tokens") && !json.contains("id_") && !json.to_lowercase().contains("id ed"), "{title}: {json}");
            assert!(!json.to_lowercase().contains(&window.app_id), "{title}: {json}");
        }
        assert_eq!(row(&window, true)["app"], "window");
        assert_eq!(row(&window, false)["app"], window.app_id.as_str(), "the person sees the id whole");
        assert_eq!(dock_label(title, &window.app_id, false), (window.app_id.clone(), title.to_string(), false));
    }
    assert_eq!(shown_app("gedit", "notes.md - gedit", true), ("gedit".to_string(), app_display_name("gedit")));
    assert_eq!(shown_app("tokens.json", "tokens.json - foo", false), ("tokens.json".to_string(), app_display_name("tokens.json")));
}

/// Review R7-H1: a title of many short words cost seconds to minutes (100,000 runs, each reading the
/// filesystem). Each is judged in under 50 ms, best of three. The judge is pure (`is_listed_place`
/// reads no filesystem and no `HOME` but the `~/` of a path), so the test sets nothing: an
/// environment variable set in a test races the tests that read it. The unfixed filter took 2 to 7
/// seconds a title with no `HOME` at all, and more with one.
#[test]
fn a_four_kilobyte_title_of_short_words_is_judged_in_no_time() {
    for title in [
        "%a ".repeat(1365), "#%a# ".repeat(819), "!a!b ".repeat(819), "%.ssh ".repeat(682), "a/b ".repeat(1024),
    ] {
        let took = (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                let hidden = hides(&title, true);
                assert!(!hidden, "no file has such a name: {}", &title[..12]);
                start.elapsed()
            })
            .min()
            .unwrap_or_default();
        assert!(took < std::time::Duration::from_millis(50), "{}: {took:?}", &title[..12]);
    }
}

/// Review R6-M2: a page's title of stacked markers cost cubic time per `describe shell`. Runs over
/// 255 bytes are not judged, and each name below it is judged once. The bound is the best of three
/// runs under 50 ms: the fixed filter takes microseconds, the unfixed one seconds.
#[test]
fn a_four_kilobyte_title_of_stacked_markers_is_judged_in_no_time() {
    for title in [format!("%{}", "~".repeat(4095)), format!("!{}", "~".repeat(4095)), format!("%{}", ":x".repeat(512)), format!("%{} \u{2014} Mozilla Firefox", "~".repeat(4090))] {
        let took = (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                let hidden = hides(&title, true);
                assert!(!hidden, "no file has such a name");
                start.elapsed()
            })
            .min()
            .unwrap_or_default();
        assert!(took < std::time::Duration::from_millis(50), "{took:?}");
    }
}
