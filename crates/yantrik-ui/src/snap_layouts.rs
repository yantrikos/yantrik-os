//! The snap layouts are three files that must agree: the regions `config/labwc/rc.xml` defines,
//! the keys it binds to them, and the rows of the window menu (`config/labwc/menu.xml`) that name
//! both. labwc says nothing when a key or a row names a region that is not there — the key simply
//! does nothing — so these tests are where a typo is caught.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    fn read(rel: &str) -> String {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    /// Every value of `attr="…"` on elements whose text starts with `<tag`.
    fn attrs(xml: &str, tag: &str, attr: &str) -> Vec<String> {
        let mut out = Vec::new();
        for piece in xml.split(&format!("<{tag}")).skip(1) {
            let head = piece.split('>').next().unwrap_or_default();
            let key = format!("{attr}=\"");
            if let Some(i) = head.find(&key) {
                let rest = &head[i + key.len()..];
                out.push(rest[..rest.find('"').unwrap_or(0)].to_string());
            }
        }
        out
    }

    fn regions(rc: &str) -> BTreeMap<String, [u32; 4]> {
        // The element, not a comment that names it: the last one in the file.
        let block = rc.rsplit("<regions>").next().and_then(|b| b.split("</regions>").next()).expect("rc.xml has <regions>");
        let mut out = BTreeMap::new();
        for piece in block.split("<region ").skip(1) {
            let head = piece.split("/>").next().unwrap();
            let get = |a: &str| -> String {
                let k = format!("{a}=\"");
                let r = &head[head.find(&k).unwrap_or_else(|| panic!("a region without {a}: {head}")) + k.len()..];
                r[..r.find('"').unwrap()].to_string()
            };
            let pct = |a: &str| -> u32 {
                let v = get(a);
                v.strip_suffix('%').unwrap_or_else(|| panic!("labwc needs the % on {a}={v}")).parse().unwrap()
            };
            let name = get("name");
            assert!(out.insert(name.clone(), [pct("x"), pct("y"), pct("width"), pct("height")]).is_none(), "{name} twice");
        }
        out
    }

    /// `(key, region)` for every keybind whose action is SnapToRegion.
    fn snap_keys(rc: &str) -> Vec<(String, String)> {
        rc.split("<keybind key=\"")
            .skip(1)
            .filter_map(|p| {
                let key = p[..p.find('"')?].to_string();
                let body = p.split("</keybind>").next()?;
                let region = attrs(body, "action name=\"SnapToRegion\"", "region").into_iter().next()?;
                Some((key, region))
            })
            .collect()
    }

    #[test]
    fn every_key_and_every_menu_row_names_a_region_that_exists() {
        let rc = read("config/labwc/rc.xml");
        let menu = read("config/labwc/menu.xml");
        let defined = regions(&rc);
        for (file, xml) in [("rc.xml", &rc), ("menu.xml", &menu)] {
            let named = attrs(xml, "action name=\"SnapToRegion\"", "region");
            assert!(!named.is_empty(), "{file} snaps to nothing");
            for r in named {
                assert!(defined.contains_key(&r), "{file} snaps to `{r}`, which rc.xml's <regions> does not define");
            }
        }
    }

    #[test]
    fn every_region_is_on_the_screen_and_every_layout_has_a_key() {
        let rc = read("config/labwc/rc.xml");
        let bound: BTreeSet<String> = snap_keys(&rc).into_iter().map(|(_, r)| r).collect();
        for (name, [x, y, w, h]) in regions(&rc) {
            assert!(w > 0 && h > 0 && x + w <= 100 && y + h <= 100, "{name} runs off the screen");
            assert!(bound.contains(&name), "{name} is a layout no key reaches");
        }
    }

    #[test]
    fn no_key_is_bound_twice() {
        let rc = read("config/labwc/rc.xml");
        let mut seen = BTreeSet::new();
        for p in rc.split("<keybind key=\"").skip(1) {
            let key = &p[..p.find('"').unwrap()];
            assert!(seen.insert(key.to_string()), "{key} is bound twice in rc.xml");
        }
    }

    /// Each menu row says its key in brackets; the key it says must be the one rc.xml binds to
    /// the row's region, so the menu never teaches a shortcut that does something else.
    #[test]
    fn each_menu_row_says_the_key_that_really_snaps_there() {
        let rc = read("config/labwc/rc.xml");
        let menu = read("config/labwc/menu.xml");
        let key_of: BTreeMap<String, String> = snap_keys(&rc).into_iter().map(|(k, r)| (r, k)).collect();
        let spoken = |key: &str| -> String {
            let k = key.strip_prefix("W-A-").expect("the snap keys are Super+Alt");
            let k = match k {
                "Left" => "←".to_string(),
                "Right" => "→".to_string(),
                "Up" => "↑".to_string(),
                "Down" => "↓".to_string(),
                "Return" => "Enter".to_string(),
                one => one.to_uppercase(),
            };
            format!("(Super+Alt+{k})")
        };
        let mut checked = 0;
        for item in menu.split("<item label=\"").skip(1) {
            let label = &item[..item.find('"').unwrap()];
            let body = item.split("</item>").next().unwrap();
            if let Some(region) = attrs(body, "action name=\"SnapToRegion\"", "region").into_iter().next() {
                let key = key_of.get(&region).unwrap_or_else(|| panic!("no key snaps to {region}"));
                assert!(label.ends_with(&spoken(key)), "the row `{label}` should end {}", spoken(key));
                checked += 1;
            }
        }
        assert_eq!(checked, regions(&rc).len(), "every layout has its row in the Snap menu");
    }
}
