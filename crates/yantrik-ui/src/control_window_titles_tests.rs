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
