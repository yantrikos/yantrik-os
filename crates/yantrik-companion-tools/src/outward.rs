//! Asking the person before anything leaves the machine, or acts outward for them (#1026, #1054).
//!
//! A send, a post, a form submit, a command on another machine, a call to a third party's tool:
//! each is outward and cannot be taken back. The companion's tools used to run under their own
//! ceiling with nothing in front of the person: three replies went out on VM 520 in `auto` and in
//! `ask` (#1026), and the messaging, browser and remote tools had the same gap (#1054). There is
//! one approvals module, the shell's, and this is the companion's one way into it, the same
//! `request_approval` / `approval_status` the other asking paths use ([`crate::app_ui`]): raise the
//! card for one of the shell's two outward questions, wait for the person, spend the grant with
//! `consume_approval`, and only then let the act go. Nothing here decides: a mode that runs the
//! question unasked is the shell's answer (full bypass, and the act is written down after), plan
//! refuses, and an unreachable shell is a "no".
//!
//! - `send_mail` ([`ask_before_sending`]): every recipient, the whole subject and the whole body.
//! - `outward_act` ([`ask_before_acting`]): the tool that is acting, who or what it is aimed at,
//!   and what it will send or do, whole.
//!
//! Every free-text value is quoted so text inside one cannot pose as another field (`subject:
//! "Invoice; to: x"` is one value). What the person cannot read they cannot allow, so a request the
//! card cannot show in full (the shell's rule for actions that cannot be undone: a value is shown
//! whole or not asked about) is not asked about, and nothing is done. The grant is bound to exactly
//! the values shown.
//!
//! `tools::outward_pin` in the companion keeps this the only way: a tool that acts outward and is
//! not in its registry, or is in it without calling this module, fails a test.

use crate::app_ui::{await_answer, raise_card, Desk};

const APP: &str = "shell";
const GRADE: &str = "sensitive";

/// One of the shell's outward questions and the words around it.
struct Kind {
    /// The shell action the card is about.
    action: &'static str,
    /// What the card says is being asked, besides the shell's own published sentence.
    purpose: &'static str,
    /// "this email", "this action": how the sentences name it.
    noun: &'static str,
    /// The sentence for one the card cannot show in full.
    too_long: &'static str,
    /// What was not done, for "Nothing was ...".
    unsent: &'static str,
}

const MAIL: Kind = Kind {
    action: "send_mail",
    purpose: "Send an email from the person's account. Once sent it cannot be undone.",
    noun: "this email",
    too_long: concat!(
        "This email is too long to put in front of the person in full, and what they cannot read they cannot allow, ",
        "so it was not asked about. Nothing was sent. Shorten it, or leave it as a draft in the Email app ",
        "(its compose action) for the person to read and send themselves."
    ),
    unsent: "sent",
};

const ACT: Kind = Kind {
    action: "outward_act",
    purpose: "Act outside this machine on the person's behalf. Once done it cannot be undone.",
    noun: "this action",
    too_long: concat!(
        "This action is too long to put in front of the person in full, and what they cannot read they cannot allow, ",
        "so it was not asked about. Nothing was done. Make it shorter, or tell the person what you wanted done so ",
        "they can do it themselves."
    ),
    unsent: "done",
};

/// One message about to leave.
pub struct OutgoingMail<'a> {
    /// The address it is sent from.
    pub account: &'a str,
    pub to: &'a [String],
    pub cc: &'a [String],
    pub bcc: &'a [String],
    pub subject: &'a str,
    pub body: &'a str,
}

/// One act aimed outside the machine.
pub struct OutwardAct<'a> {
    /// The companion tool that is acting, by name: `telegram_send`, `browser_click`.
    pub tool: &'a str,
    /// Who or what it is aimed at: a chat, a number, a page and its form, a host, a server.
    pub to: &'a str,
    /// What it will send or do: the message, the command, the field values, the call.
    pub what: &'a str,
    /// Anything else the person should see before saying yes; empty if nothing.
    pub more: &'a str,
}

/// One free-text value as the card draws it: a JSON string, so it is visibly one value whatever
/// it holds. A `"` inside is escaped and cannot end it early; `;` and `key:` inside stay inside.
fn quoted(text: &str) -> serde_json::Value {
    serde_json::Value::String(serde_json::Value::String(text.to_string()).to_string())
}

/// The arguments the mail card shows and the grant is bound to: all of the mail, nothing clipped.
pub fn card_args(mail: &OutgoingMail<'_>) -> serde_json::Value {
    let mut args = serde_json::json!({
        "account": mail.account,
        "to": quoted(&mail.to.join(", ")),
        "subject": quoted(mail.subject),
        "body": quoted(mail.body),
    });
    for (key, list) in [("cc", mail.cc), ("bcc", mail.bcc)] {
        if !list.is_empty() {
            args[key] = quoted(&list.join(", "));
        }
    }
    args
}

/// The arguments the act card shows and the grant is bound to.
pub fn act_args(act: &OutwardAct<'_>) -> serde_json::Value {
    let mut args = serde_json::json!({
        // A name a server chose (an MCP tool's) is not free text: letters, digits and underscores.
        "tool": act.tool.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect::<String>(),
        "to": quoted(act.to),
        "what": quoted(act.what),
    });
    if !act.more.is_empty() {
        args["more"] = quoted(act.more);
    }
    args
}

/// Whether the shell refused to put a request up because the card could not show it whole.
fn is_too_long(why: &str) -> bool {
    why.contains("too long to put in front of a person") || why.contains("shown whole or not asked about")
}

/// What an allowed act owes the record afterwards, if the mode let it run unasked.
pub struct Permit {
    unasked: Option<(Desk, &'static str, serde_json::Value, String)>,
}

impl std::fmt::Debug for Permit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Permit(owes_a_record: {})", self.unasked.is_some())
    }
}

impl Permit {
    /// Nothing to record: the person was asked, or this is a test.
    pub fn asked() -> Permit {
        Permit { unasked: None }
    }

    /// Say how the act went. In the person's full bypass nobody was asked, and the shell's mode
    /// promises it is "written down instead": this is that, by the shell's own
    /// `record_unasked_action`. A failure to record cannot undo the act and is not an error.
    pub fn finish(self, done: bool) {
        let Some((desk, action, args, mode)) = self.unasked else { return };
        let _ = desk.shell_call(
            "app.act",
            serde_json::json!({ "action": "record_unasked_action", "args": {
                "app": APP, "action": action, "grade": GRADE, "mode": mode, "args_json": args,
                "requester": "companion", "outcome": if done { "ok" } else { "failed" },
            }}),
        );
    }
}

/// The one asking: raise the card for `kind` with `args`, wait, spend. `Ok` only when the person
/// allowed exactly this (or the shell says its mode asks nobody); `Err` is a sentence for the
/// model and nothing was done.
fn ask(desk: &Desk, kind: &Kind, args: serde_json::Value) -> Result<Permit, String> {
    let (noun, unsent) = (kind.noun, kind.unsent);
    let asked = raise_card(desk, APP, kind.action, GRADE, &args, kind.purpose)
        .map_err(|why| if is_too_long(&why) { kind.too_long.to_string() } else { format!("{why} Nothing was {unsent}.") })?;
    let answer = asked.get("result").cloned().unwrap_or_default();
    if answer["status"] == "not_needed" {
        let mode = answer["mode"].as_str().unwrap_or("bypass_all").to_string();
        return Ok(Permit { unasked: Some((desk.clone(), kind.action, args, mode)) });
    }
    let Some(request_id) = answer["request_id"].as_str().map(str::to_string) else {
        return Err(format!(
            "The desktop was asked to put a card up for {noun} and did not say which request it was, so nothing was {unsent}."
        ));
    };
    match await_answer(desk, &request_id).as_str() {
        "granted" => spend(desk, kind, &request_id, &args).map(|()| Permit::asked()),
        "denied" => Err(format!(
            "The person at the machine said no to {noun}. Nothing was {unsent}. That is an answer, not an error: do not \
             ask again unless they bring it up."
        )),
        "consumed" => Err(format!(
            "The Allow for {noun} had already been spent, so nothing was {unsent}. Ask again if it still needs doing."
        )),
        _ => Err(format!(
            "Nobody answered the card for {noun} within {} s, so nothing was {unsent}. They were probably away from the \
             keyboard; say what you wanted and try again when they are back.",
            desk.wait.as_secs()
        )),
    }
}

/// Ask the person, on this desktop's own card, whether `mail` may go.
pub fn ask_before_sending(desk: &Desk, mail: &OutgoingMail<'_>) -> Result<Permit, String> {
    ask(desk, &MAIL, card_args(mail))
}

/// Ask the person, on this desktop's own card, whether `act` may be done.
pub fn ask_before_acting(desk: &Desk, act: &OutwardAct<'_>) -> Result<Permit, String> {
    ask(desk, &ACT, act_args(act))
}

/// The desk to ask on: the session's own, or the fake shell a test installed.
fn session_desk() -> Desk {
    #[cfg(all(unix, any(test, feature = "test-support")))]
    if let Some(dir) = crate::outward_testing::installed_dir() {
        return Desk { dir: Some(dir), wait: std::time::Duration::from_millis(600), poll: std::time::Duration::from_millis(20) };
    }
    Desk::session()
}

/// [`ask_before_sending`] on the session's own desk.
pub fn ask_person_before_sending(mail: &OutgoingMail<'_>) -> Result<Permit, String> {
    ask_before_sending(&session_desk(), mail)
}

/// [`ask_before_acting`] on the session's own desk. What every outward tool calls first, with what
/// it is about to do, and `finish`es afterwards.
pub fn ask_person_before_acting(act: &OutwardAct<'_>) -> Result<Permit, String> {
    ask_before_acting(&session_desk(), act)
}

/// The shape most tools need: ask, and only if the person allowed it run `act` (what the tool was
/// about to do), then write the act down if the mode let it run unasked. A refusal is the tool's
/// answer, in words for the model, and `act` never ran.
pub fn acting(what: &OutwardAct<'_>, act: impl FnOnce() -> String) -> String {
    match ask_person_before_acting(what) {
        Err(refusal) => refusal,
        Ok(permit) => {
            let answer = act();
            permit.finish(!looks_failed(&answer));
            answer
        }
    }
}

/// [`acting`], except that it runs `act` without asking when `skip` is true: for a tool whose
/// kind of call is provably contained (code inside the strict sandbox, a read-only command).
pub fn acting_unless(skip: bool, what: &OutwardAct<'_>, act: impl FnOnce() -> String) -> String {
    if skip {
        act()
    } else {
        acting(what, act)
    }
}

/// Whether a tool's answer reports that it did what it was asked.
pub fn outcome_ok(answer: &str) -> bool {
    !looks_failed(answer)
}

/// Whether a tool's answer reports that it did not do what it was asked: the words the companion's
/// tools begin a failure with.
fn looks_failed(answer: &str) -> bool {
    let head = answer.trim_start().to_lowercase();
    ["error", "failed", "could not", "cannot", "permission denied", "refused"].iter().any(|w| head.starts_with(w))
        || head.contains(" failed:")
}

/// What kind of text a read aims at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// An address the model chose.
    Address,
    /// Search text the model chose.
    Text,
}

/// The characters a wire address leaves as they are besides letters, digits and `_.-~`: the URL's
/// own delimiters, and `%`.
const WIRE_SAFE: &str = "%/:@!$&'()*+,;=-._~?#[]";

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || WIRE_SAFE.as_bytes().contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// An address in the form the browser will send, as ASCII: the host in punycode (the IDNA mapping
/// folds fullwidth forms and drops ignorable characters, so a look-alike shows as the name it is),
/// and every non-ASCII, control or whitespace character percent-encoded, so nothing on a card is
/// invisible. `None` when no such form exists (the host is not a valid name, or there is nothing).
///
/// The same rule, and the same test vectors (`deploy/yantrik-os/wire-address-vectors.json`), as
/// `wire_address` in `yos-mcp`: the card shows this string, the grant is bound to it, and it is
/// what is fetched, byte for byte.
pub fn wire_address(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // urlsplit: scheme, then `//authority`, then path, `?query`, `#fragment`.
    let scheme_end = url.find(':').filter(|&i| {
        i > 0 && url[..i].chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && url[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    let (scheme, rest) = match scheme_end {
        Some(i) => (url[..i].to_ascii_lowercase(), &url[i + 1..]),
        None => (String::new(), url),
    };
    let (netloc, after) = match rest.strip_prefix("//") {
        Some(r) => {
            let end = r.find(['/', '?', '#']).unwrap_or(r.len());
            (Some(&r[..end]), &r[end..])
        }
        None => (None, rest),
    };
    let (before_fragment, fragment) = match after.split_once('#') {
        Some((a, f)) => (a, Some(f)),
        None => (after, None),
    };
    let (path, query) = match before_fragment.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (before_fragment, None),
    };

    let mut wire = String::new();
    if !scheme.is_empty() {
        wire.push_str(&quote(&scheme));
        wire.push(':');
    }
    if let Some(netloc) = netloc {
        let (userinfo, at, hostport) = match netloc.rfind('@') {
            Some(i) => (&netloc[..i], "@", &netloc[i + 1..]),
            None => ("", "", netloc),
        };
        let (host, port) = if hostport.starts_with('[') {
            match hostport.find(']') {
                Some(i) => (&hostport[..=i], &hostport[i + 1..]),
                None => (hostport, ""),
            }
        } else {
            match hostport.find(':') {
                Some(i) => (&hostport[..i], &hostport[i..]),
                None => (hostport, ""),
            }
        };
        let plain = host.is_ascii() && host.chars().all(|c| !c.is_control() && !c.is_whitespace());
        let host = if plain {
            host.to_string()
        } else if host.starts_with('[') {
            return None;
        } else {
            idna::domain_to_ascii_strict(host).ok().filter(|h| !h.is_empty())?
        };
        wire.push_str("//");
        wire.push_str(&quote(userinfo));
        wire.push_str(at);
        wire.push_str(&quote(&host));
        wire.push_str(&quote(port));
    }
    wire.push_str(&quote(path));
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        wire.push('?');
        wire.push_str(&quote(q));
    }
    if let Some(f) = fragment.filter(|f| !f.is_empty()) {
        wire.push('#');
        wire.push_str(&quote(f));
    }
    (!wire.is_empty() && wire.is_ascii() && !wire.chars().any(|c| c.is_control() || c.is_whitespace())).then_some(wire)
}

/// Text (a search query) as a card spells it: whole, with every character a reader cannot see
/// written out as `<U+XXXX>`.
fn show_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            let invisible = c.is_control()
                || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{E0000}'..='\u{E007F}' | '\u{00AD}' | '\u{034F}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}' | '\u{3164}' | '\u{2800}');
            if invisible { format!("<U+{:04X}>", c as u32) } else { c.to_string() }
        })
        .collect()
}

/// Whether an address carries nothing in its query, its fragment or its userinfo: the only thing a
/// model could put in it is the path, and a path already visited this turn is not new.
fn is_bare(url: &str) -> bool {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or("");
    !url.contains('?') && !url.contains('#') && !authority.contains('@')
}

/// A read the model aims at an address (or search text) it chose. It is a GET and changes nothing,
/// but what is in the address, or the query, leaves this machine. Asked about, with the address
/// whole, only once the conversation holds something of the person's (a credential, or what a
/// memory, mail, file, clipboard, calendar or terminal read returned): in a clean conversation an
/// ordinary lookup runs unasked, and so does an address with nothing in it that this turn already
/// visited (#1063; the same rule the MCP bridge applies to `web_go`).
///
/// For an address, the card shows [`wire_address`]'s form, the grant is bound to it, and `act` is
/// given the call's arguments with the address replaced by it, so what is asked about, allowed and
/// fetched is one string. An address with no wire form is not put to the person: the answer says so
/// and nothing is fetched.
pub fn reading(tool: &str, args: &serde_json::Value, key: &str, kind: Target, act: impl FnOnce(&serde_json::Value) -> String) -> String {
    let target = args.get(key).and_then(|v| v.as_str()).unwrap_or_default();
    if target.trim().is_empty() {
        return act(args); // nothing goes anywhere; the tool will say what is missing
    }
    let Some(held) = yantrik_companion_core::taint::holds_private_data() else {
        let out = act(args);
        if let (Target::Address, true) = (kind, outcome_ok(&out)) {
            yantrik_companion_core::taint::note_visited(&wire_address(target).unwrap_or_else(|| target.to_string()));
        }
        return out;
    };
    let (shown_to, shown_what, wired) = match kind {
        Target::Address => {
            let Some(wire) = wire_address(target) else {
                return "This conversation has already read something of the person's, and the address given is not one a card can show as the \
                        browser would send it (its host is not a valid name), so the person cannot be asked about it and nothing was fetched. \
                        Not a fault, and not something to route around: use the ordinary form of the address."
                    .to_string();
            };
            if is_bare(&wire) && yantrik_companion_core::taint::has_visited(&wire) {
                let mut same = args.clone();
                same[key] = serde_json::Value::String(wire);
                return act(&same);
            }
            (wire.clone(), "read this address (a GET)".to_string(), Some(wire))
        }
        Target::Text => ("a web search".to_string(), show_text(target), None),
    };
    let more = format!("this conversation has read something of yours ({held}); what is in this address leaves this machine");
    let mut sent = args.clone();
    if let Some(wire) = &wired {
        sent[key] = serde_json::Value::String(wire.clone());
    }
    let answer = acting(&OutwardAct { tool, to: &shown_to, what: &shown_what, more: &more }, || act(&sent));
    if let (Some(wire), true) = (&wired, outcome_ok(&answer)) {
        yantrik_companion_core::taint::note_visited(wire);
    }
    answer
}

/// Burn the grant: it authorises this one act, once.
fn spend(desk: &Desk, kind: &Kind, request_id: &str, args: &serde_json::Value) -> Result<(), String> {
    let (noun, unsent) = (kind.noun, kind.unsent);
    desk.shell_call(
        "app.act",
        serde_json::json!({ "action": "consume_approval", "args": {
            "request_id": request_id, "app": APP, "action": kind.action, "args_json": args,
        }}),
    )
    .map(|_| ())
    .map_err(|e| format!("The person allowed {noun}, but the Allow could not be spent, so nothing was {unsent}: {}", e.message))
}

#[cfg(all(test, unix))]
mod tests;
