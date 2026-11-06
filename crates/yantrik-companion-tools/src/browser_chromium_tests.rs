//! `browser_outward::DESCRIBE_FN` run in a real Chromium, in an isolated world, through the same
//! `CdpPage` the tools use. The fake-page tests feed canned descriptions, so none of the script
//! ran under test, and the security review of #1063 found four ways it described the wrong thing
//! (M1, M2, L6). Skipped where no Chromium is found (`CHROME`, Playwright's, or one on PATH).

use super::*;
use crate::browser_outward::{Described, DESCRIBE_FN};

fn chrome() -> Option<std::path::PathBuf> {
    if let Some(c) = std::env::var_os("CHROME") {
        return Some(c.into());
    }
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir("/opt/pw-browsers")
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|d| [d.path().join("chrome-linux64/chrome"), d.path().join("chrome-headless-shell-linux64/chrome-headless-shell")])
        .filter(|p| p.exists())
        .collect();
    found.sort();
    found.pop().or_else(|| {
        ["chromium", "chromium-browser", "google-chrome"].iter().find_map(|n| {
            std::process::Command::new("which").arg(n).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().into())
        })
    })
}

struct Browser {
    child: std::process::Child,
    profile: std::path::PathBuf,
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

fn browser() -> Option<Browser> {
    let Some(bin) = chrome() else {
        eprintln!("skipped: no Chromium here");
        return None;
    };
    let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
    let profile = std::env::temp_dir().join(format!("yantrik-describe-{}-{port}", std::process::id()));
    let child = std::process::Command::new(bin)
        .args(["--headless=new", "--no-sandbox", "--disable-gpu", "--no-first-run"])
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--remote-allow-origins=http://127.0.0.1:{port}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut page_ws = None;
    for _ in 0..100 {
        if let Ok(r) = ureq::get(&format!("http://127.0.0.1:{port}/json/list")).call() {
            let list: Value = r.into_json().unwrap_or_default();
            page_ws = list.as_array().into_iter().flatten().find(|t| t["type"] == "page").and_then(|t| t["webSocketDebuggerUrl"].as_str().map(str::to_string));
            if page_ws.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let ws = connect_tab(&page_ws?).ok()?;
    Some(Browser { child, profile, ws })
}

/// The description of `#t` in `html`, made as the tools make it.
fn describe(b: &mut Browser, html: &str, typed: Option<&str>) -> Described {
    let tree = cdp_send(&mut b.ws, "Page.getFrameTree", json!({})).unwrap();
    let frame = tree["frameTree"]["frame"]["id"].as_str().unwrap().to_string();
    cdp_send(&mut b.ws, "Page.setDocumentContent", json!({ "frameId": frame, "html": html })).unwrap();
    let mut page = CdpPage { ws: &mut b.ws };
    let arg = typed.map(Value::from).unwrap_or(Value::Null);
    let json = page.on_element(&Pick::Selector("#t".into()), DESCRIBE_FN, &[arg]).unwrap();
    Described::parse(&json).unwrap_or_else(|| panic!("{json}"))
}

#[test]
fn describe_fn_names_where_a_form_really_goes_and_what_really_acts() {
    let Some(mut b) = browser() else { return };

    // M1: a button with no `formaction` goes to the form's action, not the page's own address.
    let d = describe(&mut b, r#"<form action="https://evil.example/x" method=post><input name=qty value=1><button id=t>Send</button></form>"#, None);
    assert_eq!((d.destination.as_str(), d.method.as_str()), ("https://evil.example/x", "post"), "{d:?}");

    // M1: fields named `action`, `method` and `elements` cannot stand in for the form's own.
    let d = describe(
        &mut b,
        r#"<form action="https://evil.example/x" method=post><input type=hidden name=action value=a><input type=hidden name=method value=get>
           <input name=elements><input type=hidden name=card value=4111111111111111><button id=t>Pay</button></form>"#,
        None,
    );
    assert_eq!((d.destination.as_str(), d.method.as_str()), ("https://evil.example/x", "post"), "{d:?}");
    assert!(d.fields.iter().any(|f| f == "card=4111111111111111"), "{d:?}");

    // M1: Enter submits through the default button, so typing names its `formaction`.
    let d = describe(&mut b, r#"<form action="https://shop.example/shop"><input id=t name=q><button formaction="https://elsewhere.example/go">Go</button></form>"#, Some("x"));
    assert_eq!(d.destination, "https://elsewhere.example/go", "{d:?}");
    assert!(d.fields.contains(&"q=x".to_string()), "{d:?}");

    // M2: a button inside an editing host is a button, not a field; the host itself is a field.
    let d = describe(&mut b, r#"<div contenteditable><button id=t form=f>Pay</button></div><form id=f action="https://pay.example/pay" method=post></form>"#, None);
    assert!(d.acts && d.destination == "https://pay.example/pay", "{d:?}");
    let d = describe(&mut b, r#"<div contenteditable id=t>notes</div>"#, None);
    assert!(!d.acts && d.kind == "field", "{d:?}");

    // M2: a link a framework turns into a DELETE or a POST is not a plain link; a plain one is.
    for html in [
        r#"<a id=t href="https://x.example/posts/1" data-method="delete">Delete</a>"#,
        r#"<a id=t href="https://x.example/posts/1" data-turbo-method="delete">Delete</a>"#,
        r#"<a id=t href="https://x.example/posts/1" hx-post="/x">Go</a>"#,
        // L6: the scheme as the browser parses it, not as the attribute is spelled.
        r#"<a id=t href=" javascript:void(0)">x</a>"#,
        r#"<a id=t href="java&#9;script:void(0)">x</a>"#,
    ] {
        assert!(describe(&mut b, html, None).acts, "{html}");
    }
    let d = describe(&mut b, r#"<a id=t href="https://news.example/story">Story</a>"#, None);
    assert!(!d.acts && d.kind == "link", "{d:?}");
}
