//! What Images is: the model every command changes and every render reads, and the commands.

use std::path::PathBuf;
use std::sync::Arc;

use yantrik_app::mind::{AskError, Mind};
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_image_core::{Gallery, ImageInfo};

use crate::orient::Orientation;
use crate::open::Loaded;
use crate::picture::{Picture, Pixels};

/// The parts of the model `render` redraws by name.
///
/// Every command names what it changed, and none redraws everything: two of these parts act
/// rather than draw (`PART_FIT` takes the window out of a zoom, `PART_RAISE` brings it
/// forward), so a redraw of every part would be a press nobody made. `PART_NOTHING` is what a
/// command that changed nothing on screen says.
pub const PART_NOTHING: &str = "nothing";
/// The picture's pixels.
pub const PART_PICTURE: &str = "picture";
/// The name, the counter, the notice and what the file says.
pub const PART_SHOWN: &str = "shown";
/// How the picture lies: the toolbar's lit buttons and the corner badge.
pub const PART_ORIENTATION: &str = "orientation";
/// The info panel, as the app was asked to set it.
pub const PART_VIEW: &str = "view";
/// Fit or fill, and out of a zoom of the person's own.
pub const PART_FIT: &str = "fit";
/// The Describe panel.
pub const PART_AI: &str = "ai";
/// The window brought forward.
pub const PART_RAISE: &str = "raise";

/// Why a view control is off: nothing is on screen to zoom, turn, describe or read the facts of.
/// The window's buttons say it on hover (`no-picture` in image_viewer.slint) and the control
/// surface refuses with it, so a person and a mind are told the same thing.
pub const NO_PICTURE: &str = "Open a picture first";

/// What the window says when it was given no picture.
pub const NOTHING_OPEN: &str = "Open a picture from Files to see it here.";

/// The picture on screen, as its last load finished: the screen changes all at once when a
/// load lands, never half-way through one.
#[derive(Debug, Clone)]
pub struct Shown {
    /// The file, if one was chosen.
    pub path: Option<PathBuf>,
    /// Its name, as the header and `describe` say it.
    pub name: String,
    /// "3 / 12", from the folder as it was when the picture landed.
    pub counter: String,
    /// Why there is no picture, when there is none.
    pub notice: String,
    pub info: ImageInfo,
    /// As loaded; `None` when nothing is open or the file would not load.
    pub picture: Option<Picture>,
    /// How the view has turned it. The file is never touched.
    pub orientation: Orientation,
    /// The last turned pixels that came back, and the orientation they were turned to. Drawn
    /// while the view is turned; a newer turn replaces them when it lands.
    pub turned: Option<(Orientation, Pixels)>,
}

impl Default for Shown {
    fn default() -> Shown {
        Shown {
            path: None,
            name: String::new(),
            counter: String::new(),
            notice: NOTHING_OPEN.into(),
            info: ImageInfo::default(),
            picture: None,
            orientation: Orientation::default(),
            turned: None,
        }
    }
}

impl Shown {
    /// A picture is on screen: nothing opened and a file that would not load both leave none.
    pub fn has_picture(&self) -> bool {
        self.picture.is_some()
    }
}

/// How the person is looking at the picture: kept by the screen itself, told to the app
/// whenever it changes (`view-changed` in ui/app.slint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewState {
    /// The whole picture in the window (contain), or the window filled (cover).
    pub fit_contain: bool,
    /// Zoomed or panned to a size of the person's own.
    pub zoomed: bool,
    pub info_open: bool,
    pub slideshow_active: bool,
    pub slideshow_paused: bool,
}

impl Default for ViewState {
    fn default() -> ViewState {
        ViewState {
            fit_contain: true,
            zoomed: false,
            info_open: false,
            slideshow_active: false,
            slideshow_paused: false,
        }
    }
}

/// The Describe panel: the one ask of the companion this window makes.
#[derive(Debug, Clone, Default)]
pub struct Ai {
    pub working: bool,
    pub response: String,
}

/// Whether an agent may be shown a picture: the verdict of `home_paths::may_read_file`, read
/// off the UI thread when the picture was loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub path: PathBuf,
    pub shown_to_agents: Result<(), String>,
}

/// What `show`, `next` and `previous` answered, kept for the reply: the command that ran under
/// the call knows whether an agent asked, and the reply is read from the model right after it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    pub showing: String,
    pub position: Option<usize>,
    /// The picture is one this caller is not shown.
    pub hidden: bool,
}

pub struct Model {
    /// The pictures in the folder of the one opened, and which of them is current. Moves at
    /// once; the screen follows when the current one has loaded.
    pub gallery: Gallery,
    pub shown: Shown,
    /// Every load is numbered; only the newest one's result is shown, so a slow large picture
    /// cannot land over a newer one.
    pub load: u64,
    /// Every picture put on screen is numbered; a turn made for an earlier one is not drawn.
    pub screen: u64,
    /// Whether an agent may be shown the gallery's current picture; `None` while it loads.
    pub verdict: Option<Verdict>,
    pub view: ViewState,
    pub ai: Ai,
    pub answer: Answer,
    /// How many times the window has been asked forward.
    pub raises: u64,
    /// The first launch has been applied; a later one was handed over by a second copy.
    pub launched: bool,
    pub rail: Rail,
    pub mind: Arc<dyn Mind>,
}

impl Model {
    pub fn new(mind: Arc<dyn Mind>) -> Model {
        Model {
            gallery: Gallery::empty(),
            shown: Shown::default(),
            load: 0,
            screen: 0,
            verdict: None,
            view: ViewState::default(),
            ai: Ai::default(),
            answer: Answer::default(),
            raises: 0,
            launched: false,
            rail: Rail::new(mind.clone()),
            mind,
        }
    }
}

/// One of the toolbar's four turn buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
}

impl Turn {
    pub fn change(self) -> fn(Orientation) -> Orientation {
        match self {
            Turn::RotateLeft => Orientation::rotate_left,
            Turn::RotateRight => Orientation::rotate_right,
            Turn::FlipHorizontal => Orientation::flip_horizontal,
            Turn::FlipVertical => Orientation::flip_vertical,
        }
    }
}

/// What the `rotate` and `flip` actions name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnVerb {
    Rotate,
    Flip,
}

/// How a picture came to be opened, which decides what a missing file means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAs {
    /// The command line the app was started with: a missing file is still named on screen.
    FirstLaunch,
    /// The `open` action, or a launch handed over by a second copy: a missing file is refused
    /// by name and the window is left as it was.
    Asked,
}

#[derive(Debug, Clone)]
pub enum Command {
    /// The next picture in the folder, wrapping at the end.
    Next,
    /// The previous picture, wrapping at the start.
    Previous,
    /// Open this file, and the folder it is in.
    Open { path: PathBuf, how: OpenAs },
    /// What a launch asked to open (`Event::Launch`): the first one's, or one handed over.
    Launch(Vec<PathBuf>),
    /// An open finished: the folder and the picture, or why there are none.
    Opened {
        load: u64,
        outcome: Result<(Gallery, Loaded), String>,
    },
    /// A picture of the folder finished loading.
    Loaded { load: u64, loaded: Loaded },
    /// A toolbar button turned or mirrored the picture.
    Turn(Turn),
    /// The `rotate` or `flip` action, with the direction as it was given.
    TurnNamed { verb: TurnVerb, direction: String },
    /// Turned pixels came back.
    Turned {
        screen: u64,
        orientation: Orientation,
        /// `None`: the picture's pixels could not be read.
        pixels: Option<Pixels>,
    },
    /// The `fit` action: the fit button's own step.
    Fit,
    /// The `toggle_info` action.
    ToggleInfo,
    /// The window's view state, as it shows it now.
    ViewSeen(ViewState),
    /// The Describe button.
    AskDescribe,
    /// The companion answered, about the picture of this name.
    Described {
        name: String,
        outcome: Result<String, AskError>,
    },
    /// The Describe panel closed.
    DismissAi,
    /// The `show` action: the window forward.
    Show,
    Rail(RailCommand),
}
