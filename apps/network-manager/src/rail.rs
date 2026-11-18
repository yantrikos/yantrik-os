//! What the agent rail knows and offers here.
//!
//! Each row says where it came from. A section with nothing true to say does not draw, which on
//! this screen is most of the time: a machine with no Wi-Fi adapter has one interesting fact
//! about it and the rail says that one rather than four empty ones.

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};
use yantrik_ipc_contracts::network::FirewallStatus;

use crate::model::Model;
use crate::update::EXPLAIN;

/// Where the answer card says its answer came from.
const FROM_SCREEN: &str = "from what is on this screen";

/// What to ask the companion about what is on screen.
///
/// One place, because two controls ask it: the rail's suggestion and the header's Explain button.
/// Only what has been read is sent — no SSID this app has not seen, and never a password.
pub fn network_question(m: &Model) -> String {
    let r = &m.reading;
    let wifi = if !r.wifi.adapter_present {
        "Wi-Fi: this machine has no Wi-Fi adapter.".to_string()
    } else {
        format!(
            "Wi-Fi: radio {}, joined to {}.",
            r.wifi.radio.as_str(),
            r.wifi.connected_ssid.as_deref().unwrap_or("nothing")
        )
    };
    let firewall = match r.firewall.state {
        FirewallStatus::Unknown => format!(
            "Firewall: could not be determined ({}).",
            r.firewall.reason.clone().unwrap_or_default()
        ),
        other => format!(
            "Firewall: {} ({}).",
            other.as_str(),
            r.firewall.kind.clone().unwrap_or_else(|| "no tool".to_string())
        ),
    };
    format!(
        "This is what a machine's network settings report. In at most four short lines say how \
         this machine is connected and whether anything about it is worth attention. Use only \
         what is shown.\n\nLink: {} {}\n{wifi}\n{firewall}\nResolvers: {}",
        r.status.kind,
        r.status.ip,
        if r.dns.nameservers.is_empty() {
            "none configured".to_string()
        } else {
            r.dns.nameservers.join(", ")
        }
    )
}

impl Source for Model {
    fn context(&self) -> Vec<ContextItem> {
        // Nothing is said before the first reading: a row about a machine nobody has read yet
        // would be a guess.
        if !self.read_once {
            return Vec::new();
        }
        let r = &self.reading;
        let link = ContextItem {
            id: "link".into(),
            label: if r.status.connected() {
                format!("{} — {}", r.status.kind, r.status.ip)
            } else {
                "Not connected".to_string()
            },
            detail: "from network-service".into(),
            source: "file".into(),
        };
        let wifi = ContextItem {
            id: "wifi".into(),
            label: if !r.wifi.adapter_present {
                "No Wi-Fi adapter".to_string()
            } else {
                match r.wifi.connected_ssid.as_deref() {
                    Some(ssid) => format!("Wi-Fi on {ssid}"),
                    None => format!("Wi-Fi radio {}", r.wifi.radio.as_str()),
                }
            },
            // Where the reading came from, in words; the kernel's file is the source either way.
            detail: r
                .wifi
                .device
                .clone()
                .unwrap_or_else(|| "as this computer reports it".to_string()),
            source: "file".into(),
        };
        let firewall = ContextItem {
            id: "firewall".into(),
            label: match r.firewall.state {
                FirewallStatus::Active => format!(
                    "Firewall filtering ({})",
                    r.firewall.kind.clone().unwrap_or_default()
                ),
                FirewallStatus::Inactive => "Firewall installed, not filtering".to_string(),
                // Only the tools were looked for; the kernel's rules were not read.
                FirewallStatus::Absent => {
                    "No supported firewall manager detected; active filtering rules not checked".to_string()
                }
                FirewallStatus::Unknown => "Firewall state unknown".to_string(),
            },
            detail: r
                .firewall
                .reason
                .clone()
                .unwrap_or_else(|| "read at this machine's privilege".to_string()),
            source: "file".into(),
        };
        vec![link, wifi, firewall]
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        if reach != Reach::Ready || !self.read_once {
            return Vec::new();
        }
        vec![Suggestion {
            id: EXPLAIN.into(),
            label: "Explain this machine's network".into(),
            detail: "reads what is on this screen".into(),
            icon: "spark".into(),
            proposes: false,
        }]
    }

    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        (suggestion == EXPLAIN).then(|| Prompt {
            text: network_question(self),
            title: "Reading this machine's network".into(),
            source: FROM_SCREEN.into(),
        })
    }

    fn proposal(&self, _suggestion: &str, answer: &str) -> Proposal {
        // The card's verb is "Close": what it holds is an answer, not a change to apply.
        Proposal {
            title: "This machine's network".into(),
            body: answer.into(),
            source: FROM_SCREEN.into(),
            verb: "Close".into(),
            ..Default::default()
        }
    }
}

impl HasRail for Model {
    fn rail(&self) -> &Rail {
        &self.rail
    }

    fn rail_mut(&mut self) -> &mut Rail {
        &mut self.rail
    }
}
