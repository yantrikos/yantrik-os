//! Which files an agent may have yPresent read and write, and which deck it is not shown (#443).
//!
//! yPresent runs as the person. `open` for an agent parsed any file the person can read, and
//! `save_as` and `export_markdown` wrote any file the person can write, `~/.bashrc` and Yantrik's
//! own data folders included (security review of #942, finding 1). An agent now names only files
//! in the person's home outside its protected places, by the rule every side shares
//! (`yantrik_ipc_contracts::home_paths`), the same rule the Text Editor and yDoc keep. The rule
//! itself is the framework's (`yantrik_app::documents::files::agent`).
//!
//! Each rule looks at the disk (whether the path is there, a file, a link), so each runs in the
//! job that opens or saves, never on the window's thread. Whether the deck in front is one an
//! agent is not shown is decided when the deck takes its file and is kept with it
//! (`Doc::hidden`); the model's own refusals read that (`update::surface::not_hidden`). The
//! person, choosing a file in the window or calling with no token, is not asked.

pub use yantrik_app::documents::files::agent::*;

/// How a deck an agent is not shown is named to it, in place of its title and file.
pub const HIDDEN: &str = "(hidden)";

/// Why an agent's action on the deck in front is refused when that deck is hidden.
pub const HIDDEN_REFUSAL: &str = "The deck in front is a file an agent is not shown, and it is left \
                                  alone; `open` another or start a `new_deck`.";
