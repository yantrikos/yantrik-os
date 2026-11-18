//! The judges against what the framework writes, what a person's hand edit leaves, and what
//! another component's file can be made to hold.

use super::*;
use crate::settings::file::render_raw;

const APP: &str = "weather";

/// The judge's verdict on all of `bytes`, shown a [`crate::quiet::STEP`] at a time as the read
/// shows them, the way [`crate::quiet::read_own_headed`] asks.
fn verdict(judge: impl Fn(&[u8], bool) -> Head, bytes: &[u8]) -> (Head, usize) {
    let step = crate::quiet::STEP as usize;
    let probe = crate::quiet::PROBE as usize;
    let mut shown = 0;
    loop {
        shown = (shown + step).min(bytes.len()).min(probe);
        let whole = shown == bytes.len();
        match judge(&bytes[..shown], whole) {
            Head::More if !whole && shown < probe => continue,
            Head::More => return (Head::NotOurs, shown),
            head => return (head, shown),
        }
    }
}

fn ours(bytes: &[u8]) -> bool {
    verdict(settings(APP), bytes).0 == Head::Ours
}

#[test]
fn what_the_framework_writes_is_ours_from_its_first_bytes() {
    let rendered = render_raw(APP, 1, serde_json::json!({ "fahrenheit": true, "note": "x".repeat(4096) })).unwrap();
    let (head, shown) = verdict(settings(APP), &rendered);
    assert_eq!(head, Head::Ours);
    assert_eq!(shown, crate::quiet::STEP as usize, "the mark is in the first step");
    assert!(ours(br#"{"kind":"yantrik-app-settings","app":"weather","schema":1,"settings":{}}"#), "compact");
    assert!(ours(br#"{"app":"weather","kind":"yantrik-app-settings","schema":1,"settings":{}}"#), "keys sorted by hand");
    assert!(ours(b" \n\t{ \"kind\" :\r\n \"yantrik-app-settings\" , \"app\" : \"weather\" }"), "whitespace between tokens");
}

#[test]
fn the_marks_words_anywhere_but_its_place_are_not_ours() {
    let cases: [&[u8]; 12] = [
        br#"[{"kind":"yantrik-app-settings","app":"weather","password":"SENTINEL"}]"#,
        br#"{"label":"yantrik-app-settings","kind":"yantrik-app-settings","app":"weather"}"#,
        br#"{"accounts":[{"kind":"yantrik-app-settings","app":"weather"}]}"#,
        br#"{"kind":"yantrik-app-settings","label":"x","app":"weather"}"#,
        br#"{"kind":"yantrik-app-settings","app":"notes","schema":1}"#,
        br#"{"kind":"yantrik-app-settings","app":"weather2","schema":1}"#,
        br#"{"kind":"yantrik-app-settings","kind":"yantrik-app-settings","app":"weather"}"#,
        br#"{"\u006bind":"yantrik-app-settings","app":"weather"}"#,
        br#"{"kind":"yantrik-app-settings-x","app":"weather"}"#,
        br#"{"kind":"yantrik-app-settings","app":"weather"]"#,
        b"\xef\xbb\xbf{\"kind\":\"yantrik-app-settings\",\"app\":\"weather\"}",
        b"",
    ];
    for (n, bytes) in cases.iter().enumerate() {
        assert!(!ours(bytes), "case {n}: {}", String::from_utf8_lossy(bytes));
    }
}

#[test]
fn a_file_that_is_not_ours_is_told_within_its_first_step() {
    let account = format!("[{}]", r#"{"id":"default","password":"SENTINEL"},"#.repeat(10_000));
    assert_eq!(verdict(settings(APP), account.as_bytes()), (Head::NotOurs, crate::quiet::STEP as usize));
    // Whitespace before the mark is room only up to the probe.
    let padded = format!("{}{{\"kind\":\"yantrik-app-settings\",\"app\":\"weather\"}}", " ".repeat(300));
    assert_eq!(verdict(settings(APP), padded.as_bytes()).0, Head::NotOurs);
}

#[test]
fn a_temp_cut_short_is_ours_only_while_it_is_the_start_of_the_mark() {
    let rendered = render_raw(APP, 1, serde_json::json!({})).unwrap();
    for cut in [0, 1, 5, 20, 40, rendered.len() / 2, rendered.len()] {
        assert_eq!(verdict(temp(APP), &rendered[..cut]).0, Head::Ours, "cut at {cut}");
        if cut < 40 {
            assert_eq!(verdict(settings(APP), &rendered[..cut]).0, Head::NotOurs, "a settings read never takes a cut file, at {cut}");
        }
    }
    for bytes in [&b"someone else's"[..], br#"{"kind":"SENTINEL"#, br#"{"kind":"yantrik-app-settings","app":"note"#, br#"[{"password":"SENTINEL"}]"#] {
        assert_eq!(verdict(temp(APP), bytes).0, Head::NotOurs, "{}", String::from_utf8_lossy(bytes));
    }
}

#[test]
fn the_old_place_is_judged_by_the_shapes_the_app_wrote_there() {
    let legacy = ["fahrenheit", "active", "locations"];
    let judge = || former(APP, &legacy);
    let taken: [&[u8]; 5] = [
        b"{\n  \"schema\": 1,\n  \"settings\": {\n    \"fahrenheit\": true\n  }\n}",
        br#"{"settings":{},"schema":1}"#,
        br#"{"locations":[{"name":"Oslo","lat":59.9,"lon":10.7}],"fahrenheit":true}"#,
        br#"{"fahrenheit": false}"#,
        br#"{"kind":"yantrik-app-settings","app":"weather","schema":1,"settings":{}}"#,
    ];
    let expected = [Head::Ours, Head::NotOurs, Head::Ours, Head::Ours, Head::Ours];
    for ((bytes, want), n) in taken.iter().zip(expected).zip(0..) {
        assert_eq!(verdict(judge(), bytes).0, want, "case {n}: {}", String::from_utf8_lossy(bytes));
    }
    let refused: [&[u8]; 6] = [
        br#"[{"id":"default","password":"SENTINEL"}]"#,
        br#"{"password":"SENTINEL","fahrenheit":true}"#,
        br#"{"schema":"1","settings":{}}"#,
        br#"{"schema":1,"accounts":[]}"#,
        br#"{"schema":1,"settings":[]}"#,
        br#"{"kind":"yantrik-app-settings","app":"notes"}"#,
    ];
    for bytes in refused {
        assert_eq!(verdict(judge(), bytes).0, Head::NotOurs, "{}", String::from_utf8_lossy(bytes));
    }
    assert_eq!(verdict(former(APP, &[]), br#"{"fahrenheit": false}"#).0, Head::NotOurs, "no legacy keys: no legacy shape");
}
