//! Which section the window opens on.
//!
//! It opened on Wi-Fi whatever the machine was, so a desktop with no Wi-Fi adapter opened on
//! "No adapter" while its traffic went out over a cable. It now opens on the interface that
//! carries traffic: the one the kernel's default route leaves by.
//!
//! Pure: the route table arrives as text, so the tests need no kernel and no window.

/// The route table and its reader are shared with network-service, which reads the same table
/// for each interface's gateway: one parser, so the two cannot disagree on the default route.
pub use yantrik_ipc_contracts::route_table::{default_route_interface, ROUTE_TABLE};

/// The window's sections, as `active-tab` numbers them.
pub const WIFI: i32 = 0;
pub const ETHERNET: i32 = 1;

/// The section to open on.
///
/// Ethernet when the default route leaves by a wired interface. Ethernet too when there is no
/// Wi-Fi adapter and there is a wired interface, whatever the route table said: "No adapter"
/// is the one thing that section can tell this machine, and the rail already says it. Wi-Fi
/// otherwise, which is where the traffic goes or where a person comes to connect.
pub fn opening_tab(route_iface: Option<&str>, ethernet: &[String], wifi_adapter_present: bool) -> i32 {
    let wired_route = route_iface.is_some_and(|iface| ethernet.iter().any(|e| e == iface));
    if wired_route || (!wifi_adapter_present && !ethernet.is_empty()) {
        ETHERNET
    } else {
        WIFI
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    // The route table's own cases are with its parser, in `yantrik_ipc_contracts::route_table`.
    #[test]
    fn the_shared_reader_finds_the_wired_default_route() {
        let t = [
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT",
            "eth0\t00000000\t0104A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0",
        ]
        .join("\n");
        let eth = names(&["eth0"]);
        assert_eq!(opening_tab(default_route_interface(&t).as_deref(), &eth, true), ETHERNET);
    }

    /// The reported case: no Wi-Fi adapter, a cable carrying the traffic.
    #[test]
    fn a_wired_machine_with_no_radio_opens_on_ethernet() {
        let eth = names(&["eth0"]);
        assert_eq!(opening_tab(Some("eth0"), &eth, false), ETHERNET);
        // Even when the route table could not be read.
        assert_eq!(opening_tab(None, &eth, false), ETHERNET);
    }

    #[test]
    fn a_machine_on_its_radio_opens_on_wifi() {
        let eth = names(&["eth0"]);
        assert_eq!(opening_tab(Some("wlan0"), &eth, true), WIFI);
        assert_eq!(opening_tab(Some("eth0"), &eth, true), ETHERNET);
        // Nothing wired and nothing routed: Wi-Fi is where a person comes to connect.
        assert_eq!(opening_tab(None, &[], true), WIFI);
    }
}
