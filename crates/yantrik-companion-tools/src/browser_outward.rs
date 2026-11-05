//! Which browser acts send something out, and what the person is shown before they do (#1054,
//! #1063).
//!
//! A click can be a link or the "Place order" button, and a keystroke can be a letter or the Enter
//! that submits a form. The rule here is the safe one: **a click asks unless it is a plain
//! navigation link, or a text field being focused.** Nothing enumerates what counts as "acting":
//! a button, a `div` with a handler, the `span` inside a button, an `iframe` or a shadow-DOM
//! component (whose inside cannot be seen) all ask. Typing that submits always asks. A link is not
//! plain with a handler attribute, a framework's method attribute (`data-method`,
//! `data-turbo-method`, `data-remote`, `hx-*`) or `ping`; a field is an `input`, a `textarea`, or an
//! editing host itself, never a control inside one. One limit is inherent: a handler added with
//! `addEventListener` leaves no trace in the DOM, so a link with one is still described as plain.
//!
//! Three things keep the card honest:
//!
//! - **The page does not write it.** The control is described by a function run in an *isolated
//!   world* (`Page.createIsolatedWorld`): the page's own scripts cannot reach its globals or its
//!   prototypes, so a hostile page cannot override `closest` or `JSON.stringify` to report its own
//!   submit button as harmless.
//! - **What is pressed is what was shown**, as of the moment before the click (the page's own click
//!   listeners run first and can still change the form). The description is the card's content and
//!   its fingerprint, card or no card. After the Allow, the press is a single evaluation that
//!   describes the same element again, compares, and only then clicks (or types and submits) it; a
//!   page that changed while the person decided is not pressed, and the answer says so, so the
//!   model looks again and asks again for what is there now. At a point, a described element is
//!   pressed this way too, not by the mouse, which the page can redirect (`Pressing::at_point`).
//! - **The card names where it goes and what goes.** A form's destination is its resolved `action`
//!   (or the pressed button's `formaction`, only when it has one; for typing, the default button's,
//!   which Enter uses), its method, and every field it will send, hidden ones included, each quoted;
//!   a password shows only its length. The form's own properties are read through
//!   `HTMLFormElement.prototype`, so a field named `action`, `method` or `elements` cannot stand in.
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
  const E = Element.prototype;
  const getter = (C, n) => Object.getOwnPropertyDescriptor(C.prototype, n).get;
  const [formAction, formMethod, formElements] = ['action', 'method', 'elements'].map((n) => getter(HTMLFormElement, n));
  const closest = (x, sel) => (x instanceof Element ? E.closest.call(x, sel) : null);
  const has = (x, n) => x instanceof Element && E.hasAttribute.call(x, n);
  const attr = (n) => (el instanceof Element ? E.getAttribute.call(el, n) || '' : '');
  const tag = lower(el instanceof Element ? getter(Element, 'tagName').call(el) : '');
  const type = lower(attr('type'));
  const textTypes = ['', 'text', 'search', 'email', 'url', 'tel', 'number', 'password'];
  const acting = 'button, a, [role=button], input, select, textarea, label, summary, [onclick], [onmousedown], [onmouseup], [onpointerdown], [onpointerup], [ontouchstart]';
  const editingHost = el.isContentEditable === true && !closest(el, acting);
  const isField = tag === 'textarea' || editingHost || (tag === 'input' && textTypes.includes(type));
  const opaque = ['iframe', 'frame', 'object', 'embed'].includes(tag) || !!el.shadowRoot;
  const anchor = closest(el, 'a[href]');
  const ctl = closest(el, 'button, input[type=submit], input[type=image], [role=button]');
  const ownerOf = (x) => (x && x.form instanceof HTMLFormElement ? x.form : null);
  const form = ownerOf(ctl) || ownerOf(el) || (el instanceof HTMLFormElement ? el : null) || closest(el, 'form');
  const sends = (x) => Array.from(x.attributes || []).some((a) => /^(data-(turbo-)?method|data-remote|data-turbo-stream|data-confirm|(data-)?hx-|ping$)/.test(lower(a.name)));
  let plainLink = false;
  if (anchor && !form && !opaque) {
    const href = E.getAttribute.call(anchor, 'href') || '';
    let scheme = '';
    try { scheme = new URL(String(anchor.href)).protocol; } catch (e) { scheme = ''; }
    let handler = false;
    for (let x = el; x; x = x.parentElement) {
      if (['onclick', 'onmousedown', 'onmouseup', 'onpointerdown', 'onpointerup', 'ontouchstart'].some((n) => has(x, n)) || sends(x)) handler = true;
      if (x === anchor) break;
    }
    plainLink = (scheme === 'http:' || scheme === 'https:') && href.trim() !== '' && href.trim()[0] !== '#' && !handler && !has(anchor, 'download');
  }
  const typing = typed !== undefined && typed !== null;
  const acts = typing ? true : !(plainLink || isField);
  const raw = ctl || anchor || el;
  const named = typing ? (el.name || el.id || tag) : (getter(Node, 'textContent').call(raw) || raw.value || (raw instanceof Element && (E.getAttribute.call(raw, 'aria-label') || E.getAttribute.call(raw, 'title'))) || tag);
  const label = String(named).replace(/\s+/g, ' ').trim().slice(0, 80);
  const controls = form ? Array.from(formElements.call(form)) : [];
  const isSubmit = (x) => (x instanceof HTMLButtonElement && lower(x.type) === 'submit') || (x instanceof HTMLInputElement && ['submit', 'image'].includes(lower(x.type)));
  let destination = '';
  let method = '';
  if (form) {
    const submitter = typing ? controls.find(isSubmit) || null : (ctl && ownerOf(ctl) === form ? ctl : null);
    destination = String(has(submitter, 'formaction') ? submitter.formAction : formAction.call(form));
    method = lower(has(submitter, 'formmethod') ? submitter.formMethod : formMethod.call(form)) || 'get';
  }
  const fields = [];
  for (const f of controls) {
    const name = f.name || f.id;
    if (!name || f.disabled) continue;
    const t = lower(f.type);
    if (['submit', 'button', 'image', 'reset', 'file', 'fieldset', 'output'].includes(t) || f instanceof HTMLObjectElement) continue;
    if ((t === 'checkbox' || t === 'radio') && !f.checked) continue;
    let v = typing && f === el ? String(typed) : String(f.value || '');
    if (t === 'password') v = '(password, ' + v.length + ' characters)';
    fields.push(name + '=' + v.replace(/\s+/g, ' '));
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
    /// The description the verdict was made on (as JSON), card or no card: the act is made only if
    /// the element still describes exactly like this. `None` when the page could not be described
    /// (the card said so) or nothing was there.
    shown: Option<String>,
    /// What the description was made with (text about to be typed and submitted, or null).
    typed: serde_json::Value,
    /// Whether the submit can be checked against `shown` (the text replaces the field's).
    exact: bool,
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

/// How an act at a point is made.
pub(crate) enum AtPoint {
    /// Made in the isolated world, on the element that was described: this is what to tell the model.
    Done(String),
    /// Not made; this says why.
    Refused(String),
    /// To be made by the mouse: the point is a frame or component the page controls (whose inside
    /// only the mouse reaches), or a page that could not be described; checked again just before.
    Mouse,
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

    fn expected(&self, for_submit: bool) -> serde_json::Value {
        match &self.shown {
            Some(json) if !for_submit || self.exact => serde_json::Value::from(json.as_str()),
            _ => serde_json::Value::Null,
        }
    }

    /// Press the element, but only if it is what the verdict was made on; one evaluation describes
    /// it again, compares, and clicks it.
    pub(crate) fn press(&self, page: &mut dyn Page, pick: &Pick) -> Outcome {
        self.press_with(page, pick, false)
    }

    fn press_with(&self, page: &mut dyn Page, pick: &Pick, double: bool) -> Outcome {
        match page.on_element(pick, &press_fn(), &[self.expected(false), double.into()]) {
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
    /// again, compares, and submits through the form's default button, as Enter would.
    pub(crate) fn type_and_submit(&self, page: &mut dyn Page, pick: &Pick, text: &str, clear: bool) -> Outcome {
        match page.on_element(pick, &type_fn(), &[text.into(), self.expected(true), clear.into()]) {
            Ok(reply) => match serde_json::from_str::<serde_json::Value>(&reply) {
                Ok(v) if v["pressed"] == true => Outcome::Done(format!("Typed into: {}", v["label"].as_str().unwrap_or(""))),
                Ok(_) => Outcome::Changed(CHANGED.to_string()),
                Err(_) => Outcome::Unavailable,
            },
            Err(_) => Outcome::Unavailable,
        }
    }

    /// Click (`focus_only`: focus) the element at a point. A mouse press is described once and then
    /// lands wherever the page has moved things by the time it arrives (a link that turns
    /// `pointer-events: none` on `mousemove` passes it to a cross-origin frame underneath), so every
    /// described target is pressed in the isolated world, on the element that was described, and
    /// only a frame or component the page controls, or a page that could not be described, is left
    /// to the mouse (#1063 review, M3).
    pub(crate) fn at_point(&self, page: &mut dyn Page, pick: &Pick, focus_only: bool, double: bool) -> AtPoint {
        let opaque = self.shown.as_deref().and_then(Described::parse).is_some_and(|d| d.kind == "opaque");
        if opaque || (self.shown.is_none() && self.asked) {
            return match self.confirm(page, pick) {
                Ok(()) => AtPoint::Mouse,
                Err(said) => AtPoint::Refused(said),
            };
        }
        if self.shown.is_none() {
            return AtPoint::Refused("Nothing is at that point, so nothing was pressed.".to_string());
        }
        let outcome = if focus_only {
            match page.on_element(pick, &focus_fn(), &[self.typed.clone(), self.expected(false)]) {
                Ok(r) if r == "focused" => Outcome::Done(String::new()),
                Ok(_) => Outcome::Changed(CHANGED.to_string()),
                Err(_) => Outcome::Unavailable,
            }
        } else {
            self.press_with(page, pick, double)
        };
        match outcome {
            Outcome::Done(said) => AtPoint::Done(said),
            Outcome::Changed(said) => AtPoint::Refused(said),
            Outcome::Unavailable => AtPoint::Refused("The control at that point could no longer be reached, so nothing was pressed. Look at the page again.".to_string()),
        }
    }

    /// For presses that are made by the mouse or the keyboard: check, right before, that the target
    /// still describes as it did. A target that cannot be described now is not pressed.
    pub(crate) fn confirm(&self, page: &mut dyn Page, pick: &Pick) -> Result<(), String> {
        let Some(shown) = &self.shown else { return Ok(()) }; // the card said the page could not be described
        match page.on_element(pick, DESCRIBE_FN, std::slice::from_ref(&self.typed)) {
            Ok(now) if &now == shown => Ok(()),
            _ => Err(CHANGED.to_string()),
        }
    }

    /// [`Self::confirm`] for the Enter that submits typed text: the focused field, described with
    /// the text, against the card. Text appended to what was there could not be described ahead,
    /// and was asked about as such.
    pub(crate) fn confirm_submit(&self, page: &mut dyn Page) -> Result<(), String> {
        if !self.exact {
            return Ok(());
        }
        self.confirm(page, &Pick::Active)
    }
}

/// Click `this` (twice and a `dblclick` if `double`) if it still describes as `expected` (null:
/// just click).
fn press_fn() -> String {
    format!(
        "function (expected, double) {{ const describe = {DESCRIBE_FN}; const now = describe.call(this, null); \
         if (expected !== null && now !== expected) return JSON.stringify({{ pressed: false }}); \
         const label = JSON.parse(now).label; this.click(); \
         if (double) {{ this.click(); this.dispatchEvent(new MouseEvent('dblclick', {{ bubbles: true, detail: 2 }})); }} \
         return JSON.stringify({{ pressed: true, label }}); }}"
    )
}

/// Focus `this` if it still describes as `expected`, for typing at a point.
fn focus_fn() -> String {
    format!(
        "function (typed, expected) {{ const describe = {DESCRIBE_FN}; const now = describe.call(this, typed); \
         if (expected !== null && now !== expected) return 'changed'; this.focus(); return 'focused'; }}"
    )
}

/// Type `text` into `this`, then submit only if the form it sends still describes as `expected`:
/// through the form's default button, as Enter does, so the button's own `formaction` is what the
/// description named, by the prototype's methods, which a field named `submit` cannot replace.
pub(crate) fn type_fn() -> String {
    format!(
        "function (text, expected, clear) {{ const describe = {DESCRIBE_FN}; const el = this; el.focus(); \
         const proto = el.tagName.toLowerCase() === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype; \
         const set = Object.getOwnPropertyDescriptor(proto, 'value'); const value = (clear ? '' : String(el.value || '')) + text; \
         if (set && set.set) set.set.call(el, value); else el.value = value; \
         el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }})); \
         const now = describe.call(el, value); if (expected !== null && now !== expected) return JSON.stringify({{ pressed: false }}); \
         const form = el.form instanceof HTMLFormElement ? el.form : null; \
         if (form) {{ const F = HTMLFormElement.prototype; \
           const def = Array.from(Object.getOwnPropertyDescriptor(F, 'elements').get.call(form)).find((x) => (x instanceof HTMLButtonElement && x.type === 'submit') || (x instanceof HTMLInputElement && ['submit', 'image'].includes(x.type))); \
           if (def) F.requestSubmit.call(form, def); else F.requestSubmit.call(form); }} \
         else {{ el.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', keyCode: 13, bubbles: true }})); }} \
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
    pressing.exact = exact;
    Ok(pressing)
}

fn ask(page: &mut dyn Page, pick: &Pick, tool: &str, typed: Option<&str>) -> Result<Pressing, String> {
    let arg = typed.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
    let unasked = |shown: Option<String>| Ok(Pressing { permit: Permit::asked(), shown, typed: arg.clone(), exact: true, asked: false });
    let (to, what, shown) = match page.on_element(pick, DESCRIBE_FN, std::slice::from_ref(&arg)) {
        Err(e) if e == NOT_FOUND => return unasked(None), // nothing there; the act will say so
        Ok(json) => match Described::parse(&json) {
            // No card, but the press is still checked against this description (review L1).
            Some(d) if !d.acts => return unasked(Some(json)),
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
    Ok(Pressing { permit, shown, typed: arg, exact: true, asked: true })
}

#[cfg(test)]
#[path = "browser_outward_tests.rs"]
mod tests;
