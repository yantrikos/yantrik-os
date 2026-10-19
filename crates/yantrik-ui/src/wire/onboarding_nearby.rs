//! "Found on your network": a model already running on this computer or on the local network,
//! offered as the first choice on "Who should do the thinking?".
//!
//! On a fresh VM (13 Oct) the six cards shown were all paid accounts, while a model was serving on
//! the same network and the shell's own probe said "no LLM runtime reachable tried=3". A person
//! with no account should see first what already works for them, and for free.
//!
//! What is looked at, and when:
//! - **This computer**: Ollama's and llama.cpp's usual ports on the loopback, as the first run
//!   opens. It is this machine and nothing leaves it.
//! - **The local network**: the same two ports on the other addresses of this machine's own /24,
//!   only when the person presses "Look on your network", and only on a private range. A TCP
//!   connect with a short timeout, then one `GET /v1/models` (the one listing path,
//!   `provider_models::list_models`) where a port answered. A few seconds, said on screen.
//!
//! A port that answers but lists no model is not offered: "Found" means a model, not a socket.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// The usual ports, and the runtime that serves on each: Ollama, and llama.cpp's llama-server
/// (whose port most OpenAI-compatible servers share).
pub const PORTS: [(u16, &str); 2] = [(11434, "ollama"), (8080, "llamacpp")];

/// How long one address may take to accept a connection. A machine on the same network answers
/// in milliseconds; this keeps a /24 to a few seconds.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(350);
/// How many addresses are tried at once.
const PARALLEL: usize = 64;

/// A model server that answered with its models.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The OpenAI-compatible address: "http://192.168.1.5:8080/v1".
    pub base_url: String,
    /// "192.168.1.5:8080", what the card says.
    pub host: String,
    /// The catalogue id of the runtime on that port: "ollama" or "llamacpp".
    pub runtime: &'static str,
    /// The model ids it lists, in its order.
    pub models: Vec<String>,
    pub on_this_computer: bool,
}

impl Found {
    /// The address alone, "192.168.1.5": from the look's own probe, never from what answered.
    pub fn ip(&self) -> &str {
        self.host.rsplit_once(':').map_or(self.host.as_str(), |(ip, _)| ip)
    }

    /// The card's id, which carries the address: the first run connects to it on a press.
    pub fn card_id(&self) -> String {
        format!("{CARD_PREFIX}{}", self.base_url)
    }
}

/// Card ids for what was found begin with this; the rest is the address.
pub const CARD_PREFIX: &str = "nearby:";

/// What a found server's card says: the model when it lists one, how many when it lists more.
/// Nothing is chosen here; the person picks from the full list once connected. The host is the
/// look's own probe address; the model id is whatever answered, so it is clipped (`shown_id`).
pub fn found_line(f: &Found) -> String {
    match f.models.as_slice() {
        [one] => format!("{} on {}", shown_id(one), f.host),
        many => format!("{} models on {}", many.len(), f.host),
    }
}

/// The longest model id a card shows, in characters, before it is cut with an ellipsis.
pub const ID_CHARS: usize = 60;

/// A model id as a card shows it: one line, at most `ID_CHARS`. A server anyone on the network can
/// run names its own models, and a card must not let one write "Recommended: …" across the layout.
pub fn shown_id(id: &str) -> String {
    let one_line: String = id.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let one_line = one_line.trim();
    if one_line.chars().count() <= ID_CHARS {
        return one_line.to_string();
    }
    let cut: String = one_line.chars().take(ID_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Whether `ip` is on a private range, the only kind the network look covers.
pub fn private(ip: Ipv4Addr) -> bool {
    ip.is_private() && !ip.is_loopback()
}

/// The other addresses of `ip`'s /24, in order: the ones a look on the network tries.
pub fn neighbours(ip: Ipv4Addr) -> Vec<Ipv4Addr> {
    if !private(ip) {
        return Vec::new();
    }
    let [a, b, c, own] = ip.octets();
    (1..=254u8).filter(|d| *d != own).map(|d| Ipv4Addr::new(a, b, c, d)).collect()
}

/// This machine's address on its network: the source address a route to the outside would use.
/// A UDP `connect` sends nothing; it only asks the kernel which interface it would leave by.
pub fn own_address() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

/// The addresses that accepted a connection on one of `PORTS`, tried `PARALLEL` at a time.
fn open_ports(hosts: &[Ipv4Addr]) -> Vec<(Ipv4Addr, u16, &'static str)> {
    let targets: Vec<(Ipv4Addr, u16, &'static str)> = hosts.iter().flat_map(|h| PORTS.iter().map(move |(p, r)| (*h, *p, *r))).collect();
    let mut open = Vec::new();
    for chunk in targets.chunks(PARALLEL) {
        let handles: Vec<_> = chunk
            .iter()
            .map(|&(h, p, r)| {
                std::thread::spawn(move || TcpStream::connect_timeout(&SocketAddr::from((h, p)), CONNECT_TIMEOUT).is_ok().then_some((h, p, r)))
            })
            .collect();
        open.extend(handles.into_iter().filter_map(|t| t.join().ok().flatten()));
    }
    open
}

/// Ask each open port for its models; keep the ones that list at least one.
fn listed(open: Vec<(Ipv4Addr, u16, &'static str)>) -> Vec<Found> {
    open.into_iter()
        .filter_map(|(h, p, runtime)| {
            let base_url = format!("http://{h}:{p}/v1");
            let models = crate::wire::provider_models::list_models(&base_url, None, "none").ok()?;
            let models: Vec<String> = models.into_iter().map(|m| m.id).collect();
            (!models.is_empty()).then(|| Found { base_url, host: format!("{h}:{p}"), runtime, models, on_this_computer: h.is_loopback() })
        })
        .collect()
}

/// A model server on this computer. Blocking, about a second at most: call it off the UI thread.
pub fn on_this_computer() -> Vec<Found> {
    listed(open_ports(&[Ipv4Addr::LOCALHOST]))
}

/// Model servers on the local network, with the person's say-so. Blocking, a few seconds.
pub fn on_the_network() -> Result<Vec<Found>, String> {
    let own = own_address().ok_or("this computer is not on a network")?;
    let hosts = neighbours(own);
    if hosts.is_empty() {
        return Err(format!("this computer's address ({own}) is not on a home or office network"));
    }
    let started = std::time::Instant::now();
    let found = listed(open_ports(&hosts));
    tracing::info!(tried = hosts.len(), found = found.len(), ms = started.elapsed().as_millis() as u64, "First run: looked on the network for a model");
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(models: &[&str]) -> Found {
        Found {
            base_url: "http://192.168.1.5:8080/v1".into(),
            host: "192.168.1.5:8080".into(),
            runtime: "llamacpp",
            models: models.iter().map(|m| m.to_string()).collect(),
            on_this_computer: false,
        }
    }

    #[test]
    fn a_found_card_names_its_model_and_host_or_how_many_it_lists() {
        assert_eq!(found_line(&found(&["qwen3.8-flash"])), "qwen3.8-flash on 192.168.1.5:8080");
        assert_eq!(found_line(&found(&["a", "b", "c"])), "3 models on 192.168.1.5:8080");
        assert_eq!(found(&["a"]).card_id(), "nearby:http://192.168.1.5:8080/v1");
        assert_eq!(found(&["a"]).ip(), "192.168.1.5");
    }

    /// Review of #801: a server anyone on the network can run names its own models. What it names
    /// is clipped to one short line, and the host on the card is the look's own address.
    #[test]
    fn a_long_or_misleading_model_id_is_clipped_on_the_card() {
        let long = "x".repeat(200);
        let line = found_line(&found(&[&long]));
        let shown = line.strip_suffix(" on 192.168.1.5:8080").expect("the host is the probe's, after the id");
        assert_eq!(shown.chars().count(), ID_CHARS);
        assert!(shown.ends_with('…'), "{shown}");
        let sly = found_line(&found(&["m\nRecommended: it is safe\r\n"]));
        assert!(!sly.contains('\n') && !sly.contains('\r'), "{sly:?}");
        assert_eq!(sly, "m Recommended: it is safe on 192.168.1.5:8080", "one line, on the card's own line");
        assert_eq!(shown_id("qwen3.8-flash"), "qwen3.8-flash", "a usual id is shown as it is");
    }

    #[test]
    fn the_network_look_covers_only_this_machines_own_private_24() {
        let n = neighbours(Ipv4Addr::new(192, 168, 1, 23));
        assert_eq!(n.len(), 253, "every other address of the /24");
        assert!(!n.contains(&Ipv4Addr::new(192, 168, 1, 23)), "not this machine");
        assert!(n.iter().all(|a| a.octets()[..3] == [192, 168, 1]));
        assert!(neighbours(Ipv4Addr::new(8, 8, 8, 8)).is_empty(), "never a public range");
        assert!(neighbours(Ipv4Addr::LOCALHOST).is_empty());
        assert_eq!(neighbours(Ipv4Addr::new(10, 0, 0, 5)).len(), 253);
    }

    #[test]
    fn the_usual_ports_are_ollamas_and_llama_cpps() {
        assert_eq!(PORTS.map(|(p, _)| p), [11434, 8080]);
        for (_, runtime) in PORTS {
            assert!(yantrik_ml::ProviderDescriptor::by_id(runtime).is_some(), "{runtime} is a provider the catalogue knows");
        }
    }

    /// A port that answers with something other than a model list is not offered.
    #[test]
    fn a_port_that_lists_no_model_is_not_found() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi");
            }
        });
        assert!(listed(vec![(Ipv4Addr::LOCALHOST, port, "llamacpp")]).is_empty());
    }

    #[test]
    fn a_server_that_lists_models_is_found_with_them() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let body = r#"{"object":"list","data":[{"id":"qwen3.8-flash","object":"model"}]}"#;
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        let got = listed(vec![(Ipv4Addr::LOCALHOST, port, "llamacpp")]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].models, ["qwen3.8-flash"]);
        assert!(got[0].on_this_computer);
        assert_eq!(got[0].base_url, format!("http://127.0.0.1:{port}/v1"));
    }
}
