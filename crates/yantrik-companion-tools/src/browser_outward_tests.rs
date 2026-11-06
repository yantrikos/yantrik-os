//! Tests of `browser_outward.rs`: the card, the asking, and the press checked against it.
use super::*;

fn described(kind: &str, acts: bool, destination: &str, fields: &[&str]) -> String {
    serde_json::json!({ "acts": acts, "kind": kind, "label": "Place order", "url": "https://shop.example/cart",
                        "destination": destination, "method": if destination.is_empty() { "" } else { "post" },
                        "fields": fields, "typing": false })
    .to_string()
}

/// A page that answers from a script: each describe gets the next canned description, and a
/// press answers as told.
struct Fake {
    describes: Vec<Result<String, String>>,
    press: String,
    pressed: bool,
    focused: bool,
    /// What each press was told to expect.
    expected: Vec<serde_json::Value>,
}

impl Page for Fake {
    fn on_element(&mut self, _pick: &Pick, function_src: &str, args: &[serde_json::Value]) -> Result<String, String> {
        if function_src.contains("this.click()") || function_src.contains("requestSubmit") {
            self.pressed = true;
            self.expected.push(args.first().cloned().unwrap_or_default());
            return Ok(self.press.clone());
        }
        if function_src.contains("this.focus(); return 'focused'") {
            self.focused = true;
            return Ok("focused".into());
        }
        if self.describes.len() > 1 { self.describes.remove(0) } else { self.describes[0].clone() }
    }
}

fn fake(describes: Vec<Result<String, String>>) -> Fake {
    Fake { describes, press: r#"{"pressed":true,"label":"Place order"}"#.to_string(), pressed: false, focused: false, expected: vec![] }
}

#[test]
fn the_description_script_asks_by_default_and_exempts_only_links_and_text_fields() {
    let js = DESCRIBE_FN;
    // The default: acts unless it is a plain link or a text field.
    assert!(js.contains("const acts = typing ? true : !(plainLink || isField);"));
    // Climbs from the inner node to its control, anchor and form, by the prototype's own methods.
    for needle in ["closest(el, 'a[href]')", "closest(el, 'button, input[type=submit], input[type=image], [role=button]')", "closest(el, 'form')", "E.closest.call"] {
        assert!(js.contains(needle), "{needle}");
    }
    // What the page cannot be seen into asks, and handlers or framework methods make a link not plain.
    for needle in ["'iframe'", "el.shadowRoot", "'onclick'", "has(anchor, 'download')", "data-(turbo-)?method", "(data-)?hx-"] {
        assert!(js.contains(needle), "{needle}");
    }
    // The destination: the submitter's only where it has the attribute, else the form's, read
    // through the prototype so `<input name=action>` cannot stand in (review M1).
    for needle in ["has(submitter, 'formaction')", "has(submitter, 'formmethod')", "formAction.call(form)", "formElements.call(form)", "(password, "] {
        assert!(js.contains(needle), "{needle}");
    }
    assert!(press_fn().contains("now !== expected") && press_fn().contains("this.click()"));
    assert!(type_fn().contains("now !== expected") && type_fn().contains("F.requestSubmit.call(form, def)"));
}

#[test]
fn the_card_names_the_page_the_destination_and_each_field_quoted_on_its_own() {
    let d = Described::parse(&described("button", true, "https://elsewhere.example/buy", &["qty=2", "note=, card=(password, 0 characters)"])).unwrap();
    let (to, what) = d.card(None);
    assert_eq!(to, "https://shop.example/cart -> POST https://elsewhere.example/buy");
    assert_eq!(what, r#"press "Place order" sending ["qty=2", "note=, card=(password, 0 characters)"]"#);
    let d = Described::parse(&described("opaque", true, "", &[])).unwrap();
    assert!(d.card(None).0.contains("inside a frame or component the page controls"));
    let d = Described::parse(&described("field", true, "https://elsewhere.example/post", &["q=hi"])).unwrap();
    assert_eq!(d.card(Some("hi")).1, r#"type "hi" into "Place order" and submit sending ["q=hi"]"#);
}

#[cfg(unix)]
mod asking {
    use super::*;
    use crate::outward_testing::{actions, first_card, installed, shown};

    #[test]
    fn a_button_raises_a_card_with_the_real_destination_and_a_no_presses_nothing() {
        let (_shell, heard) = installed("br-no", "pending", "denied");
        let mut page = fake(vec![Ok(described("button", true, "https://elsewhere.example/buy", &["qty=2"]))]);
        let refusal = ask_before_press(&mut page, &Pick::Selector("#buy".into()), "browser_click").err().expect("a no");
        assert!(refusal.contains("said no") && refusal.contains("Nothing was done"), "{refusal}");
        let card = first_card(&heard);
        assert_eq!(shown(&card, "to"), "https://shop.example/cart -> POST https://elsewhere.example/buy");
        assert!(!page.pressed && !actions(&heard).contains(&"consume_approval".to_string()));
    }

    #[test]
    fn a_plain_link_and_a_text_field_need_no_card_and_everything_else_does() {
        for (kind, acts, asked) in [("link", false, false), ("field", false, false), ("button", true, true), ("other", true, true), ("opaque", true, true)] {
            let (_shell, heard) = installed(&format!("br-{kind}"), "pending", "denied");
            let mut page = fake(vec![Ok(described(kind, acts, "", &[]))]);
            let result = ask_before_press(&mut page, &Pick::Point(1.0, 2.0), "browser_click_xy");
            assert_eq!(result.is_err(), asked, "{kind}");
            assert_eq!(!heard.lock().unwrap().is_empty(), asked, "{kind}: a card was or was not raised");
        }
    }

    /// The point H1 made: typing at a point asks when the point is not a text field.
    #[test]
    fn typing_at_a_point_that_is_a_button_asks_even_without_submit() {
        let (_shell, heard) = installed("br-xy-button", "pending", "denied");
        let mut page = fake(vec![Ok(described("button", true, "", &[]))]);
        assert!(ask_before_typing(&mut page, &Pick::Point(5.0, 5.0), "browser_type_xy", "a", false, true).is_err());
        assert!(!heard.lock().unwrap().is_empty());
        let (_shell, heard) = installed("br-xy-field", "pending", "denied");
        let mut page = fake(vec![Ok(described("field", false, "", &[]))]);
        assert!(ask_before_typing(&mut page, &Pick::Point(5.0, 5.0), "browser_type_xy", "a", false, true).is_ok());
        assert!(heard.lock().unwrap().is_empty());
    }

    #[test]
    fn typing_that_submits_always_asks_and_shows_the_form_it_sends() {
        let (_shell, heard) = installed("br-submit", "pending", "denied");
        let typing = serde_json::json!({ "acts": true, "kind": "field", "label": "q", "url": "https://bank.example/", "destination": "https://evil.example/collect",
                                         "method": "post", "fields": ["q=my card number", "token=abc"], "typing": true }).to_string();
        let mut page = fake(vec![Ok(typing)]);
        assert!(ask_before_typing(&mut page, &Pick::Selector("#q".into()), "browser_type", "my card number", true, true).is_err());
        let card = first_card(&heard);
        assert_eq!(shown(&card, "to"), "https://bank.example/ -> POST https://evil.example/collect");
        assert!(shown(&card, "what").contains("token=abc"));
    }

    #[test]
    fn a_page_that_cannot_be_described_is_asked_about_and_then_pressed_the_old_way() {
        let (_shell, heard) = installed("br-unknown", "pending", "granted");
        let mut page = fake(vec![Err("cannot be done here".into())]);
        let pressing = ask_before_press(&mut page, &Pick::Selector("#x".into()), "browser_click").expect("allowed");
        assert!(pressing.was_asked());
        assert!(shown(&first_card(&heard), "what").contains("could not describe"));
        page.press = "not json".into();
        assert!(matches!(pressing.press(&mut page, &Pick::Selector("#x".into())), Outcome::Unavailable));
    }

    #[test]
    fn nothing_there_needs_no_card() {
        let (_shell, heard) = installed("br-gone", "pending", "denied");
        let mut page = fake(vec![Err(NOT_FOUND.into())]);
        assert!(ask_before_press(&mut page, &Pick::Selector("#gone".into()), "browser_click").is_ok());
        assert!(heard.lock().unwrap().is_empty());
    }

    #[test]
    fn an_allowed_press_is_made_once_and_a_page_that_changed_is_not_pressed() {
        let (_shell, heard) = installed("br-allow", "pending", "granted");
        let mut page = fake(vec![Ok(described("button", true, "https://shop.example/buy", &["qty=1"]))]);
        let pressing = ask_before_press(&mut page, &Pick::Selector("#buy".into()), "browser_click").expect("allowed");
        assert_eq!(actions(&heard).iter().filter(|a| *a == "consume_approval").count(), 1);
        assert!(matches!(pressing.press(&mut page, &Pick::Selector("#buy".into())), Outcome::Done(s) if s == "Clicked: Place order"));
        assert!(page.pressed);

        // The same press when the page's script says the element no longer matches.
        let (_shell, _heard) = installed("br-changed", "pending", "granted");
        let mut page = fake(vec![Ok(described("button", true, "https://shop.example/buy", &["qty=1"]))]);
        let pressing = ask_before_press(&mut page, &Pick::Selector("#buy".into()), "browser_click").expect("allowed");
        page.press = r#"{"pressed":false}"#.into();
        assert!(matches!(pressing.press(&mut page, &Pick::Selector("#buy".into())), Outcome::Changed(s) if s.contains("nothing was pressed")));

        // A press by the mouse at a point re-describes first and refuses if it differs.
        let (_shell, _heard) = installed("br-xy", "pending", "granted");
        let first = described("button", true, "https://shop.example/buy", &["qty=1"]);
        let moved = described("button", true, "https://evil.example/buy", &["qty=1"]);
        let mut page = fake(vec![Ok(first.clone()), Ok(first), Ok(moved)]);
        let pressing = ask_before_press(&mut page, &Pick::Point(3.0, 4.0), "browser_click_xy").expect("allowed");
        assert!(pressing.confirm(&mut page, &Pick::Point(3.0, 4.0)).is_ok());
        assert!(pressing.confirm(&mut page, &Pick::Point(3.0, 4.0)).is_err(), "the page moved the control");
    }

    /// Review M3: a point described as a link (no card) or a button (a card) is pressed in the
    /// isolated world on the element described, never by the mouse, which the page can redirect.
    /// Only a frame's inside goes to the mouse, and a re-describe that fails refuses (fail closed).
    #[test]
    fn a_described_point_is_pressed_on_its_element_and_only_a_frame_goes_to_the_mouse() {
        let pt = Pick::Point(3.0, 4.0);
        let link = described("link", false, "", &[]);
        let (_shell, _heard) = installed("xy-link", "pending", "denied");
        let mut page = fake(vec![Ok(link.clone())]);
        let pressing = ask_before_press(&mut page, &pt, "browser_click_xy").expect("a link needs no card");
        assert!(matches!(pressing.at_point(&mut page, &pt, false, false), AtPoint::Done(_)));
        assert!(page.pressed);
        assert_eq!(page.expected, vec![serde_json::Value::from(link)], "no card, but checked against what was described (L1)");

        let (_shell, _heard) = installed("xy-frame", "pending", "granted");
        let frame = described("opaque", true, "", &[]);
        let mut page = fake(vec![Ok(frame.clone()), Ok(frame), Err("JS exception".into())]);
        let pressing = ask_before_press(&mut page, &pt, "browser_click_xy").expect("allowed");
        assert!(matches!(pressing.at_point(&mut page, &pt, false, false), AtPoint::Mouse));
        assert!(matches!(pressing.at_point(&mut page, &pt, false, false), AtPoint::Refused(_)), "a describe that fails is not pressed");
        assert!(!page.pressed);

        let (_shell, _heard) = installed("xy-field", "pending", "denied");
        let mut page = fake(vec![Ok(described("field", false, "", &[]))]);
        let pressing = ask_before_typing(&mut page, &pt, "browser_type_xy", "hi", false, true).expect("a field needs no card");
        assert!(matches!(pressing.at_point(&mut page, &pt, true, false), AtPoint::Done(_)) && page.focused);

        let (_shell, _heard) = installed("xy-nothing", "pending", "denied");
        let mut page = fake(vec![Err(NOT_FOUND.into())]);
        let pressing = ask_before_press(&mut page, &pt, "browser_click_xy").expect("nothing there");
        assert!(matches!(pressing.at_point(&mut page, &pt, false, false), AtPoint::Refused(_)) && !page.pressed);
    }
}
