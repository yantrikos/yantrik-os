//! Every change to the model is one of these, wherever it came from: a button, the Mind, or what
//! the library's lane or a browser worker found.

use serde_json::Value;
use yantrik_app::app::WindowId;

use crate::library::{CharacterEntry, GameEntry};

/// The nine commands, by the name the surface knows them by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    NewCharacter,
    NewGame,
    UpdateGame,
    UpdateCharacter,
    Build,
    Play,
    Verify,
    Screenshot,
    Delete,
}

impl Verb {
    pub fn name(self) -> &'static str {
        match self {
            Verb::NewCharacter => "new_character",
            Verb::NewGame => "new_game",
            Verb::UpdateGame => "update_game",
            Verb::UpdateCharacter => "update_character",
            Verb::Build => "build",
            Verb::Play => "play",
            Verb::Verify => "verify",
            Verb::Screenshot => "screenshot",
            Verb::Delete => "delete",
        }
    }

    /// The header's and the buttons' ids: the surface names, dashed where the surface underscores.
    pub fn of_button(id: &str) -> Option<Verb> {
        Some(match id {
            "new-character" => Verb::NewCharacter,
            "new-game" => Verb::NewGame,
            "build" => Verb::Build,
            "play" => Verb::Play,
            "verify" => Verb::Verify,
            "screenshot" => Verb::Screenshot,
            "delete" => Verb::Delete,
            _ => return None,
        })
    }

    /// The three that reach a browser: seconds to minutes, so they run on a worker of their own.
    pub fn is_slow(self) -> bool {
        matches!(self, Verb::Play | Verb::Verify | Verb::Screenshot)
    }

    /// The ones that carry a spec the person may have typed.
    pub fn takes_spec(self) -> bool {
        matches!(self, Verb::NewCharacter | Verb::NewGame | Verb::UpdateGame | Verb::UpdateCharacter)
    }
}

/// One command, with the arguments it was given. Empty means not given.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub verb: Verb,
    pub spec: String,
    pub game: String,
}

impl Request {
    /// The command as its arguments name it, in the surface's spelling (`spec`, `game`); the
    /// buttons build the same object out of the window's fields. A value that is missing or only
    /// spaces is refused with the sentence the surface has always given.
    pub fn from_args(verb: Verb, args: &Value) -> Result<Request, String> {
        let text = |name: &str| args.get(name).and_then(|v| v.as_str()).map(str::trim).unwrap_or_default().to_string();
        let required = |name: &str, what: &str| {
            let value = text(name);
            if value.is_empty() {
                Err(format!("`{name}` is required: {what}"))
            } else {
                Ok(value)
            }
        };
        const GAME: &str = "the title or slug of the game, as `describe` lists under `games`";
        let none = String::new();
        Ok(match verb {
            Verb::NewCharacter => Request { verb, spec: required("spec", "the character JSON to save")?, game: none },
            Verb::NewGame => Request { verb, spec: required("spec", "the game JSON to save")?, game: none },
            Verb::UpdateGame => {
                Request { verb, spec: required("spec", "the game JSON that replaces the saved one")?, game: text("game") }
            }
            Verb::UpdateCharacter => {
                Request { verb, spec: required("spec", "the character JSON that replaces the saved one")?, game: none }
            }
            Verb::Build | Verb::Play | Verb::Verify | Verb::Screenshot | Verb::Delete => {
                Request { verb, spec: none, game: required("game", GAME)? }
            }
        })
    }
}

/// The library as the lists show it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Listing {
    pub games: Vec<GameEntry>,
    pub characters: Vec<CharacterEntry>,
}

/// A browser job a command has started.
#[derive(Debug, Clone)]
pub struct Browse {
    pub verb: Verb,
    pub slug: String,
    pub html: std::path::PathBuf,
}

/// What a command on the lane came to.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A command answered: the answer, the game it leaves selected, and the browser job it starts.
    Done { value: Value, select: Option<String>, slow: Option<Browse> },
    /// A character's spec, for the editor.
    Spec(String),
    /// The library read again.
    Reloaded,
}

/// A command on the lane has landed.
#[derive(Debug, Clone)]
pub struct Landing {
    /// The order it was asked in: an older listing never replaces a newer one.
    pub seq: u64,
    /// The command, if it was one; the text it carried when it came from the editor.
    pub request: Option<Request>,
    pub from_editor: bool,
    pub result: Result<Reply, String>,
    pub listing: Listing,
}

#[derive(Debug, Clone)]
pub enum Command {
    // ── The window ──
    /// A header or workbench button, by its id.
    Button(String),
    SpecEdited(String),
    GameEdited(String),
    PickGame(String),
    PickCharacter(String),

    // ── The surface ──
    Run(Request),

    // ── What the lane and the workers found ──
    Landed(Box<Landing>),
    /// A command on the lane ended without its answer: a panic, or the pool at its bound.
    /// `slow` says whether it was one of the three that start a browser job.
    LaneFailed { slow: bool },
    /// A browser job ended: its verdict in a sentence, or why it failed.
    Verdict { job: u64, result: Result<String, String> },
    /// A browser job ended without a verdict: a panic.
    VerdictLost { job: u64 },

    // ── The window coming forward ──
    Reload,

    /// The editor's text did not reach the draft lane: said, so the person knows it is only here.
    DraftNotKept { why: String },

    // ── The app's life ──
    /// The person closed the window while a command was on the lane: it closes, and the app runs
    /// on without it until the lane is empty (`fx.linger`).
    Closing(WindowId),
    /// A second launch opened the window again while that close waited.
    Reopened,
    /// The process is asked to end: a logout, a signal, the display going, or a person's quit.
    Ending,
}
