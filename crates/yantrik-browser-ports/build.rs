//! Turns `browsers.conf` (in this crate) into constants. The Python browser service reads the same
//! file at run time (a release copies it beside the service); nothing else may spell these ports
//! or paths.

use std::{collections::BTreeMap, env, fs, path::PathBuf};

const CONF: &str = "browsers.conf";

fn main() {
    println!("cargo:rerun-if-changed={CONF}");
    let text = fs::read_to_string(CONF).unwrap_or_else(|e| panic!("cannot read {CONF}: {e}"));
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').unwrap_or_else(|| panic!("{CONF}: not `key = value`: {line}"));
        keys.insert(k.trim().to_string(), v.trim().to_string());
    }
    let get = |k: &str| keys.get(k).unwrap_or_else(|| panic!("{CONF}: no `{k}`")).clone();
    let port = |k: &str| get(k).parse::<u16>().unwrap_or_else(|_| panic!("{CONF}: `{k}` is not a port"));
    let (person, mind) = (port("person_port"), port("mind_view_port"));
    let (first, last) = (port("identity_port_first"), port("identity_port_last"));
    assert!(person != mind && first > mind && last >= first, "{CONF}: the ports overlap");
    let out = format!(
        "pub const PERSON_PORT: u16 = {person};\npub const MIND_VIEW_PORT: u16 = {mind};\n\
         pub const IDENTITY_PORT_FIRST: u16 = {first};\npub const IDENTITY_PORT_LAST: u16 = {last};\n\
         pub const MIND_VIEW_PROFILE: &str = {:?};\npub const IDENTITY_PROFILES: &str = {:?};\n",
        get("mind_view_profile"),
        get("identity_profiles")
    );
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("ports.rs");
    fs::write(dest, out).unwrap();
}
