//! What nmcli's and the firewall tools' answers mean, tested without either on the machine.
//!
//! The faults these cover were invisible to a reader of either half of the app. The Network
//! Manager app called five methods — `wifi_toggle`, `wifi_scan`, `wifi_connect`,
//! `wifi_disconnect`, `wifi_forget` — that `network-service` did not implement, and three of the
//! five went into a `let _ =`, so every press logged that it had happened. And two properties the
//! screen drew as measurements, `wifi-enabled` and `firewall-enabled`, were Slint defaults that
//! Rust never wrote: "Firewall: Off" reached a security audit of this OS as a finding about a
//! machine nothing had ever looked at.
//!
//! So the decisions live in modules with no window and no socket in them — `nmcli` and
//! `firewall` under the service, `connect` under the app — and this is where they are checked.
//! The machine this runs on has no nmcli, no adapter and no firewall, which is the same position
//! `tests/container-core` is in with docker — and the same reason it works: every case below is a
//! captured string, including the ones a machine with working hardware could never produce on
//! demand.

#[path = "../../services/network-service/src/nmcli.rs"]
pub mod nmcli;

#[path = "../../services/network-service/src/firewall.rs"]
pub mod firewall;

#[path = "../../apps/network-manager/src/connect.rs"]
pub mod connect;

#[cfg(test)]
mod wire {
    //! The wire, as both ends speak it.
    //!
    //! Calendar's two ends disagreed about parameter *names*. Network's disagreed about every
    //! method name there was. These serialize what the app sends and parse it the way the service
    //! parses it, which is the drift that made every button in this app a no-op.

    use yantrik_ipc_contracts::network::*;

    #[test]
    fn every_method_the_app_calls_is_a_constant_both_ends_read() {
        // The five the app used to call by hand, and the names the service answered to. Written
        // out here on purpose: if a constant is renamed, this is the file that says what the old
        // wire looked like.
        assert_eq!(method::WIFI_SCAN, "network.wifi_scan");
        assert_eq!(method::WIFI_CONNECT, "network.wifi_connect");
        assert_eq!(method::WIFI_DISCONNECT, "network.wifi_disconnect");
        assert_eq!(method::WIFI_FORGET, "network.wifi_forget");
        assert_eq!(method::INTERFACES, "network.interfaces");
        assert_eq!(method::STATUS, "network.status");
        assert_eq!(method::DNS, "network.dns");
        // `wifi_toggle` is gone. It sent `{"enabled": !current}` computed from a property nothing
        // wrote, so it asked to turn on a radio that was already on as often as not.
        assert_eq!(method::WIFI_RADIO, "network.wifi_radio");
    }

    #[test]
    fn the_radio_request_carries_the_state_asked_for_not_a_flip() {
        let sent = serde_json::to_value(WifiRadioParams { enabled: true }).unwrap();
        let read: WifiRadioParams = serde_json::from_value(sent.clone()).unwrap();
        assert!(read.enabled);
        assert_eq!(sent, serde_json::json!({ "enabled": true }));
    }

    #[test]
    fn a_connect_without_a_password_does_not_send_the_field_at_all() {
        let sent = serde_json::to_value(WifiConnectParams {
            ssid: "HomeNet".into(),
            password: None,
        })
        .unwrap();
        assert!(
            sent.get("password").is_none(),
            "an open or saved network sends no secret: {sent}"
        );
        let read: WifiConnectParams = serde_json::from_value(sent).unwrap();
        assert_eq!(read.ssid, "HomeNet");
        assert_eq!(read.password, None);
    }

    #[test]
    fn a_connect_with_a_password_round_trips_and_never_prints_it() {
        let params = WifiConnectParams {
            ssid: "HomeNet".into(),
            password: Some("hunter2-the-real-one".into()),
        };
        let sent = serde_json::to_value(&params).unwrap();
        let read: WifiConnectParams = serde_json::from_value(sent).unwrap();
        assert_eq!(read.password.as_deref(), Some("hunter2-the-real-one"));

        // The one that matters. `{:?}` turns up in log lines, traces and hand-built error
        // strings, and a passphrase reaching any of those is the thing this whole path avoids.
        let printed = format!("{params:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(printed.contains("<given>"), "{printed}");
        let printed_none = format!("{:?}", WifiConnectParams { ssid: "x".into(), password: None });
        assert!(printed_none.contains("<none>"), "{printed_none}");
    }

    #[test]
    fn the_forget_request_names_the_network_the_way_the_service_looks_it_up() {
        let sent = serde_json::to_value(WifiForgetParams { ssid: "HomeNet".into() }).unwrap();
        let read: WifiForgetParams = serde_json::from_value(sent.clone()).unwrap();
        assert_eq!(read.ssid, "HomeNet");
        assert!(sent.get("name").is_none(), "the service reads `ssid`");
    }

    #[test]
    fn a_scan_request_with_nothing_in_it_is_a_cached_read() {
        // The three-second refresh sends `{}`. Defaulting `rescan` to false is what keeps that
        // refresh from putting the radio off the air twenty times a minute.
        let read: WifiScanParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!read.rescan);
        let sent = serde_json::to_value(WifiScanParams { rescan: true }).unwrap();
        let read: WifiScanParams = serde_json::from_value(sent).unwrap();
        assert!(read.rescan);
    }

    #[test]
    fn the_interface_list_the_service_sends_is_the_list_the_app_reads() {
        // The app used to pull this apart field by field out of a `serde_json::Value` with
        // `unwrap_or_default()` over the whole vec, so a shape change emptied the ethernet pane
        // in silence. Both ends now build this struct.
        let rows = vec![NetworkInterfaceInfo {
            name: "eth0".into(),
            mac_address: "52:54:00:12:34:56".into(),
            ip_address: Some("10.0.0.5".into()),
            rx_bytes: 1,
            tx_bytes: 2,
            state: "up".into(),
            conn_type: ConnectionType::Ethernet,
        }];
        let sent = serde_json::to_value(&rows).unwrap();
        let read: Vec<NetworkInterfaceInfo> = serde_json::from_value(sent).unwrap();
        assert_eq!(read[0].name, "eth0");
        assert_eq!(read[0].conn_type, ConnectionType::Ethernet);
        assert_eq!(read[0].conn_type.as_str(), "ethernet");
    }

    #[test]
    fn an_interface_list_of_the_wrong_shape_is_an_error_and_not_an_empty_list() {
        // This is the trap that was left in the app: `unwrap_or_default()` turned a mismatch into
        // "this machine has no interfaces". The typed parse fails instead, and the app puts the
        // failure in the notice.
        let wrong = serde_json::json!([{ "name": "eth0", "state": 17 }]);
        let read: Result<Vec<NetworkInterfaceInfo>, _> = serde_json::from_value(wrong);
        assert!(read.is_err(), "a shape mismatch has to be an error");
    }

    #[test]
    fn a_radio_state_is_three_words_on_the_wire_and_never_a_bool() {
        assert_eq!(
            serde_json::to_value(RadioState::Unknown).unwrap(),
            serde_json::json!("unknown")
        );
        assert_eq!(RadioState::On.as_str(), "on");
        assert_eq!(RadioState::Off.as_str(), "off");
    }

    #[test]
    fn an_absent_adapter_and_an_off_radio_are_different_answers() {
        let no_hardware = WifiState {
            adapter_present: false,
            reason: Some("this machine has no Wi-Fi adapter".into()),
            ..WifiState::default()
        };
        let off = WifiState {
            adapter_present: true,
            device: Some("wlan0".into()),
            radio: RadioState::Off,
            ..WifiState::default()
        };
        let a = serde_json::to_value(&no_hardware).unwrap();
        let b = serde_json::to_value(&off).unwrap();
        assert_eq!(a["adapter_present"], serde_json::json!(false));
        assert_eq!(a["radio"], serde_json::json!("unknown"));
        assert_eq!(b["adapter_present"], serde_json::json!(true));
        assert_eq!(b["radio"], serde_json::json!("off"));
        assert_ne!(a, b, "the screen used to draw both of these as \"Wi-Fi: Off\"");
    }

    #[test]
    fn an_unread_rule_count_is_null_and_not_zero() {
        let unknown = FirewallState {
            kind: Some("nftables".into()),
            state: FirewallStatus::Unknown,
            rule_count: None,
            rules: Vec::new(),
            reason: Some("reading the nftables ruleset needs root".into()),
        };
        let sent = serde_json::to_value(&unknown).unwrap();
        assert_eq!(sent["rule_count"], serde_json::Value::Null);
        assert_eq!(sent["state"], serde_json::json!("unknown"));
        assert!(sent["reason"].is_string(), "unknown always carries a reason");
    }

    #[test]
    fn absent_and_inactive_are_different_states_on_the_wire() {
        assert_eq!(FirewallStatus::Absent.as_str(), "absent");
        assert_eq!(FirewallStatus::Inactive.as_str(), "inactive");
        assert_eq!(
            serde_json::to_value(FirewallStatus::Absent).unwrap(),
            serde_json::json!("absent")
        );
    }
}

#[cfg(test)]
mod terse {
    //! nmcli's `-t` output, including the shapes that break a naive split.

    use super::nmcli::*;

    #[test]
    fn a_plain_record_splits_into_its_fields() {
        assert_eq!(
            split_terse("wlan0:wifi:connected:HomeNet"),
            vec!["wlan0", "wifi", "connected", "HomeNet"]
        );
    }

    #[test]
    fn a_colon_inside_an_ssid_is_escaped_and_must_not_split_the_record() {
        // A café calling its network "Cafe: Free" is the ordinary case, and `line.split(':')`
        // turns it into two fields — so the list shows "Cafe" and this machine cannot join it.
        let parsed = split_terse(r"*:Cafe\: Free:72:WPA2:270 Mbit/s");
        assert_eq!(parsed[0], "*");
        assert_eq!(parsed[1], "Cafe: Free");
        assert_eq!(parsed[2], "72");
        assert_eq!(parsed.len(), 5);
    }

    #[test]
    fn a_backslash_inside_an_ssid_survives_the_round_trip() {
        let parsed = split_terse(r" :C\\Wifi:41:WPA3:130 Mbit/s");
        assert_eq!(parsed[1], r"C\Wifi");
    }

    #[test]
    fn a_backslash_before_something_that_is_not_an_escape_is_kept_as_written() {
        assert_eq!(split_terse(r"a\nb:c")[0], r"a\nb");
    }

    #[test]
    fn a_hidden_network_has_an_empty_ssid_and_is_listed_as_hidden_rather_than_dropped() {
        let text = " ::55:WPA2:54 Mbit/s\n";
        let rows = parse_scan(text, &[]);
        assert_eq!(rows.len(), 1, "a row dropped here is how a list says nothing is there");
        assert!(rows[0].hidden);
        assert_eq!(rows[0].ssid, "");
        assert_eq!(rows[0].signal, 55);
        assert!(!rows[0].is_saved, "a hidden network cannot match a saved name");
    }

    #[test]
    fn the_in_use_marker_is_the_connected_row() {
        let text = concat!(
            "*:HomeNet:81:WPA2:270 Mbit/s\n",
            " :Neighbour:44:WPA2:130 Mbit/s\n",
            " :OpenCafe:23:--:54 Mbit/s\n"
        );
        let rows = parse_scan(text, &["Neighbour".to_string()]);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].is_connected);
        assert!(!rows[1].is_connected);
        assert!(rows[1].is_saved, "the saved list marks the rows it matches");
        assert!(!rows[0].is_saved);
        // nmcli's own word for an open network, kept rather than reworded to "Open".
        assert_eq!(rows[2].security, "--");
        assert_eq!(rows[0].rate, "270 Mbit/s");
    }

    #[test]
    fn an_empty_scan_is_an_empty_list_and_not_an_error() {
        assert!(parse_scan("", &[]).is_empty());
        assert!(parse_scan("\n\n", &[]).is_empty());
    }

    #[test]
    fn the_saved_list_keeps_only_wireless_connections() {
        // `Wired connection 1` is a saved connection too, and "Known networks" listing it would
        // be nonsense.
        let text = concat!(
            "HomeNet:11111111-1111-1111-1111-111111111111:802-11-wireless:yes\n",
            "Wired connection 1:22222222-2222-2222-2222-222222222222:802-3-ethernet:yes\n",
            "Cafe\\: Free:33333333-3333-3333-3333-333333333333:802-11-wireless:no\n"
        );
        let known = parse_known(text);
        assert_eq!(known.len(), 2);
        assert_eq!(known[0].ssid, "HomeNet");
        assert!(known[0].is_active);
        assert_eq!(known[1].ssid, "Cafe: Free");
        assert!(!known[1].is_active);
        assert_eq!(known[1].uuid, "33333333-3333-3333-3333-333333333333");
    }

    #[test]
    fn a_machine_with_no_wireless_device_has_none_in_the_device_list() {
        // The wired test VM, exactly as `nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device status`
        // prints it there.
        let text = concat!(
            "eth0:ethernet:connected:Wired connection 1\n",
            "lo:loopback:unmanaged:\n"
        );
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 2);
        assert!(
            wifi_device(&devices).is_none(),
            "no wifi row means no adapter, which is not the same as a radio being off"
        );
        assert_eq!(devices[0].connection, "Wired connection 1");
    }

    #[test]
    fn an_unmanaged_wireless_device_is_still_an_adapter() {
        // The hardware is in the machine; NetworkManager has been told not to touch it. Reporting
        // "no adapter" here would be a second wrong answer.
        let devices = parse_devices("wlp3s0:wifi:unmanaged:\neth0:ethernet:connected:Wired\n");
        let wifi = wifi_device(&devices).expect("the adapter is present");
        assert_eq!(wifi.name, "wlp3s0");
        assert_eq!(wifi.state, "unmanaged");
    }

    #[test]
    fn the_radio_switch_reads_three_ways() {
        assert_eq!(parse_radio("enabled\n"), Some(true));
        assert_eq!(parse_radio("disabled"), Some(false));
        // Anything else is unknown rather than false. `nmcli radio wifi` answers for the software
        // switch whether or not there is hardware behind it, which is why it is never read alone.
        assert_eq!(parse_radio(""), None);
        assert_eq!(parse_radio("missing"), None);
    }

    #[test]
    fn a_device_show_gives_the_first_address_its_prefix_and_the_gateway() {
        let text = concat!(
            "GENERAL.CONNECTION:HomeNet\n",
            "GENERAL.STATE:100 (connected)\n",
            "IP4.ADDRESS[1]:192.168.1.34/24\n",
            "IP4.ADDRESS[2]:192.168.1.35/24\n",
            "IP4.GATEWAY:192.168.1.1\n",
            "GENERAL.HWADDR:AA\\:BB\\:CC\\:DD\\:EE\\:FF\n"
        );
        let detail = parse_device_show(text);
        assert_eq!(detail.address.as_deref(), Some("192.168.1.34"));
        assert_eq!(detail.prefix, Some(24));
        assert_eq!(detail.gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(detail.connection.as_deref(), Some("HomeNet"));
    }

    #[test]
    fn a_device_with_no_address_reports_none_rather_than_a_dash() {
        let detail = parse_device_show("GENERAL.CONNECTION:--\nIP4.GATEWAY:--\n");
        assert_eq!(detail.address, None);
        assert_eq!(detail.gateway, None);
        assert_eq!(detail.connection, None);
    }

    #[test]
    fn a_prefix_becomes_the_mask_the_window_has_a_field_for() {
        assert_eq!(prefix_to_mask(24).as_deref(), Some("255.255.255.0"));
        assert_eq!(prefix_to_mask(16).as_deref(), Some("255.255.0.0"));
        assert_eq!(prefix_to_mask(32).as_deref(), Some("255.255.255.255"));
        assert_eq!(prefix_to_mask(0).as_deref(), Some("0.0.0.0"));
        // Out of range is a parse that went wrong, and 0.0.0.0 would be a plausible wrong answer.
        assert_eq!(prefix_to_mask(33), None);
    }

    #[test]
    fn the_connected_ssid_comes_from_the_scan_marker_before_the_profile_name() {
        let devices = parse_devices("wlan0:wifi:connected:my-laptop-profile\n");
        let scanned = parse_scan("*:HomeNet:81:WPA2:270 Mbit/s\n", &[]);
        assert_eq!(
            connected_ssid(&scanned, wifi_device(&devices)),
            Some("HomeNet".to_string()),
            "the profile may have been renamed; the marker is the SSID"
        );
    }

    #[test]
    fn with_no_scan_row_the_profile_name_is_the_fallback() {
        let devices = parse_devices("wlan0:wifi:connected:HomeNet\n");
        assert_eq!(
            connected_ssid(&[], wifi_device(&devices)),
            Some("HomeNet".to_string())
        );
    }

    #[test]
    fn a_disconnected_device_is_joined_to_nothing() {
        let devices = parse_devices("wlan0:wifi:disconnected:--\n");
        assert_eq!(connected_ssid(&[], wifi_device(&devices)), None);
        assert_eq!(connected_ssid(&[], None), None);
    }
}

#[cfg(test)]
mod failures {
    //! Six ways this can go wrong, and why they are six and not one.

    use super::nmcli::*;

    fn ran(code: i32, stdout: &str, stderr: &str) -> Exit {
        Exit::Ran {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn a_zero_exit_hands_back_stdout() {
        assert_eq!(outcome(&ran(0, "enabled\n", "")).unwrap(), "enabled\n");
    }

    #[test]
    fn a_missing_nmcli_is_not_a_machine_with_no_networks() {
        let trouble = outcome(&Exit::Missing).unwrap_err();
        assert_eq!(trouble, Trouble::NmcliMissing);
        assert_eq!(trouble.kind(), "nmcli_missing");
        assert!(trouble.message().contains("not installed"), "{}", trouble.message());
    }

    #[test]
    fn a_stopped_daemon_says_so_in_its_own_words() {
        // nmcli's own line, as a machine where the unit is masked prints it.
        let exit = ran(8, "", "Error: NetworkManager is not running.\n");
        let trouble = outcome(&exit).unwrap_err();
        assert_eq!(trouble, Trouble::NetworkManagerDown);
        assert_eq!(trouble.kind(), "networkmanager_down");
    }

    #[test]
    fn no_wifi_hardware_is_its_own_answer() {
        let exit = ran(10, "", "Error: No Wi-Fi device found.\n");
        let trouble = outcome(&exit).unwrap_err();
        assert_eq!(trouble, Trouble::NoWifiAdapter);
        assert_eq!(
            trouble.message(),
            "this machine has no Wi-Fi adapter",
            "the sentence the app draws instead of \"Wi-Fi: Off\""
        );
    }

    #[test]
    fn a_refused_secret_is_a_wrong_password_and_not_a_missing_network() {
        // What nmcli writes when the AP rejects the PSK. The distinction matters: one invites the
        // person to type it again, the other does not.
        let exit = ran(
            4,
            "",
            "Error: Connection activation failed: (7) Secrets were required, but not provided.\n",
        );
        let trouble = outcome(&exit).unwrap_err();
        assert_eq!(trouble.kind(), "wrong_password");
        assert!(trouble.message().contains("refused the password"), "{}", trouble.message());
        // nmcli's own sentence is carried, not replaced by a guess about what it meant.
        assert!(trouble.message().contains("(7)"), "{}", trouble.message());
    }

    #[test]
    fn a_network_that_is_not_there_names_the_name_that_was_asked_for() {
        let exit = ran(10, "", "Error: No network with SSID 'nope-not-here' found.\n");
        let trouble = outcome(&exit).unwrap_err();
        assert_eq!(trouble.kind(), "ssid_not_found");
        assert!(trouble.message().contains("nope-not-here"), "{}", trouble.message());
    }

    #[test]
    fn polkit_turning_a_change_away_is_not_a_hardware_problem() {
        let exit = ran(4, "", "Error: Failed to set 'wireless' to off: Not authorized to enable/disable WiFi device\n");
        let trouble = outcome(&exit).unwrap_err();
        assert_eq!(trouble.kind(), "permission_denied");
        assert!(trouble.message().contains("not allowed"), "{}", trouble.message());
    }

    #[test]
    fn nmclis_own_wait_running_out_is_reported_as_a_timeout() {
        let exit = ran(5, "", "Error: Timeout expired (2 seconds)\n");
        assert_eq!(outcome(&exit).unwrap_err().kind(), "timed_out");
    }

    #[test]
    fn an_unrecognised_failure_is_passed_through_verbatim() {
        let exit = ran(1, "", "Error: something nobody has classified yet\n");
        let trouble = outcome(&exit).unwrap_err();
        assert_eq!(trouble.kind(), "failed");
        assert_eq!(trouble.message(), "Error: something nobody has classified yet");
    }

    #[test]
    fn a_failure_with_nothing_on_either_stream_still_says_which_status() {
        let trouble = outcome(&ran(3, "", "")).unwrap_err();
        assert!(trouble.message().contains("status 3"), "{}", trouble.message());
    }

    #[test]
    fn a_binary_that_will_not_start_is_not_a_binary_that_is_absent() {
        let trouble = outcome(&Exit::Unstartable("Permission denied (os error 13)".into()))
            .unwrap_err();
        assert_eq!(trouble.kind(), "nmcli_unstartable");
        assert!(trouble.message().contains("os error 13"), "{}", trouble.message());
    }

    #[test]
    fn the_first_line_is_the_one_worth_showing() {
        assert_eq!(first_line("\n\n  Error: one\nusage: nmcli\n").unwrap(), "Error: one");
        assert_eq!(first_line("   \n"), None);
    }
}

#[cfg(test)]
mod firewalls {
    //! The reading that used to be a Slint default drawn in warning colour.

    use super::firewall::*;
    use super::nmcli::Exit;
    use yantrik_ipc_contracts::network::FirewallStatus;

    fn ran(code: i32, stdout: &str, stderr: &str) -> Exit {
        Exit::Ran {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    const LOADED: &str = r#"table inet filter {
	chain input {
		type filter hook input priority filter; policy drop;
		ct state established,related accept
		iif "lo" accept
		ip protocol icmp accept
		tcp dport 22 accept
		counter drop
	}

	chain forward {
		type filter hook forward priority filter; policy drop;
	}

	chain output {
		type filter hook output priority filter; policy accept;
	}
}
"#;

    #[test]
    fn no_firewall_tool_at_all_is_absent_and_names_what_was_looked_for() {
        // The answer on an ISO-built machine: neither build recipe installs a firewall package,
        // and `--variant=minbase` does not bring one in.
        let state = read_nftables(&Exit::Missing);
        assert_eq!(state.state, FirewallStatus::Absent);
        assert_eq!(state.kind, None);
        assert_eq!(state.rule_count, None, "there is nothing to have counted");
        let reason = state.reason.unwrap();
        assert!(reason.contains("nft"), "{reason}");
        assert!(reason.contains("ufw"), "{reason}");
        assert!(reason.contains("firewall-cmd"), "{reason}");
    }

    #[test]
    fn a_loaded_ruleset_is_active_with_the_rules_it_actually_has() {
        let state = read_nftables(&ran(0, LOADED, ""));
        assert_eq!(state.state, FirewallStatus::Active);
        assert_eq!(state.kind.as_deref(), Some("nftables"));
        // Five rules in `input`, and none of the `type … hook …` policy lines, the chain
        // declarations or the braces.
        assert_eq!(state.rule_count, Some(5));
        assert_eq!(state.rules.len(), 5);
        assert!(state.rules.iter().all(|r| r.chain == "input"));
        assert_eq!(state.rules[0].action, "accept");
        assert_eq!(state.rules[0].text, "ct state established,related accept");
        assert_eq!(state.rules[4].action, "drop");
        assert_eq!(state.reason, None, "nothing is wrong, so nothing is said");
    }

    #[test]
    fn a_rule_the_keyword_counters_would_have_missed_is_counted() {
        // The companion's own `firewall_status` counts lines beginning with one of six words and
        // calls the answer "~N rules"; `oifname`, `counter` and `jump` are not among them.
        let text = "table inet f {\n\tchain c {\n\t\toifname \"eth0\" jump outbound\n\t}\n}\n";
        let state = read_nftables(&ran(0, text, ""));
        assert_eq!(state.rule_count, Some(1));
        assert_eq!(state.rules[0].action, "jump");
    }

    #[test]
    fn an_empty_ruleset_that_was_read_is_inactive_which_is_a_measurement() {
        let state = read_nftables(&ran(0, "", ""));
        assert_eq!(state.state, FirewallStatus::Inactive);
        assert_eq!(state.rule_count, Some(0), "this zero was counted");
        assert_eq!(state.kind.as_deref(), Some("nftables"));
    }

    #[test]
    fn a_ruleset_this_session_may_not_read_is_unknown_and_says_why() {
        // The case on every machine this OS ships: `nft list ruleset` needs CAP_NET_ADMIN and the
        // desktop session does not have it.
        let exit = ran(1, "", "Error: Could not process rule: Operation not permitted\n");
        let state = read_nftables(&exit);
        assert_eq!(state.state, FirewallStatus::Unknown);
        assert_eq!(state.kind.as_deref(), Some("nftables"));
        assert_eq!(state.rule_count, None, "not zero — nobody counted");
        let reason = state.reason.unwrap();
        assert!(reason.contains("root"), "{reason}");
        assert!(reason.contains("Operation not permitted"), "{reason}");
    }

    #[test]
    fn ufw_reports_active_with_the_rows_under_the_rule() {
        let text = concat!(
            "Status: active\n",
            "Logging: on (low)\n",
            "Default: deny (incoming), allow (outgoing)\n",
            "\n",
            "To                         Action      From\n",
            "--                         ------      ----\n",
            "22/tcp                     ALLOW IN    Anywhere\n",
            "80/tcp                     DENY IN     Anywhere\n"
        );
        let state = read_ufw(&ran(0, text, ""));
        assert_eq!(state.state, FirewallStatus::Active);
        assert_eq!(state.kind.as_deref(), Some("ufw"));
        assert_eq!(state.rule_count, Some(2));
        assert_eq!(state.rules[0].action, "allow");
        assert_eq!(state.rules[1].action, "deny");
    }

    #[test]
    fn ufw_saying_inactive_is_the_one_honest_use_of_the_word_off() {
        let state = read_ufw(&ran(0, "Status: inactive\n", ""));
        assert_eq!(state.state, FirewallStatus::Inactive);
        assert_eq!(state.rule_count, Some(0));
    }

    #[test]
    fn ufw_refusing_an_unprivileged_caller_is_unknown_not_inactive() {
        let exit = ran(1, "", "ERROR: You need to be root to run this script\n");
        let state = read_ufw(&exit);
        assert_eq!(state.state, FirewallStatus::Unknown);
        let reason = state.reason.unwrap();
        assert!(reason.contains("root"), "{reason}");
    }

    #[test]
    fn firewalld_answers_its_state_without_privilege() {
        let running = read_firewalld(&ran(0, "running\n", ""));
        assert_eq!(running.state, FirewallStatus::Active);
        // It said running and nothing about rules; null rather than a number nobody read.
        assert_eq!(running.rule_count, None);
        assert!(running.reason.is_some());

        // firewalld exits 252 when its daemon is down, which is a successful reading.
        let stopped = read_firewalld(&ran(252, "not running\n", ""));
        assert_eq!(stopped.state, FirewallStatus::Inactive);
        assert_eq!(stopped.kind.as_deref(), Some("firewalld"));
    }

    #[test]
    fn a_privilege_refusal_is_recognised_however_the_tool_words_it() {
        assert!(refused_for_privilege("Error: Could not process rule: Operation not permitted"));
        assert!(refused_for_privilege("ERROR: You need to be root to run this script"));
        assert!(refused_for_privilege("Authorization failed: Not authorized"));
        assert!(!refused_for_privilege("Status: inactive"));
    }

    #[test]
    fn a_rule_with_no_verdict_word_is_unknown_rather_than_guessed_at() {
        // A rule whose verdict lives in a chain it jumps to. Labelling it ACCEPT in a security
        // display would be a fabrication one size smaller than the one this replaces.
        let rules = parse_nft_ruleset("table inet f {\n\tchain c {\n\t\ttcp dport 443 counter\n\t}\n}\n");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].action, "unknown");
    }

    #[test]
    fn comments_and_blank_lines_are_not_rules() {
        let text = "# generated\ntable inet f {\n\tchain c {\n\t\ttype filter hook input priority 0; policy accept;\n\n\t}\n}\n";
        assert!(parse_nft_ruleset(text).is_empty());
    }
}

#[cfg(test)]
mod resolvers {
    //! `network.dns_set`, and the choosing that decides which profile it lands on.
    //!
    //! The tool this replaces — the companion's `network_dns_set` — did
    //! `std::fs::write("/etc/resolv.conf", …)` and answered "DNS set to 1.1.1.1". That file is
    //! NetworkManager's: a desktop session cannot write it, and a session that can (the blanket
    //! `NOPASSWD: ALL` both build recipes install) writes something NetworkManager rewrites on
    //! the next carrier change. So the setting either failed or had a half-life, and the sentence
    //! was the same either way.
    //!
    //! Resolvers live on a connection profile. Everything below is about picking the right
    //! profile and reading back what actually happened, on a machine with no NetworkManager on
    //! it.

    use super::nmcli::*;
    use yantrik_ipc_contracts::network::*;

    fn active(rows: &[(&str, &str, &str, &str)]) -> Vec<ActiveConnection> {
        rows.iter()
            .map(|(name, uuid, kind, device)| ActiveConnection {
                name: (*name).to_string(),
                uuid: (*uuid).to_string(),
                kind: (*kind).to_string(),
                device: (*device).to_string(),
            })
            .collect()
    }

    #[test]
    fn the_method_is_a_constant_both_ends_read() {
        assert_eq!(method::DNS_SET, "network.dns_set");
    }

    #[test]
    fn the_request_is_a_list_and_not_a_primary_and_a_secondary() {
        // `{primary, secondary}` could not say "three resolvers", and an empty `secondary` meant
        // both "only one" and "leave the second alone".
        let sent = serde_json::to_value(DnsSetParams {
            servers: vec!["1.1.1.1".into(), "1.0.0.1".into()],
        })
        .unwrap();
        assert_eq!(sent, serde_json::json!({ "servers": ["1.1.1.1", "1.0.0.1"] }));
        let read: DnsSetParams = serde_json::from_value(sent).unwrap();
        assert_eq!(read.servers.len(), 2);
    }

    #[test]
    fn the_result_carries_both_readings_because_they_can_disagree() {
        // On a machine with a stub resolver in front, `/etc/resolv.conf` says 127.0.0.53 and the
        // real servers are only in NetworkManager's view of the device. A check written against
        // resolv.conf alone would call a change that worked a failure.
        let wire = serde_json::json!({
            "connection": "Wired connection 1",
            "device": "enp0s3",
            "resolv_conf": { "nameservers": ["127.0.0.53"], "search_domains": [] },
            "device_dns": ["1.1.1.1", "1.0.0.1"],
        });
        let read: DnsSetResult = serde_json::from_value(wire).unwrap();
        assert_eq!(read.resolv_conf.nameservers, vec!["127.0.0.53"]);
        assert_eq!(read.device_dns, vec!["1.1.1.1", "1.0.0.1"]);
    }

    #[test]
    fn the_profile_carrying_the_default_route_is_the_one_chosen() {
        // A laptop docked: wired and wireless both up, the route on the wire. Setting resolvers
        // on the other one is a change that reports success and resolves nothing.
        let rows = active(&[
            ("lo", "uuid-lo", "loopback", "lo"),
            ("Cafe: Free", "uuid-wifi", "802-11-wireless", "wlp3s0"),
            ("Wired connection 1", "uuid-eth", "802-3-ethernet", "enp0s3"),
        ]);
        let chosen = resolver_connection(&rows, &["enp0s3".to_string()]).unwrap();
        assert_eq!(chosen.uuid, "uuid-eth");
    }

    #[test]
    fn loopback_is_never_chosen_even_when_it_sorts_first() {
        // NetworkManager manages `lo` as a profile of its own on 1.42 and later and nmcli lists
        // it first. Setting resolvers on it changes nothing and looks exactly like success.
        let rows = active(&[
            ("lo", "uuid-lo", "loopback", "lo"),
            ("Wired connection 1", "uuid-eth", "802-3-ethernet", "enp0s3"),
        ]);
        let chosen = resolver_connection(&rows, &[]).unwrap();
        assert_eq!(chosen.uuid, "uuid-eth");
    }

    #[test]
    fn a_profile_with_no_device_is_not_chosen() {
        let rows = active(&[("Stale", "uuid-stale", "802-3-ethernet", "")]);
        assert!(resolver_connection(&rows, &[]).is_none());
    }

    #[test]
    fn a_machine_with_nothing_up_has_no_profile_to_change() {
        assert!(resolver_connection(&[], &[]).is_none());
        assert!(resolver_connection(&active(&[("lo", "u", "loopback", "lo")]), &[]).is_none());
    }

    #[test]
    fn the_active_list_survives_a_connection_named_after_an_ssid_with_a_colon_in_it() {
        // A Wi-Fi profile is named after its SSID, and `Cafe: Free` is an ordinary network name.
        // nmcli escapes the colon; splitting on a bare one truncates the name, and a truncated
        // name is one `nmcli connection modify` cannot find.
        let rows = parse_active("Cafe\\: Free:5f2c-uuid:802-11-wireless:wlp3s0\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Cafe: Free");
        assert_eq!(rows[0].uuid, "5f2c-uuid");
        assert_eq!(rows[0].device, "wlp3s0");
    }

    #[test]
    fn the_uuid_is_what_the_change_is_addressed_to() {
        // Which is why it is parsed at all. `nmcli connection modify "-h"` argues with nmcli's
        // own argument parsing, and an SSID may begin with a dash.
        let rows = parse_active("-h:abc-uuid:802-11-wireless:wlp3s0\n");
        assert_eq!(rows[0].name, "-h");
        assert_eq!(rows[0].uuid, "abc-uuid");
    }

    #[test]
    fn networkmanagers_own_view_of_a_devices_resolvers_is_read_in_its_order() {
        // `nmcli -t -f IP4.DNS,IP6.DNS device show enp0s3`. The keys are indexed, and the order
        // is which resolver is asked first — sorting it would misreport which server answers.
        let text = "IP4.DNS[1]:1.1.1.1\nIP4.DNS[2]:1.0.0.1\n";
        assert_eq!(parse_device_dns(text), vec!["1.1.1.1", "1.0.0.1"]);
    }

    #[test]
    fn an_ipv6_resolver_survives_whether_or_not_nmcli_escaped_its_colons() {
        // `2606:4700:4700::1111` is colons all the way down. nmcli's terse mode escapes them, and
        // this is the escaped form:
        let escaped = parse_device_dns("IP6.DNS[1]:2606\\:4700\\:4700\\:\\:1111\n");
        assert_eq!(escaped, vec!["2606:4700:4700::1111"]);
        // And this is the same line with nothing escaped, which is what a truncating parser turns
        // into `2606` — a resolver this machine cannot ask, reported as the one it was told to
        // use.
        let bare = parse_device_dns("IP6.DNS[1]:2606:4700:4700::1111\n");
        assert_eq!(bare, vec!["2606:4700:4700::1111"]);
    }

    #[test]
    fn a_device_with_no_resolvers_reads_as_none_rather_than_as_a_parse_failure() {
        // nmcli prints the key with nothing after it when there are none.
        assert!(parse_device_dns("IP4.DNS:\nIP6.DNS:\n").is_empty());
        assert!(parse_device_dns("").is_empty());
        // And `--`, which is nmcli's other way of writing "nothing here".
        assert!(parse_device_dns("IP4.DNS[1]:--\n").is_empty());
    }

    #[test]
    fn an_unrelated_field_in_the_same_output_is_not_taken_for_a_resolver() {
        let text = "IP4.ADDRESS[1]:192.168.1.24/24\nIP4.DNS[1]:1.1.1.1\nIP4.GATEWAY:192.168.1.1\n";
        assert_eq!(parse_device_dns(text), vec!["1.1.1.1"]);
    }
}

#[cfg(test)]
mod secrets {
    //! The road a Wi-Fi password travels (#178): a prompt the person types into, then the
    //! service — and never an action argument.
    //!
    //! `wifi_connect` used to declare `password` as an optional parameter. An action's args are
    //! what the approval card draws, what `record_unasked_action` writes into `mind-audit.jsonl`,
    //! what a grant binds to and what the answer echoes, so the secret was handed to every
    //! recording surface in the system before anything connected — and `yos check network`
    //! failed on the parameter's name alone.

    use super::connect::*;
    use yantrik_ipc_contracts::network::{KnownNetwork, WifiConnectParams, WifiState};

    /// The same two lists `deploy/yantrik-os/yos` and
    /// `crates/yantrik-ui/src/control_approvals.rs` carry, spelled out here so this file fails
    /// if the app's published action would fail either of them on a live machine.
    const SECRET_WORDS: [&str; 7] =
        ["passphrase", "password", "passwd", "pin", "secret", "credential", "unlock"];
    const SECRET_PERMITTED: [&str; 1] = ["pinned"];

    fn named_like_a_secret(name: &str) -> bool {
        let lower = name.to_lowercase();
        SECRET_WORDS.iter().any(|w| lower.contains(w)) && !SECRET_PERMITTED.contains(&lower.as_str())
    }

    #[test]
    fn wifi_connect_declares_no_parameter_named_like_a_secret() {
        let action = wifi_connect_action();
        assert_eq!(action.name, "wifi_connect");
        // The only argument is the name. Written out: a `password` slipping back in beside
        // `ssid`, under any of the seven words, is the regression this file exists for.
        let names: Vec<&str> = action.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["ssid"], "{names:?}");
        for p in &action.params {
            assert!(!named_like_a_secret(&p.name), "`{}` is named like a secret", p.name);
        }
        // The JSON Schema a model gets handed says the same thing, and its property keys are
        // exactly what `yos check` reads when it fails a surface on a secret-named parameter.
        let schema = action.schema();
        for key in schema["parameters"]["properties"].as_object().unwrap().keys() {
            assert!(!named_like_a_secret(key), "`{key}` is named like a secret");
        }
        assert_eq!(schema["parameters"]["required"], serde_json::json!(["ssid"]));
        // Still the grade and the settling the connect needs: an association waits on a
        // handshake and on DHCP, and joining a network is a deliberate decision.
        assert_eq!(schema["permission"], serde_json::json!("sensitive"));
        assert_eq!(schema["settles"], serde_json::json!("later"));
    }

    #[test]
    fn the_card_the_audit_line_and_both_answers_of_a_connect_carry_no_password() {
        let secret = "hunter2-the-real-one";
        let action = wifi_connect_action();
        // A connect call, shaped the way the shell records one: the published schema beside
        // the args of the call, which is what an approval card draws and what
        // `record_unasked_action` writes into `mind-audit.jsonl`.
        let record = serde_json::json!({
            "app": "network",
            "action": action.schema(),
            "args": { "ssid": "Cafe: Free" },
            "answer_joining": joining_answer("Cafe: Free"),
            "answer_waiting": waiting_answer("Cafe: Free"),
        })
        .to_string();
        assert!(!record.contains(secret), "{record}");
        assert!(!record.contains("hunter2"), "{record}");
        // Neither answer the action can give even has a field a secret could sit in: one names
        // the join, the other names the wait and the prompt. Both must stay readable in a log.
        for answer in [joining_answer("Cafe: Free"), waiting_answer("Cafe: Free")] {
            assert_eq!(answer.get("password"), None, "{answer}");
            assert_eq!(answer.get("secret"), None, "{answer}");
        }
        assert_eq!(waiting_answer("Cafe: Free")["waiting_on_person"], serde_json::json!("Cafe: Free"));
    }

    #[test]
    fn what_the_person_types_reaches_the_backend_and_nothing_readable_carries_it() {
        use std::sync::Mutex;

        struct Recorder(Mutex<Vec<WifiConnectParams>>);

        impl Backend for Recorder {
            fn wifi_connect(&self, request: &WifiConnectParams) -> Result<WifiState, String> {
                self.0.lock().unwrap().push(request.clone());
                Ok(WifiState {
                    connected_ssid: Some(request.ssid.clone()),
                    ..WifiState::default()
                })
            }
        }

        let recorder = Recorder(Mutex::new(Vec::new()));
        // The dialog's Connect button and the action's saved-network path both hand `submit`
        // what the prompt holds; this is the typed-password one.
        let state = submit(&recorder, " Cafe: Free ", "hunter2-the-real-one").unwrap();
        let seen = recorder.0.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].ssid, "Cafe: Free");
        assert_eq!(
            seen[0].password.as_deref(),
            Some("hunter2-the-real-one"),
            "the typed secret reaches the backend in the one field built for it"
        );
        // The caller reads back the join, not the secret — and a trace printing the request or
        // the state finds nothing either, which is what `WifiConnectParams`' hand-written
        // `Debug` is for.
        assert_eq!(state.connected_ssid.as_deref(), Some("Cafe: Free"));
        let printed = format!("{state:?} {:?}", seen[0]);
        assert!(!printed.contains("hunter2"), "{printed}");
        drop(seen);

        // An empty box is not a password: the open-network path sends no secret field at all,
        // so nothing downstream can mistake "" for one.
        submit(&recorder, "OpenCafe", "").unwrap();
        let seen = recorder.0.lock().unwrap();
        assert_eq!(seen[1].password, None);
        let wire = serde_json::to_string(&seen[1]).unwrap();
        assert!(!wire.contains("password"), "{wire}");
    }

    #[test]
    fn a_saved_network_joins_and_any_other_asks_the_person() {
        // The rule the person's own click follows in `network_manager.slint`: a saved row
        // joins outright, any other raises the password dialog. The action takes the same
        // branch, so the mind cannot skip a prompt the person would have seen.
        let known = vec![KnownNetwork {
            ssid: "HomeNet".into(),
            uuid: "11111111-1111-1111-1111-111111111111".into(),
            is_active: false,
        }];
        assert_eq!(plan("HomeNet", &known), Plan::Join);
        assert_eq!(plan("Cafe: Free", &known), Plan::AskPerson);
        // A saved name is matched whole: a prefix of one is a different network.
        assert_eq!(plan("Home", &known), Plan::AskPerson);
        assert_eq!(plan("HomeNet2", &known), Plan::AskPerson);
        // A machine with nothing saved asks, whatever the name.
        assert_eq!(plan("HomeNet", &[]), Plan::AskPerson);
    }
}
