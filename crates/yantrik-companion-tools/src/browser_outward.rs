//! Which browser acts send something out, and what to tell the person about them (#1054).
//!
//! A click can be a link or the "Place order" button, and a keystroke can be a letter or the Enter
//! that submits a form. The companion's browser tools used to do either with nothing in front of
//! the person. Reading a page and moving around in it are not outward; pressing a control that
//! submits or acts is, so before one is pressed the page is asked what it is: a button (or a
//! submit input, or a control with a button role), or anything inside a form, is an act and goes
//! through [`crate::outward`] with the page, the control, and the form's method, target and field
//! values. A link, a scroll, or typing into a field without submitting is not.
//!
//! The values shown are the page's own at the moment of asking; a password field shows only that
//! it holds something. The shell shows the card whole or refuses it, and the grant is bound to
//! the values shown.

use crate::outward::{ask_person_before_acting, OutwardAct, Permit};

/// JavaScript that describes the control `pick` evaluates to (`null` if there is none) as JSON:
/// whether it acts, and what the person should see.
pub(crate) fn probe_js(pick: &str) -> String {
    format!(
        r#"(() => {{
  const el = ({pick});
  if (!el) return JSON.stringify({{ found: false }});
  const tag = el.tagName.toLowerCase();
  const type = (el.getAttribute('type') || '').toLowerCase();
  const role = (el.getAttribute('role') || '').toLowerCase();
  const form = el.closest('form');
  const isButton = tag === 'button' || role === 'button'
    || (tag === 'input' && ['submit', 'button', 'image', 'reset'].includes(type));
  const isLink = tag === 'a' && !form && role !== 'button';
  const acts = !isLink && (isButton || (!!form && ['a', 'button', 'input', 'select'].includes(tag) && type !== 'text'));
  const label = ((el.textContent || el.value || el.getAttribute('aria-label') || '').trim() || tag).replace(/\s+/g, ' ');
  let fields = [];
  if (form) {{
    for (const f of Array.from(form.elements)) {{
      const name = f.name || f.id;
      if (!name || f.disabled || ['submit', 'button', 'image', 'reset', 'file'].includes((f.type || '').toLowerCase())) continue;
      const t = (f.type || '').toLowerCase();
      if ((t === 'checkbox' || t === 'radio') && !f.checked) continue;
      const v = t === 'password' ? '(password, ' + (f.value || '').length + ' characters)' : String(f.value || '');
      fields.push(name + '=' + v.replace(/\s+/g, ' '));
    }}
  }}
  return JSON.stringify({{
    found: true, acts, label,
    url: location.href,
    form: form ? ((form.method || 'get').toUpperCase() + ' ' + (form.action || location.href)) : '',
    fields,
  }});
}})()"#
    )
}

/// What the person is told for a control that acts, from [`probe_js`]'s answer: `None` if the
/// control does not (a link, a field), or if the page said nothing usable.
pub(crate) fn act_from_probe(probe: &str) -> Option<(String, String, String)> {
    let v: serde_json::Value = serde_json::from_str(probe).ok()?;
    if !v["found"].as_bool().unwrap_or(false) || !v["acts"].as_bool().unwrap_or(false) {
        return None;
    }
    let url = v["url"].as_str().unwrap_or("");
    let form = v["form"].as_str().unwrap_or("");
    let to = if form.is_empty() { url.to_string() } else { format!("{url} (form: {form})") };
    let fields: Vec<&str> = v["fields"].as_array().map(|a| a.iter().filter_map(|f| f.as_str()).collect()).unwrap_or_default();
    let what = if fields.is_empty() {
        format!("press '{}'", v["label"].as_str().unwrap_or(""))
    } else {
        format!("press '{}' with {}", v["label"].as_str().unwrap_or(""), fields.join(", "))
    };
    Some((to, what, String::new()))
}

/// Ask before a press or click on the control `pick` evaluates to, if that control acts. `eval`
/// runs JavaScript in the page. `Ok` carries the permit to `finish` after the click (nothing to
/// record when the control only navigates or the person was asked); `Err` is the answer to give the
/// model and the click must not be made.
pub(crate) fn before_press(
    eval: &mut dyn FnMut(&str) -> Result<String, String>,
    pick: &str,
    tool: &str,
) -> Result<Permit, String> {
    // A page that cannot say what the control is does not get the benefit of the doubt: that is
    // asked about as an act, with what little is known.
    let (to, what, more) = match eval(&probe_js(pick)) {
        Ok(probe) if serde_json::from_str::<serde_json::Value>(&probe).is_ok_and(|v| v["found"] == false) => {
            return Ok(Permit::asked()); // nothing there to press; the click will say so
        }
        Ok(probe) => match act_from_probe(&probe) {
            Some(act) => act,
            None if serde_json::from_str::<serde_json::Value>(&probe).is_ok() => return Ok(Permit::asked()),
            None => unknown_control(),
        },
        Err(_) => unknown_control(),
    };
    ask_person_before_acting(&OutwardAct { tool, to: &to, what: &what, more: &more })
}

fn unknown_control() -> (String, String, String) {
    ("a page that did not describe the control".to_string(), "press a control the page could not describe".to_string(), String::new())
}

/// Ask before typing that submits (Enter, or the form's submit) into a field, whichever way the
/// tool picks it. `target` says which field, for the card.
pub(crate) fn before_submit(
    eval: &mut dyn FnMut(&str) -> Result<String, String>,
    tool: &str,
    target: &str,
    text: &str,
) -> Result<Permit, String> {
    let url = eval("location.href").unwrap_or_default();
    let to = format!("{} (field: {target})", url.trim_matches('"'));
    ask_person_before_acting(&OutwardAct { tool, to: &to, what: &format!("type '{text}' and submit"), more: "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(acts: bool, form: &str, fields: &[&str]) -> String {
        serde_json::json!({ "found": true, "acts": acts, "label": "Place order", "url": "https://shop.example/cart",
                            "form": form, "fields": fields })
        .to_string()
    }

    #[test]
    fn a_link_or_a_field_is_not_an_act_and_a_button_is() {
        assert!(act_from_probe(&probe(false, "", &[])).is_none());
        assert!(act_from_probe("{\"found\":false}").is_none());
        assert!(act_from_probe("not json").is_none());
        let (to, what, _) = act_from_probe(&probe(true, "POST https://shop.example/buy", &["qty=2", "card=(password, 16 characters)"])).unwrap();
        assert_eq!(to, "https://shop.example/cart (form: POST https://shop.example/buy)");
        assert_eq!(what, "press 'Place order' with qty=2, card=(password, 16 characters)");
    }

    #[test]
    fn the_probe_script_decides_by_the_controls_kind() {
        let js = probe_js("document.querySelector('#go')");
        for needle in ["tag === 'button'", "role === 'button'", "'submit'", "closest('form')", "(password, "] {
            assert!(js.contains(needle), "{needle}");
        }
        assert!(js.contains("document.querySelector('#go')"));
    }

    #[cfg(unix)]
    mod asking {
        use super::*;
        use crate::outward_testing::{actions, first_card, installed, shown};

        fn page(probe: String) -> impl FnMut(&str) -> Result<String, String> {
            move |js: &str| if js.contains("JSON.stringify") { Ok(probe.clone()) } else { Ok("https://shop.example/cart".to_string()) }
        }

        #[test]
        fn pressing_a_button_in_a_form_raises_a_card_with_the_page_the_form_and_its_fields_and_a_no_presses_nothing() {
            let (_shell, heard) = installed("br-no", "pending", "denied");
            let probe = probe(true, "POST https://shop.example/buy", &["qty=2", "card=(password, 16 characters)"]);

            let refusal = before_press(&mut page(probe), "document.querySelector('#buy')", "browser_click").unwrap_err();

            assert!(refusal.contains("said no") && refusal.contains("Nothing was done"), "{refusal}");
            let card = first_card(&heard);
            assert_eq!(card["args_json"]["tool"], "browser_click");
            assert_eq!(shown(&card, "to"), "https://shop.example/cart (form: POST https://shop.example/buy)");
            assert_eq!(shown(&card, "what"), "press 'Place order' with qty=2, card=(password, 16 characters)");
            assert!(!actions(&heard).contains(&"consume_approval".to_string()));
        }

        #[test]
        fn a_link_is_pressed_without_a_card() {
            let (_shell, heard) = installed("br-link", "pending", "denied");
            before_press(&mut page(probe(false, "", &[])), "document.querySelector('a')", "browser_click").expect("a link needs no card");
            assert!(heard.lock().unwrap().is_empty(), "a card was raised for a link");
        }

        #[test]
        fn a_page_that_cannot_describe_the_control_is_asked_about_not_waved_through() {
            let (_shell, heard) = installed("br-unknown", "pending", "denied");
            let mut broken = |_: &str| -> Result<String, String> { Err("JS exception".to_string()) };
            let refusal = before_press(&mut broken, "document.querySelector('#x')", "browser_click_xy").unwrap_err();
            assert!(refusal.contains("said no"), "{refusal}");
            assert!(shown(&first_card(&heard), "what").contains("could not describe"));
        }

        #[test]
        fn an_allow_for_a_button_is_spent_once() {
            let (_shell, heard) = installed("br-yes", "pending", "granted");
            before_press(&mut page(probe(true, "", &[])), "document.querySelector('button')", "browser_click_element").expect("allowed");
            assert_eq!(actions(&heard).iter().filter(|a| *a == "consume_approval").count(), 1);
        }

        #[test]
        fn typing_that_submits_raises_a_card_with_the_page_the_field_and_the_text_and_a_no_types_nothing() {
            let (_shell, heard) = installed("br-type", "pending", "denied");
            let refusal = before_submit(&mut page(String::new()), "browser_type", "#search", "my card number; and more").unwrap_err();
            assert!(refusal.contains("said no"), "{refusal}");
            let card = first_card(&heard);
            assert_eq!(shown(&card, "to"), "https://shop.example/cart (field: #search)");
            assert_eq!(shown(&card, "what"), "type 'my card number; and more' and submit");
        }
    }
}
