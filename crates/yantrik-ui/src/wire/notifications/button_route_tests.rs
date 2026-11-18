//! The tests of how a notification button is routed to its app, and that it is carried as app.act
//! and never as a press. Separate from notifications.rs, which is over the file-size limit, so that
//! file does not grow.

use super::button_route;

fn installed() -> Vec<crate::apps::DesktopEntry> {
    let mut installed = crate::surfaces::shipped_catalogue();
    installed.push(
        crate::apps::parse_desktop_text(
            "org.example.Mailer",
            "[Desktop Entry]\nType=Application\nName=Mailer\nExec=/usr/bin/mailer\n\
             X-Yantrik-Surface=mailer\nX-Yantrik-Aliases=post\n",
        )
        .unwrap(),
    );
    installed.push(
        crate::apps::parse_desktop_text(
            "firefox",
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=/usr/bin/firefox %u\n",
        )
        .unwrap(),
    );
    installed
}

#[test]
fn a_button_reaches_any_app_that_declares_a_surface() {
    let installed = installed();
    let route = |app: &str| button_route(app, &installed);
    // Ours, under the names they send as.
    assert_eq!(route("Downloads"), Some(("download-manager".into(), Some("downloads".into()))));
    assert_eq!(route("Calendar"), Some(("calendar".into(), Some("calendar".into()))));
    assert_eq!(route("container-manager"), Some(("containers".into(), Some("container-manager".into()))));
    // Somebody else's, by its Name and by its alias, opened by that name when it is closed.
    assert_eq!(route("Mailer"), Some(("mailer".into(), Some("mailer".into()))));
    assert_eq!(route("post"), Some(("mailer".into(), Some("post".into()))));
    // The desktop's own buttons reach the desktop, which nothing opens.
    assert_eq!(route("Yantrik"), Some(("shell".into(), None)));
    // A program that declared nothing did not route this notification: no surface to call.
    assert_eq!(route("Firefox"), None);
    assert_eq!(route("browser"), None, "the browser publishes nothing");
    assert_eq!(route("no-such-app"), None);
}

/// A notification button is a poster's own words: `{app: "email", actions: [{id:
/// "open_message_id", args: {...}}]}` from any same-user process or a Mind. `forward_to_app`
/// carries it to the app as the shell, so it must only ever send `app.act`, which refuses an
/// action only the shell's own press runs, and never `app.press`.
#[test]
fn a_notification_button_is_carried_as_app_act_and_never_as_a_press() {
    let src = include_str!("../notifications.rs");
    let code = src.split("#[cfg(test)]").next().unwrap();
    let at = &code[code.find("fn forward_to_app(").expect("fn forward_to_app")..];
    let body = &at[..at.find("\n}\n").unwrap()];
    assert!(body.contains(".call(\"app.act\""), "{body}");
    assert!(!body.contains("press"), "{body}");
    assert!(!body.contains("app_act::"), "the shell's own press is not what a button does: {body}");
}
