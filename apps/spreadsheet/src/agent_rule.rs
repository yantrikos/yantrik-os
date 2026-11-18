//! Which files an agent may have ySheets read and write, and which sheet it is not shown (#443).
//!
//! The sheet runs as the person, and `get_range` hands back what a file holds. So `open` for an
//! agent is limited to files in the person's home outside its protected places, and `save` and
//! `save_over` to the same: `~/.bashrc` is code run as the person at their next login, and it is
//! on the protected list. The rule is the one every side shares
//! (`yantrik_ipc_contracts::home_paths`), and it is the framework's
//! (`yantrik_app::documents::files::agent`). The person, choosing a file in the window or calling
//! with no token, is not asked.
//!
//! Each rule looks at the disk (whether the path is there, a file, a link), so each runs in the
//! job that opens or saves, never on the window's thread. Whether the sheet in front is one an
//! agent is not shown is decided when the sheet takes its file and is kept with it
//! (`Doc::hidden`); the model's own refusals read that (`update::cells::not_hidden`).

pub use yantrik_app::documents::files::agent::*;

/// How a sheet an agent is not shown is named to it, in place of its path.
pub const HIDDEN: &str = "(hidden)";

/// Why an agent's action on the sheet in front is refused when that sheet is hidden. It names no
/// path: the notice a refusal leaves is in `describe` too.
pub const HIDDEN_REFUSAL: &str =
    "This sheet is not read or saved for an agent: the file is one an agent is not shown.";
