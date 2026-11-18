//! The Mind's view of Network: a projection of the committed model, in the same keys and words
//! the app has always answered, so `yos check network` reads the same.

use serde_json::{json, Value};
use yantrik_app::control::View;

use crate::model::Model;
use crate::reading::text_or_null;

pub fn view(m: &Model) -> View {
    let r = &m.reading;

    let interfaces: Vec<Value> = r
        .ethernet_rows()
        .iter()
        .map(|i| {
            json!({
                "name": i.name,
                "type": "ethernet",
                "state": i.status,
                "ip": text_or_null(&i.ip_address),
                "mac": text_or_null(&i.mac_address),
                // What the pane shows, null where it says "not reported".
                "speed": text_or_null(&i.speed),
                "subnet": text_or_null(&i.subnet),
                "gateway": text_or_null(&i.gateway),
                "dns": text_or_null(&i.dns),
            })
        })
        .collect();

    let known: Vec<Value> = r
        .known
        .iter()
        .map(|k| json!({ "ssid": k.ssid, "active": k.is_active }))
        .collect();

    let in_range: Vec<Value> = m
        .in_range
        .iter()
        .map(|n| {
            json!({
                "ssid": n.ssid,
                "signal": n.signal,
                "security": n.security,
                "connected": n.is_connected,
                "saved": n.is_saved,
            })
        })
        .collect();

    // Trouble leads the summary, the way a failed service leads the shell's: it is the reason to
    // look. "No Wi-Fi adapter" is a fact about the machine and not trouble, so it does not lead —
    // but it is never rendered as "Wi-Fi off" either.
    let summary = if !m.read_once {
        // Before the first reading lands (it is a job now, not a call on the UI thread):
        // "offline" here would be a reading nobody took.
        "Network — not read yet".to_string()
    } else if let Some(e) = &r.service_error {
        format!("Network — the network service did not answer: {e}")
    } else if r.status.connected() {
        let via = if r.status.is_wifi() {
            r.wifi.connected_ssid.clone().unwrap_or_else(|| "WiFi".to_string())
        } else {
            r.status.kind.clone()
        };
        format!("Network — online via {via}, {}", r.status.ip)
    } else {
        "Network — offline".to_string()
    };

    View::new(summary)
        .with("connected", r.status.connected())
        .with("type", text_or_null(&r.status.kind))
        .with("ip_address", text_or_null(&r.status.ip))
        .with("interfaces", Value::Array(interfaces))
        .with(
            "dns",
            json!({
                "nameservers": r.dns.nameservers,
                "search_domains": r.dns.search_domains,
            }),
        )
        .with(
            "wifi",
            json!({
                "adapter_present": r.wifi.adapter_present,
                "device": r.wifi.device,
                // "on", "off" or "unknown" — never a bool, because a bool cannot say "there is
                // no radio" and this app used to answer false for that.
                "radio": r.wifi.radio.as_str(),
                "connected_ssid": r.wifi.connected_ssid,
                "signal": r.wifi.signal,
                "reason": r.wifi.reason,
                "known": Value::Array(known),
                "in_range": Value::Array(in_range),
            }),
        )
        .with(
            "firewall",
            json!({
                "kind": r.firewall.kind,
                "state": r.firewall.state.as_str(),
                // null, not 0. An active firewall whose ruleset this session may not read has an
                // unknown number of rules, and zero is a number somebody counted.
                "rules": r.firewall.rule_count,
                "reason": r.firewall.reason,
            }),
        )
        .with(
            "source",
            json!({
                "service": "network-service",
                "answered": r.service_ok,
                "read_at": r.read_at,
                // What each half of this view was read with, so a caller can judge it.
                "interfaces": "/proc/net/dev, SIOCGIFADDR and SIOCGIFNETMASK, /sys/class/net/<if>/speed and /proc/net/route, via network-service",
                "dns": "/etc/resolv.conf, or systemd-resolved's upstream list behind its stub, via network-service",
                "wifi": "/sys/class/net for the adapter, nmcli for the rest",
                "firewall": "nft, ufw or firewall-cmd, read unprivileged",
                "error": r.service_error,
            }),
        )
        // Said twice: the strip under the header is the person's half of this.
        .with("notice", text_or_null(&m.notice))
}
