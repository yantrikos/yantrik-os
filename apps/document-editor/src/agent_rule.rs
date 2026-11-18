//! Which files an agent may have yDoc read and write, and which document it is not shown (#443).
//!
//! yDoc runs as the person, and `describe documents` hands back the first 4000 characters of the
//! document. So `open` for an agent read any file the person can read - the snippets store,
//! `~/.ssh/id_ed25519` - and `describe` gave it back; `save_as` and `export_markdown` wrote any
//! file the person can write (security review of #942, finding 1). An agent now names only files
//! in the person's home outside its protected places, by the rule every side shares
//! (`yantrik_ipc_contracts::home_paths`), the same rule the Text Editor keeps.
//!
//! Each rule looks at the disk (whether the path is there, a file, a link), so each runs in the
//! job that opens or saves, never on the window's thread. Whether the document in front is one
//! an agent is not shown is decided when the document takes its file and is kept with it
//! (`Model::hidden`); the model's own refusals read that (`update::surface::not_hidden`). The
//! person, choosing a file in the window or calling with no token, is not asked.

pub use yantrik_app::documents::files::agent::*;

/// How a document an agent is not shown is named to it, in place of its title and path.
pub const HIDDEN: &str = "(hidden)";

/// Why an agent's action on the document in front is refused when that document is hidden.
pub const HIDDEN_REFUSAL: &str = "The document in front is a file an agent is not shown, and its \
                                  text is left alone; `open` another or start a `new` one.";
