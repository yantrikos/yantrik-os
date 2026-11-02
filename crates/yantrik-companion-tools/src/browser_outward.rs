//! Which browser acts send something out, and what the person is shown before they do (#1054,
//! #1063).
//!
//! A click can be a link or the "Place order" button, and a keystroke can be a letter or the Enter
//! that submits a form. The rule here is the safe one: **a click asks unless it is provably a plain
//! navigation link, or a text field being focused.** Nothing enumerates what counts as "acting":
//! a button, a `div` with a handler, the `span` inside a button, an `iframe` or a shadow-DOM
//! component (whose inside cannot be seen) all ask. Typing that submits always asks.
//!
//! Three things keep the card honest:
//!
//! - **The page does not write it.** The control is described by a function run in an *isolated
//!   world* (`Page.createIsolatedWorld`): the page's own scripts cannot reach its globals or its
//!   prototypes, so a hostile page cannot override `closest` or `JSON.stringify` to report its own
//!   submit button as harmless.
//! - **What is pressed is what was shown.** The description is the card's content and its
//!   fingerprint. After the Allow, the press is a single evaluation that describes the same element
//!   again, compares, and only then clicks (or types and submits) it; a page that changed while the
//!   person decided is not pressed, and the answer says so, so the model looks again and asks again
//!   for what is there now.
//! - **The card names where it goes and what goes.** A form's destination is its resolved `action`
//!   (or the pressed button's `formaction`), its method, and every field it will send, hidden ones
//!   included, each quoted; a password shows only its length.
//!
//! Where the page cannot be described at all (no isolated world, a script error) the click still
//! asks, as "a control the page could not describe", and is then made the way it always was.

use crate::outward::{ask_person_before_acting, OutwardAct, Permit};

/// Which element an act is aimed at.
#[derive(Clone, Debug)]
pub(crate) enum Pick {
    /// `document.querySelector(selector)`.
    Selector(String),
    /// The `n`th element (1-based) of the last `browser_snapshot`.
    Index(u64),
    /// `document.elementFromPoint(x, y)`.
    Point(f64, f64),
    /// `document.activeElement`.
    Active,
}

/// A page the tools can describe and press controls on.
pub(crate) trait Page {
    /// Run `function_src` (a JavaScript function expression using `this`) on the element `pick`
    /// resolves to, in a world the page's own scripts cannot reach, with `args` as its arguments.
    /// The answer is the function's string result. `Err` is "no such element" or "cannot be done
    /// here"; the two are told apart by [`NOT_FOUND`].
    fn on_element(&mut self, pick: &Pick, function_src: &str, args: &[serde_json::Value]) -> Result<String, String>;
}

/// What [`Page::on_element`] says when nothing is at the pick.
pub(crate) const NOT_FOUND: &str = "no such element";

/// Describes the element as `this`. `typed` (an argument, or null) is text about to be typed into
/// it and submitted: the description then covers the form as it will be sent. Returns JSON.
pub(crate) const DESCRIBE_FN: &str = r#"function (typed) {
  const el = this;
  const lower = (s) => String(s || '').toLowerCase();
  const tag = lower(el.tagName);
  const attr = (n) => (el.getAttribute ? el.getAttribute(n) || '' : '');
  const type = lower(attr('type'));
  const textTypes = ['', 'text', 'search', 'email', 'url', 'tel', 'number', 'password'];
  const isField = tag === 'textarea' || el.isContentEditable === true || (tag === 'input' && textTypes.includes(type));
  const opaque = ['iframe', 'frame', 'object', 'embed'].includes(tag) || !!el.shadowRoot;
  const anchor = el.closest ? el.closest('a[href]') : null;
  const form = (el.closest ? el.closest('form') : null) || el.form || null;
  let plainLink = false;
  if (anchor && !form && !opaque) {
    const href = anchor.getAttribute('href') || '';
    const m = /^([a-z][a-z0-9+.-]*):/i.exec(href);
    const scheme = m ? lower(m[1]) : '';
    let handler = false;
    for (let x = el; x; x = x.parentElement) {
      if (['onclick', 'onmousedown', 'onmouseup', 'onpointerdown', 'onpointerup', 'ontouchstart'].some((n) => x.hasAttribute(n))) handler = true;
      if (x === anchor) break;
    }
    plainLink = (scheme === '' || scheme === 'http' || scheme === 'https') && href !== '' && href[0] !== '#' && !handler && !anchor.hasAttribute('download');
  }
  const typing = typed !== undefined && typed !== null;
  const acts = typing ? true : !(plainLink || isField);
  const ctl = el.closest ? el.closest('button, input[type=submit], input[type=image], [role=button]') : null;
  const raw = ctl || anchor || el;
  const named = typing ? (el.name || el.id || tag) : (raw.textContent || raw.value || (raw.getAttribute && (raw.getAttribute('aria-label') || raw.getAttribute('title'))) || tag);
  const label = String(named).replace(/\s+/g, ' ').trim().slice(0, 80);
  let destination = '';
  let method = '';
  if (form) {
    const submitter = ctl && ctl.form === form ? ctl : null;
    destination = String(submitter && submitter.formAction ? submitter.formAction : form.action);
    method = lower(submitter && submitter.formMethod ? submitter.formMethod : form.method) || 'get';
  }
  const fields = [];
  if (form) {
    for (const f of Array.from(form.elements)) {
      const name = f.name || f.id;
      if (!name || f.disabled) continue;
      const t = lower(f.type);
      if (['submit', 'button', 'image', 'reset', 'file'].includes(t)) continue;
      if ((t === 'checkbox' || t === 'radio') && !f.checked) continue;
      let v = typing && f === el ? String(typed) : String(f.value || '');
      if (t === 'password') v = '(password, ' + v.length + ' characters)';
      fields.push(name + '=' + v.replace(/\s+/g, ' '));
    }
  }
  const kind = opaque ? 'opaque' : plainLink ? 'link' : isField ? 'field' : ctl ? 'button' : 'other';
  return JSON.stringify({ acts, kind, label, url: String(location.href), destination, method, fields, typing });
}"#;

/// What the page's description of a control says, as the card needs it.
#[derive(Debug, PartialEq)]
pub(crate) struct Described {
    pub acts: bool,
    pub kind: String,
    pub label: String,
    pub url: String,
    pub destination: String,
    pub method: String,
    pub fields: Vec<String>,
    pub typing: bool,
}

impl Described {
    pub(crate) fn parse(json: &str) -> Option<Described> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let text = |k: &str| v[k].as_str().unwrap_or("").to_string();
        Some(Described {
            acts: v["acts"].as_bool()?,
            kind: text("kind"),
            label: text("label"),
            url: text("url"),
            destination: text("destination"),
            method: text("method"),
            fields: v["fields"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            typing: v["typing"].as_bool().unwrap_or(false),
        })
    }

    /// Where it goes, and what is done, for the card. Every page-supplied piece is quoted on its
    /// own, so none can pose as another (a field whose value reads `, card=(password, 0 characters)`
    /// is still one quoted string).
    pub(crate) fn card(&self, text_typed: Option<&str>) -> (String, String) {
        let quote = |s: &str| serde_json::Value::String(s.to_string()).to_string();
        let to = if self.destination.is_empty() {
            if self.kind == "opaque" {
                format!("{} (inside a frame or component the page controls)", self.url)
            } else {
                format!("{} (the page handles it itself)", self.url)
            }
        } else {
            format!("{} -> {} {}", self.url, self.method.to_uppercase(), self.destination)
        };
        let sent = if self.fields.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = self.fields.iter().map(|f| quote(f)).collect();
            format!(" sending [{}]", list.join(", "))
        };
        let what = match text_typed {
            Some(text) => format!("type {} into {} and submit{sent}", quote(text), quote(&self.label)),
            None if self.kind == "opaque" => "press something its page controls; the content of a frame or component cannot be inspected".to_string(),
            None => format!("press {}{sent}", quote(&self.label)),
        };
        (to, what)
    }
}

/// The answer to "may this act be done?", carried to the act.
pub(crate) struct Pressing {
    permit: Permit,
    /// The description the person was shown (as JSON), when there was one: the act is made only if
    /// the element still describes exactly like this.
    shown: Option<String>,
    /// Whether the person was asked at all.
    asked: bool,
}

/// What came of the press.
pub(crate) enum Outcome {
    /// It was made; this is what to tell the model.
    Done(String),
    /// The page changed while the person decided; nothing was pressed; this says so.
    Changed(String),
    /// This page cannot be pressed this way (no isolated world): make the press the old way.
    Unavailable,
}

const CHANGED: &str = "The page changed while the person was deciding (the control or the form it sends is not what the card showed), so nothing was pressed. Look at the page again and retry; the person will be asked about what is there now.";

impl Pressing {
    pub(crate) fn finish(self, ok: bool) {
        self.permit.finish(ok);
    }

    #[cfg(test)]
    pub(crate) fn was_asked(&self) -> bool {
        self.asked
    }

    /// Press the element, but only if it is what the card showed; one evaluation describes it
    /// again, compares, and clicks it.
    pub(crate) fn press(&self, page: &mut dyn Page, pick: &Pick) -> Outcome {
        let expected = self.shown.as_deref().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
        match page.on_element(pick, &press_fn(), &[expected]) {
            Ok(reply) => match serde_json::from_str::<serde_json::Value>(&reply) {
                Ok(v) if v["pressed"] == true => Outcome::Done(format!("Clicked: {}", v["label"].as_str().unwrap_or(""))),
                Ok(_) => Outcome::Changed(CHANGED.to_string()),
                Err(_) => Outcome::Unavailable,
            },
            Err(_) => Outcome::Unavailable,
        }
    }

    /// Type `text` into the element and submit it, but only if the form it sends is what the card
    /// showed: one evaluation sets the value, lets the page's own handlers run, describes the form
    /// again, compares, and submits.
    pub(crate) fn type_and_submit(&self, page: &mut dyn Page, pick: &Pick, text: &str, clear: bool) -> Outcome {
        let expected = self.shown.as_deref().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
        match page.on_element(pick, &type_fn(), &[text.into(), expected, clear.into()]) {
            Ok(reply) => match serde_json::from_str::<serde_json::Value>(&reply) {
                Ok(v) if v["pressed"] == true => Outcome::Done(format!("Typed into: {}", v["label"].as_str().unwrap_or(""))),
                Ok(_) => Outcome::Changed(CHANGED.to_string()),
                Err(_) => Outcome::Unavailable,
            },
            Err(_) => Outcome::Unavailable,
        }
    }

    /// For presses that are made by the mouse at a point (a frame's inside cannot be reached any
    /// other way): check, right before the mouse events, that the point still describes as shown.
    pub(crate) fn confirm(&self, page: &mut dyn Page, pick: &Pick, typed: Option<&str>) -> Result<(), String> {
        let Some(shown) = &self.shown else { return Ok(()) };
        let args = [typed.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null)];
        match page.on_element(pick, DESCRIBE_FN, &args) {
            Ok(now) if &now == shown => Ok(()),
            Ok(_) => Err(CHANGED.to_string()),
            Err(_) => Ok(()), // cannot be described now either way: the card named it as it was
        }
    }
}

/// Click `this` if it still describes as `expected` (null: just click).
fn press_fn() -> String {
    format!(
        "function (expected) {{ const describe = {DESCRIBE_FN}; const now = describe.call(this, null); \
         if (expected !== null && now !== expected) return JSON.stringify({{ pressed: false }}); \
         const label = JSON.parse(now).label; this.click(); return JSON.stringify({{ pressed: true, label }}); }}"
    )
}

/// Type `text` into `this`, then submit only if the form it sends still describes as `expected`.
pub(crate) fn type_fn() -> String {
    format!(
        "function (text, expected, clear) {{ const describe = {DESCRIBE_FN}; const el = this; el.focus(); \
         const proto = el.tagName.toLowerCase() === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype; \
         const set = Object.getOwnPropertyDescriptor(proto, 'value'); const value = (clear ? '' : String(el.value || '')) + text; \
         if (set && set.set) set.set.call(el, value); else el.value = value; \
         el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }})); \
         const now = describe.call(el, value); if (expected !== null && now !== expected) return JSON.stringify({{ pressed: false }}); \
         if (el.form) {{ el.form.submit(); }} else {{ el.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', keyCode: 13, bubbles: true }})); }} \
         return JSON.stringify({{ pressed: true, label: JSON.parse(now).label }}); }}"
    )
}

fn unknown() -> (String, String) {
    ("a page that did not describe the control".to_string(), "press a control the page could not describe".to_string())
}

/// Ask before pressing `pick`, if pressing it can act. A click asks unless the element is a plain
/// navigation link or a text field; a page that cannot say what it is is asked about as such.
pub(crate) fn ask_before_press(page: &mut dyn Page, pick: &Pick, tool: &str) -> Result<Pressing, String> {
    ask(page, pick, tool, None)
}

/// Ask before typing `text` into `pick` and, if `submit`, submitting. Typing alone only asks if the
/// element is not a text field (a click at a point may land on a button); submitting always asks.
///
/// `exact` is whether the text replaces what the field holds (the usual case): then the form that is
/// described is exactly the form that will be sent, and the submit is checked against it. Text that
/// is appended to what is there cannot be described ahead, so the card is shown and the submit is
/// made the old way.
pub(crate) fn ask_before_typing(page: &mut dyn Page, pick: &Pick, tool: &str, text: &str, submit: bool, exact: bool) -> Result<Pressing, String> {
    let mut pressing = ask(page, pick, tool, submit.then_some(text))?;
    if !exact {
        pressing.shown = None;
    }
    Ok(pressing)
}

fn ask(page: &mut dyn Page, pick: &Pick, tool: &str, typed: Option<&str>) -> Result<Pressing, String> {
    let arg = typed.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
    let (to, what, shown) = match page.on_element(pick, DESCRIBE_FN, &[arg]) {
        Err(e) if e == NOT_FOUND => return Ok(Pressing { permit: Permit::asked(), shown: None, asked: false }), // nothing there; the act will say so
        Ok(json) => match Described::parse(&json) {
            Some(d) if !d.acts => return Ok(Pressing { permit: Permit::asked(), shown: None, asked: false }),
            Some(d) => {
                let (to, what) = d.card(typed);
                (to, what, Some(json))
            }
            None => {
                let (to, what) = unknown();
                (to, what, None)
            }
        },
        Err(_) => {
            let (to, what) = unknown();
            (to, what, None)
        }
    };
    let permit = ask_person_before_acting(&OutwardAct { tool, to: &to, what: &what, more: "" })?;
    Ok(Pressing { permit, shown, asked: true })
}

#[cfg(test)]
mod tests {
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
    }

    impl Page for Fake {
        fn on_element(&mut self, _pick: &Pick, function_src: &str, _args: &[serde_json::Value]) -> Result<String, String> {
            if function_src.contains("this.click()") || function_src.contains("el.form.submit()") {
                self.pressed = true;
                return Ok(self.press.clone());
            }
            if self.describes.len() > 1 { self.describes.remove(0) } else { self.describes[0].clone() }
        }
    }

    fn fake(describes: Vec<Result<String, String>>) -> Fake {
        Fake { describes, press: r#"{"pressed":true,"label":"Place order"}"#.to_string(), pressed: false }
    }

    #[test]
    fn the_description_script_asks_by_default_and_exempts_only_links_and_text_fields() {
        let js = DESCRIBE_FN;
        // The default: acts unless it is a plain link or a text field.
        assert!(js.contains("const acts = typing ? true : !(plainLink || isField);"));
        // Climbs from the inner node to its control, anchor and form.
        for needle in ["el.closest('a[href]')", "el.closest('button, input[type=submit], input[type=image], [role=button]')", "el.closest('form')"] {
            assert!(js.contains(needle), "{needle}");
        }
        // What the page cannot be seen into asks, and handlers make a link not plain.
        for needle in ["'iframe'", "el.shadowRoot", "'onclick'", "anchor.hasAttribute('download')"] {
            assert!(js.contains(needle), "{needle}");
        }
        // The destination is the resolved one, the submitter's own if it has one.
        for needle in ["submitter.formAction", "form.action", "submitter.formMethod", "(password, "] {
            assert!(js.contains(needle), "{needle}");
        }
        assert!(press_fn().contains("now !== expected") && press_fn().contains("this.click()"));
        assert!(type_fn().contains("now !== expected"));
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
            assert!(pressing.confirm(&mut page, &Pick::Point(3.0, 4.0), None).is_ok());
            assert!(pressing.confirm(&mut page, &Pick::Point(3.0, 4.0), None).is_err(), "the page moved the control");
        }
    }
}
