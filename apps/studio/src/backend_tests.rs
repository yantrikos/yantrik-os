use super::*;

#[test]
fn the_same_prompt_and_seed_draw_the_same_picture() {
    let one = draw("a lighthouse in fog", 7, 64, 48);
    let two = draw("a lighthouse in fog", 7, 64, 48);
    assert_eq!(one, two);
}

#[test]
fn a_different_word_or_a_different_seed_draws_a_different_picture() {
    let base = draw("a lighthouse in fog", 7, 64, 48);
    assert_ne!(base, draw("a lighthouse in rain", 7, 64, 48));
    assert_ne!(base, draw("a lighthouse in fog", 8, 64, 48));
}

#[test]
fn the_placeholder_is_a_readable_png_of_the_size_asked_for() {
    let bytes = draw("a lighthouse", 1, 120, 80);
    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
    let (width, height) = crate::gallery::png_size(&bytes);
    assert_eq!((width, height), (Some(120), Some(80)));
}

#[test]
fn the_fake_backend_caps_a_placeholder_rather_than_drawing_thousands_of_pixels_nobody_will_see() {
    let fake = Fake::capped_at(256);
    let shot = fake
        .generate(
            &Request {
                prompt: "a very large request".into(),
                negative: String::new(),
                width: 4096,
                height: 2048,
                steps: 30,
                seed: 3,
                cfg: 7.0,
            },
            &Stop::new(),
        )
        .unwrap();
    assert_eq!((shot.width, shot.height), (256, 128));
    // And it says so, rather than letting the sidecar claim 4096x2048.
    assert_eq!(shot.sent, "256x128");
}

#[test]
fn a_cancelled_request_never_reaches_a_backend() {
    let cancel = Stop::new();
    cancel.set();
    let request = Request {
        prompt: "anything".into(),
        negative: String::new(),
        width: 64,
        height: 64,
        steps: 4,
        seed: 1,
        cfg: 7.0,
    };
    assert_eq!(
        Fake::capped_at(64).generate(&request, &cancel).unwrap_err(),
        "cancelled"
    );
    // The network backends check before they open a socket, so a cancelled generation costs
    // nothing and, for the hosted one, sends no prompt anywhere.
    let hosted = OpenAiImages::new(BackendConfig {
        api_key_env: "STUDIO_TEST_NO_SUCH_VARIABLE".into(),
        model: "gpt-image-1".into(),
        ..BackendConfig::defaults_for(Kind::OpenAiImages)
    });
    assert_eq!(hosted.generate(&request, &cancel).unwrap_err(), "cancelled");
}

#[test]
fn a_hosted_backend_with_no_key_exported_says_what_to_export() {
    std::env::remove_var("STUDIO_TEST_MISSING_KEY");
    let hosted = OpenAiImages::new(BackendConfig {
        api_key_env: "STUDIO_TEST_MISSING_KEY".into(),
        model: "gpt-image-1".into(),
        ..BackendConfig::defaults_for(Kind::OpenAiImages)
    });
    let problem = hosted
        .generate(
            &Request {
                prompt: "anything".into(),
                negative: String::new(),
                width: 64,
                height: 64,
                steps: 4,
                seed: 1,
                cfg: 7.0,
            },
            &Stop::new(),
        )
        .unwrap_err();
    // A variable that is not set is not named (it may be a key pasted where a name goes).
    assert!(problem.contains("named in studio.json's `api_key_env`") && !problem.contains("STUDIO_TEST_MISSING_KEY"), "{problem}");
}

#[test]
fn a_size_the_service_does_not_accept_is_snapped_and_both_numbers_are_kept() {
    assert_eq!(snap_to_a_supported_size(1024, 1024), "1024x1024");
    assert_eq!(snap_to_a_supported_size(512, 512), "1024x1024");
    assert_eq!(snap_to_a_supported_size(1920, 1080), "1536x1024");
    assert_eq!(snap_to_a_supported_size(1080, 1920), "1024x1536");
    assert_eq!(snap_to_a_supported_size(2000, 700), "1536x1024");
    assert_eq!(snap_to_a_supported_size(0, 0), "1024x1024");
}

#[test]
fn a_filename_from_a_server_cannot_change_what_the_next_request_asks_for() {
    assert_eq!(url_part("Studio_00001_.png"), "Studio_00001_.png");
    assert_eq!(url_part("a&b=c d.png"), "a%26b%3Dc%20d.png");
    assert_eq!(url_part("sub/folder"), "sub%2Ffolder");
}

#[test]
fn the_file_a_history_entry_points_at_is_found_whichever_node_wrote_it() {
    let history = json!({
        "abc": {
            "outputs": {
                "9": { "images": [{ "filename": "Studio_00001_.png", "subfolder": "", "type": "output" }] }
            }
        }
    });
    let found = Found::read(&history, "abc").unwrap();
    assert_eq!(found.filename, "Studio_00001_.png");
    assert_eq!(found.kind, "output");

    // Queued but not finished: the entry is there, its outputs are not. That is "still
    // working", and treating it as a failure would end every render early.
    assert!(Found::read(&json!({ "abc": { "outputs": {} } }), "abc").is_none());
    assert!(Found::read(&json!({}), "abc").is_none());

    // A preview rather than an output is still a picture, and is taken if it is all there is.
    let preview = json!({
        "abc": { "outputs": { "3": { "images": [{ "filename": "p.png", "type": "temp" }] } } }
    });
    assert_eq!(Found::read(&preview, "abc").unwrap().kind, "temp");

    // And an entry naming an empty filename is not one.
    let empty = json!({
        "abc": { "outputs": { "9": { "images": [{ "filename": "", "type": "output" }] } } }
    });
    assert!(Found::read(&empty, "abc").is_none());
}

#[test]
fn a_server_error_message_is_passed_through_rather_than_summarised_away() {
    // ComfyUI's own words for a graph it cannot run are the useful part of the failure.
    let body = r#"{"error":{"message":"Value not in list","type":"invalid_prompt"},"node_errors":{}}"#;
    let said = |body: &str| explain(ureq::Response::new(400, "Bad Request", body).unwrap());
    assert_eq!(said(body), "Value not in list");
    assert_eq!(said(r#"{"error":"not authenticated"}"#), "not authenticated");
    assert_eq!(said(""), "no explanation was sent");
    // A body that is not JSON at all is still the server's own words, trimmed to a length a
    // summary line can carry.
    assert_eq!(said("  upstream is down  "), "upstream is down");
    let shortened = said(&"x".repeat(900));
    assert!(shortened.chars().count() <= 301, "{}", shortened.chars().count());
    assert!(shortened.ends_with('…'));
}
