//! Reading a foreign window without looking at it.
//!
//! GTK, Qt, Chromium and Firefox all publish their entire widget tree over D-Bus, because screen
//! readers need it. Nobody has to ask them to: the toolkit's accessibility bridge does it, and it
//! has been there for twenty years.
//!
//! That is semantic eyesight for other people's software, free, with no GPU and no model. It
//! makes the rule I wrote earlier — *semantic for ours, visual for theirs* — too pessimistic.
//! Most of theirs is semantic too. Vision belongs third, after `app.describe` for our own windows
//! and after this for everyone else's.
//!
//! # The protocol, in the small
//!
//! AT-SPI is plain D-Bus on a bus of its own. The session bus knows where:
//! `org.a11y.Bus.GetAddress` at `/org/a11y/bus` hands back an address, and everything else lives
//! there. An object is a `(bus_name, object_path)` pair — the `(so)` signature that appears all
//! over this file — and the tree hangs off the registry's root.
//!
//! Written against `zbus` directly rather than the `atspi` crate: zbus is already a workspace
//! dependency for the system bus, the surface used here is four methods and three properties, and
//! a crate that pins its own zbus version is a conflict waiting to happen for that.
//!
//! # What it deliberately does not return
//!
//! The raw tree is mostly scaffolding. A zenity dialog with two fields nests six unnamed panels
//! before reaching anything a person would mention. Sending that to a model is the same mistake
//! as sending a screenshot: a lot of bytes that have to be read to find out they said nothing.
//!
//! So the walk flattens, and keeps only nodes that carry a name, some text, or an action. What
//! comes back is what someone would say if you asked them what was on screen.

use std::collections::HashMap;

use serde::Serialize;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::{proxy, Connection};

/// How deep to walk. Toolkit trees nest hard; past this the content is layout, not meaning.
const MAX_DEPTH: usize = 12;

/// Ceiling on nodes reported for one window.
const MAX_ELEMENTS: usize = 120;

/// Ceiling on nodes *visited*, including the scaffolding that is thrown away. A pathological tree
/// must not be able to hold the service for minutes.
const MAX_VISITS: usize = 3000;

/// Text longer than this is a document, not a label.
const MAX_TEXT: usize = 400;

/// An AT-SPI object reference: which connection, and where on it.
type Reference = (String, OwnedObjectPath);

#[proxy(interface = "org.a11y.Bus", default_service = "org.a11y.Bus", default_path = "/org/a11y/bus")]
trait A11yBus {
    fn get_address(&self) -> zbus::Result<String>;
}

#[proxy(interface = "org.a11y.atspi.Accessible", assume_defaults = false)]
trait Accessible {
    fn get_children(&self) -> zbus::Result<Vec<Reference>>;
    fn get_role_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn description(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn child_count(&self) -> zbus::Result<i32>;
}

#[proxy(interface = "org.a11y.atspi.Text", assume_defaults = false)]
trait Text {
    fn get_text(&self, start: i32, end: i32) -> zbus::Result<String>;
    #[zbus(property)]
    fn character_count(&self) -> zbus::Result<i32>;
}

#[proxy(interface = "org.a11y.atspi.Action", assume_defaults = false)]
trait Action {
    fn get_actions(&self) -> zbus::Result<Vec<(String, String, String)>>;
    fn do_action(&self, index: i32) -> zbus::Result<bool>;
}

/// One window a person is looking at.
#[derive(Debug, Clone, Serialize)]
pub struct Window {
    /// The application's own name for itself: `zenity`, `firefox`, `soffice`.
    pub app: String,
    pub title: String,
    pub role: String,
    pub pid: u32,
    /// Opaque handle for `a11y.describe` and `a11y.act`. Deliberately not a path a caller could
    /// construct: the only ones that work are ones we handed out.
    pub id: String,
}

/// One thing inside a window that a person would mention.
#[derive(Debug, Clone, Serialize)]
pub struct Element {
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// What this element can be asked to do, if anything. The presence of `press` here is why a
    /// button in a foreign app can be pressed without a synthetic mouse.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    pub id: String,
}

/// A connection to the accessibility bus.
pub struct Atspi {
    conn: Connection,
    /// Handed-out ids, so a caller can name an element without being able to invent one.
    handles: HashMap<String, Reference>,
    next_handle: u64,
}

impl Atspi {
    /// Find the accessibility bus and connect to it.
    ///
    /// Two ways it is normally absent, and they mean different things: no session bus at all (a
    /// headless box, a service started outside the user's session), or a session bus with no
    /// `org.a11y.Bus` on it (accessibility never started). Both are reported as themselves rather
    /// than as one vague failure, because the fix differs.
    pub async fn connect() -> Result<Self, String> {
        let session = Connection::session()
            .await
            .map_err(|e| format!("no session bus, so no accessibility bus either: {e}"))?;
        let locator = A11yBusProxy::new(&session)
            .await
            .map_err(|e| format!("cannot reach org.a11y.Bus: {e}"))?;
        let address = locator.get_address().await.map_err(|e| {
            format!("the session bus has no accessibility bus running ({e}); \
                     start at-spi-bus-launcher, or set GTK_MODULES=gail:atk-bridge for apps")
        })?;

        let conn = zbus::connection::Builder::address(address.as_str())
            .map_err(|e| format!("accessibility bus address {address:?} is not usable: {e}"))?
            .build()
            .await
            .map_err(|e| format!("cannot connect to the accessibility bus: {e}"))?;

        Ok(Self { conn, handles: HashMap::new(), next_handle: 0 })
    }

    async fn accessible(&self, r: &Reference) -> Result<AccessibleProxy<'_>, String> {
        AccessibleProxy::builder(&self.conn)
            .destination(r.0.clone())
            .and_then(|b| b.path(r.1.clone()))
            .map_err(|e| e.to_string())?
            .build()
            .await
            .map_err(|e| e.to_string())
    }

    fn hand_out(&mut self, r: Reference) -> String {
        self.next_handle += 1;
        let id = format!("w{}", self.next_handle);
        self.handles.insert(id.clone(), r);
        id
    }

    pub fn resolve(&self, id: &str) -> Option<Reference> {
        self.handles.get(id).cloned()
    }

    /// Every top-level window currently published.
    ///
    /// The registry's root has one child per application; each application's children are its
    /// windows. Anything that fails to answer is skipped rather than reported: an app that is
    /// shutting down mid-walk is normal, not news.
    pub async fn windows(&mut self) -> Result<Vec<Window>, String> {
        let root: Reference = (
            "org.a11y.atspi.Registry".to_string(),
            ObjectPath::try_from("/org/a11y/atspi/accessible/root")
                .map_err(|e| e.to_string())?
                .into(),
        );

        let apps = self.accessible(&root).await?.get_children().await.map_err(|e| e.to_string())?;
        // `zbus::fdo::DBusProxy` rather than a hand-rolled one: the method is
        // `GetConnectionUnixProcessID`, and zbus's name derivation turns a Rust
        // `get_connection_unix_process_id` into `...ProcessId`. Spelled wrong the call fails, and
        // a swallowed failure here reads as "pid 0" on every window.
        let bus = zbus::fdo::DBusProxy::new(&self.conn).await.ok();

        let mut windows = Vec::new();
        for app_ref in apps {
            let Ok(app) = self.accessible(&app_ref).await else { continue };
            let app_name = app.name().await.unwrap_or_default();
            let pid = match (&bus, zbus::names::BusName::try_from(app_ref.0.as_str())) {
                (Some(b), Ok(name)) => match b.get_connection_unix_process_id(name).await {
                    Ok(pid) => pid,
                    Err(e) => {
                        // Reported rather than silently zero. An application that cannot be
                        // attributed is still worth listing, but nobody should read a 0 as a fact.
                        tracing::debug!(bus_name = %app_ref.0, error = %e, "no pid for connection");
                        0
                    }
                },
                _ => 0,
            };
            let Ok(children) = app.get_children().await else { continue };

            for win_ref in children {
                let Ok(win) = self.accessible(&win_ref).await else { continue };
                let role = win.get_role_name().await.unwrap_or_else(|_| "window".into());
                let title = win.name().await.unwrap_or_default();
                let id = self.hand_out(win_ref);
                windows.push(Window { app: app_name.clone(), title, role, pid, id });
            }
        }
        Ok(windows)
    }

    /// What is in one window, flattened to the parts worth mentioning.
    pub async fn describe(&mut self, id: &str) -> Result<(String, Vec<Element>), String> {
        let root = self
            .resolve(id)
            .ok_or_else(|| format!("no window called `{id}`; call a11y.windows first"))?;

        let mut elements = Vec::new();
        let mut visits = 0usize;
        self.walk(&root, 0, &mut elements, &mut visits).await;

        let summary = summarise(&elements);
        Ok((summary, elements))
    }

    /// Depth-first, keeping only what carries meaning.
    ///
    /// Written as an explicit stack rather than recursion because an async fn cannot call itself
    /// without boxing, and a tree this shape would box on every node.
    async fn walk(
        &mut self,
        root: &Reference,
        _depth: usize,
        out: &mut Vec<Element>,
        visits: &mut usize,
    ) {
        let mut stack: Vec<(Reference, usize)> = vec![(root.clone(), 0)];

        while let Some((node, depth)) = stack.pop() {
            *visits += 1;
            if *visits > MAX_VISITS || out.len() >= MAX_ELEMENTS {
                return;
            }
            let Ok(acc) = self.accessible(&node).await else { continue };

            let role = acc.get_role_name().await.unwrap_or_default();
            let name = acc.name().await.unwrap_or_default();
            let text = self.text_of(&node).await;
            let actions = self.actions_of(&node).await;

            // The filter that makes this readable. A panel with no name, no text and nothing to
            // do is scaffolding — six of them stack up before a zenity dialog reaches its first
            // label — and passing scaffolding to a model is the screenshot problem again.
            let worth_saying =
                !name.is_empty() || !text.is_empty() || !actions.is_empty();

            // Children are read before the handle is minted, because minting borrows `self`
            // mutably and the proxy holds it immutably. Ending the proxy's life here rather than
            // reordering the logic keeps the reading order of the code the reading order of the
            // tree.
            let children =
                if depth < MAX_DEPTH { acc.get_children().await.ok() } else { None };
            drop(acc);

            if worth_saying {
                let id = self.hand_out(node.clone());
                out.push(Element { role, name, text, actions, id });
            }

            if let Some(children) = children {
                // Reversed, so popping the stack yields the tree in reading order.
                for child in children.into_iter().rev() {
                    stack.push((child, depth + 1));
                }
            }
        }
    }

    async fn text_of(&self, r: &Reference) -> String {
        let Ok(proxy) = TextProxy::builder(&self.conn)
            .destination(r.0.clone())
            .and_then(|b| b.path(r.1.clone()))
            .map(|b| b.build())
        else {
            return String::new();
        };
        let Ok(proxy) = proxy.await else { return String::new() };
        let Ok(count) = proxy.character_count().await else { return String::new() };
        if count <= 0 {
            return String::new();
        }
        proxy
            .get_text(0, count.min(MAX_TEXT as i32))
            .await
            .unwrap_or_default()
            .chars()
            .take(MAX_TEXT)
            .collect()
    }

    async fn actions_of(&self, r: &Reference) -> Vec<String> {
        let Ok(proxy) = ActionProxy::builder(&self.conn)
            .destination(r.0.clone())
            .and_then(|b| b.path(r.1.clone()))
            .map(|b| b.build())
        else {
            return Vec::new();
        };
        let Ok(proxy) = proxy.await else { return Vec::new() };
        proxy
            .get_actions()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _description, _keybinding)| name)
            .take(4)
            .collect()
    }

    /// Do one of the things an element said it could do.
    ///
    /// This is the payoff. A button in a foreign application can be pressed by name, through the
    /// same interface a screen reader uses — no synthetic pointer, no guessing at coordinates,
    /// and it works on a window that is not even on top.
    pub async fn act(&self, element: &str, action: &str) -> Result<String, String> {
        let r = self
            .resolve(element)
            .ok_or_else(|| format!("no element called `{element}`; call a11y.describe first"))?;

        let proxy = ActionProxy::builder(&self.conn)
            .destination(r.0.clone())
            .and_then(|b| b.path(r.1.clone()))
            .map_err(|e| e.to_string())?
            .build()
            .await
            .map_err(|e| e.to_string())?;

        let available = proxy.get_actions().await.map_err(|e| e.to_string())?;
        let wanted = action.trim().to_lowercase();
        let index = available
            .iter()
            .position(|(name, _, _)| name.to_lowercase() == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = available.iter().map(|(n, _, _)| n.as_str()).collect();
                if names.is_empty() {
                    format!("`{element}` has nothing it can be asked to do")
                } else {
                    format!("`{element}` cannot `{action}`; it offers: {}", names.join(", "))
                }
            })?;

        match proxy.do_action(index as i32).await {
            Ok(true) => Ok(available[index].0.clone()),
            Ok(false) => Err(format!("the application refused `{action}`")),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// One line describing a window, from what is in it.
///
/// Counted by role rather than listed, because "a dialog with two text fields and two buttons" is
/// what someone would say, and naming all four is what a transcript would do.
fn summarise(elements: &[Element]) -> String {
    if elements.is_empty() {
        return "an empty window".to_string();
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for e in elements {
        *counts.entry(e.role.as_str()).or_default() += 1;
    }
    // The first named element is almost always the window's own title or heading.
    let lead = elements
        .iter()
        .find(|e| !e.name.is_empty())
        .map(|e| e.name.clone())
        .unwrap_or_else(|| "untitled".to_string());

    let mut parts: Vec<String> = counts
        .into_iter()
        .filter(|(role, _)| *role != "filler" && *role != "panel")
        .map(|(role, n)| if n == 1 { format!("1 {role}") } else { format!("{n} {role}s") })
        .collect();
    parts.sort();
    parts.truncate(5);

    if parts.is_empty() {
        lead
    } else {
        format!("{lead} — {}", parts.join(", "))
    }
}

/// Values arriving from another process are data, never instruction.
///
/// A window title is written by whoever wrote the application, and one of the things it can say is
/// something that reads like a command. Nothing here interprets these strings; they are trimmed
/// and passed along as content. Named so that stays true when someone extends this file.
#[allow(dead_code)]
fn untrusted(value: OwnedValue) -> String {
    value.downcast_ref::<&str>().map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(role: &str, name: &str) -> Element {
        Element {
            role: role.into(),
            name: name.into(),
            text: String::new(),
            actions: Vec::new(),
            id: "x".into(),
        }
    }

    #[test]
    fn a_window_is_summarised_the_way_someone_would_say_it() {
        let elements = vec![
            el("dialog", "Expenses"),
            el("label", "Merchant"),
            el("text", ""),
            el("text", ""),
            el("push button", "Cancel"),
            el("push button", "OK"),
        ];
        let s = summarise(&elements);
        assert!(s.starts_with("Expenses"), "the lead should be what the window calls itself: {s}");
        assert!(s.contains("2 push buttons"), "{s}");
        assert!(s.contains("2 texts"), "{s}");
    }

    #[test]
    fn scaffolding_is_not_counted() {
        // Six nested panels is what a real zenity dialog looks like, and none of them is a thing
        // a person would mention.
        let mut elements = vec![el("dialog", "Expenses")];
        elements.extend((0..6).map(|_| el("panel", "")));
        let s = summarise(&elements);
        assert!(!s.contains("panel"), "layout must not appear in the summary: {s}");
    }

    #[test]
    fn an_empty_window_says_so_rather_than_inventing_content() {
        assert_eq!(summarise(&[]), "an empty window");
    }

    #[test]
    fn the_caps_are_small_enough_to_stay_a_glance() {
        // The point of this over a screenshot is that it is cheap to read. A cap large enough to
        // dump a document would give that away.
        assert!(MAX_ELEMENTS <= 200);
        assert!(MAX_TEXT <= 1000);
        assert!(MAX_DEPTH <= 20);
    }
}
