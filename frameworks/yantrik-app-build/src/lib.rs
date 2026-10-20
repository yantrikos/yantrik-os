//! How a Yantrik app's UI is compiled (docs/architecture.md, migration step 2).
//!
//! Every app's `build.rs` was the same twenty lines: read the design tokens' and the kit's folders
//! from the `links` variables, add the shell's folder, set the style, compile. Now it is
//!
//! ```ignore
//! fn main() {
//!     yantrik_app_build::UiBuild::new().compile("ui/app.slint");
//! }
//! ```
//!
//! The app's crate must depend on `yantrik-design-tokens` and `yantrik-ui-kit` (normal
//! dependencies), whose build scripts publish their Slint folders to it. Both are on the import
//! path, and both are libraries a `.slint` file can name wherever it lives:
//!
//! ```slint
//! import { Theme } from "@yantrik-design/theme.slint";
//! import { YButton } from "@yantrik-kit/y_button.slint";
//! ```

#![warn(unreachable_pub)]

use std::collections::HashMap;
use std::path::PathBuf;

/// The style every Yantrik window is drawn in.
pub const STYLE: &str = "fluent-dark";

/// A UI compile for an app (or the shell).
#[derive(Debug)]
pub struct UiBuild {
    design: PathBuf,
    kit: PathBuf,
    include: Vec<PathBuf>,
}

impl UiBuild {
    /// The design tokens and the kit, from the folders their build scripts published.
    ///
    /// Panics with the reason when the app does not depend on them: a build script has no other
    /// way to say so.
    pub fn new() -> UiBuild {
        UiBuild {
            design: published("DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH", "yantrik-design-tokens"),
            kit: published("DEP_YANTRIK_UI_KIT_SLINT_PATH", "yantrik-ui-kit"),
            include: Vec::new(),
        }
    }

    /// One more folder on the import path, relative to the app's own folder.
    ///
    /// For an app whose screen still lives in the shell's folder, until it moves into the app
    /// (migration step 3). The path is written in the app's `build.rs`, where the architecture
    /// check sees it.
    pub fn include_dir(mut self, relative: &str) -> UiBuild {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
        self.include.push(PathBuf::from(manifest).join(relative));
        self
    }

    /// Compiles `entry` (relative to the app's folder), and panics with Slint's diagnostics if it
    /// does not compile.
    pub fn compile(self, entry: &str) {
        let mut include = vec![self.design.clone(), self.kit.clone()];
        include.extend(self.include);
        let libraries = HashMap::from([
            ("yantrik-design".to_string(), self.design),
            ("yantrik-kit".to_string(), self.kit),
        ]);
        let config = slint_build::CompilerConfiguration::new()
            .with_style(STYLE.into())
            .with_include_paths(include)
            .with_library_paths(libraries);
        if let Err(e) = slint_build::compile_with_config(entry, config) {
            panic!("{entry} did not compile: {e}");
        }
    }
}

impl Default for UiBuild {
    fn default() -> UiBuild {
        UiBuild::new()
    }
}

fn published(var: &str, crate_name: &str) -> PathBuf {
    std::env::var(var)
        .map(PathBuf::from)
        .unwrap_or_else(|_| panic!("{crate_name} must be a dependency of this crate ({var} is not set)"))
}
