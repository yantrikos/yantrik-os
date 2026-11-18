//! The layers of docs/architecture.md and where each crate sits in them (`layers.txt`).

use std::fmt;

/// A crate's place. Dependencies point down: L0 at the bottom, the minds at the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// Foundation: types and rules with no UI and no service behind them.
    L0,
    /// Design: the tokens and the kit.
    L1,
    /// Platform: what an app or a service is built on.
    L2,
    /// OS services.
    L3,
    /// The shell and its separate windows.
    L4,
    /// The minds.
    L5,
    /// An app, or what an app author copies: L0-L2 only, as a third-party app would be.
    App,
    /// Not product code (test crates, these checks): placed, but not held to the layers.
    Tool,
}

impl Layer {
    fn parse(word: &str) -> Option<Layer> {
        Some(match word {
            "L0" => Layer::L0,
            "L1" => Layer::L1,
            "L2" => Layer::L2,
            "L3" => Layer::L3,
            "L4" => Layer::L4,
            "L5" => Layer::L5,
            "app" => Layer::App,
            "tool" => Layer::Tool,
            _ => return None,
        })
    }

    /// Whether a crate here may depend on a crate at `to`.
    pub fn may_use(self, to: Layer) -> bool {
        match (self, to) {
            (Layer::Tool, _) => true,
            // Nothing product-side links a test crate or an app.
            (_, Layer::Tool) | (_, Layer::App) => false,
            (Layer::App, to) => to <= Layer::L2,
            (from, to) => to <= from,
        }
    }

    /// Whether this crate's build may only reach other crates through their libraries (an app's
    /// or a framework's), never by a relative path into their folders.
    pub fn builds_from_libraries_only(self) -> bool {
        matches!(self, Layer::L0 | Layer::L1 | Layer::L2 | Layer::App)
    }
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = match self {
            Layer::L0 => "L0",
            Layer::L1 => "L1",
            Layer::L2 => "L2",
            Layer::L3 => "L3",
            Layer::L4 => "L4",
            Layer::L5 => "L5",
            Layer::App => "app",
            Layer::Tool => "tool",
        };
        f.write_str(word)
    }
}

/// `layers.txt`: each line a layer and a member's folder, or `dir/*` for every member under dir.
#[derive(Debug)]
pub struct Placements(Vec<(Layer, String)>);

impl Placements {
    pub fn parse(text: &str) -> Result<Placements, String> {
        let mut out = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut words = line.split_whitespace();
            let (Some(layer), Some(path), None) = (words.next(), words.next(), words.next()) else {
                return Err(format!(
                    "layers.txt line {}: expected `<layer> <path>`, got `{line}`",
                    n + 1
                ));
            };
            let layer = Layer::parse(layer)
                .ok_or_else(|| format!("layers.txt line {}: no layer `{layer}`", n + 1))?;
            out.push((layer, path.trim_end_matches('/').to_string()));
        }
        Ok(Placements(out))
    }

    /// The layer of the member in folder `dir` (relative to the workspace root, `/`-separated).
    /// An exact entry wins over a `dir/*` one, so one member of a folder can be placed apart.
    pub fn place(&self, dir: &str) -> Option<Layer> {
        let exact = self.0.iter().find(|(_, p)| p == dir);
        let under = || {
            self.0.iter().find(|(_, p)| {
                p.strip_suffix("/*").is_some_and(|parent| {
                    dir.strip_prefix(parent)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
            })
        };
        exact.or_else(under).map(|(layer, _)| *layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependencies_point_down() {
        assert!(Layer::L4.may_use(Layer::L1));
        assert!(Layer::L3.may_use(Layer::L3));
        assert!(
            !Layer::L4.may_use(Layer::L5),
            "the shell does not link a mind"
        );
        assert!(!Layer::L0.may_use(Layer::L1));
        assert!(Layer::App.may_use(Layer::L2));
        assert!(
            !Layer::App.may_use(Layer::L3),
            "an app uses the platform, not the OS's insides"
        );
        assert!(!Layer::App.may_use(Layer::App), "nor another app");
        assert!(!Layer::L4.may_use(Layer::Tool));
        assert!(Layer::Tool.may_use(Layer::L5));
    }

    #[test]
    fn a_folder_is_placed_by_its_entry() {
        let p = Placements::parse(
            "# note\nL3 services/*\nL0 crates/a  # why\napp apps/*\nL4 apps/special\n",
        )
        .unwrap();
        assert_eq!(p.place("services/weather-service"), Some(Layer::L3));
        assert_eq!(p.place("crates/a"), Some(Layer::L0));
        assert_eq!(p.place("apps/notes"), Some(Layer::App));
        assert_eq!(
            p.place("apps/special"),
            Some(Layer::L4),
            "an exact entry wins"
        );
        assert_eq!(
            p.place("crates/b"),
            None,
            "an unplaced crate is not guessed"
        );
        assert_eq!(p.place("servicesx/y"), None);
        assert!(Placements::parse("L9 crates/a").is_err());
        assert!(Placements::parse("L0").is_err());
    }
}
