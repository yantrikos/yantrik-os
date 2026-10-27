//! One instance, a window per display (#963): a Mind's windows and the person's are separate
//! windows of the one process that owns the app's data.
//!
//! # Why one instance
//!
//! The instance lock (#911) keeps two processes from ever writing one Notes vault, Email store or
//! document at once. A Mind's copy of the app would be a second writer, and a separate copy of the
//! data would break the Mind's look after the act: it writes through an intent, looks at its own
//! window, and must see what it wrote. So the one instance keeps its lock, its socket, its store
//! and its model, and every window, the person's and a Mind's, draws from that one model.
//!
//! # Whose window
//!
//! Each window records whose it is when it opens ([`Owner`]), from who asked for it
//! ([`asker`]):
//! - a launch or `present` from the person's display opens or raises a window of the person's,
//!   drawn by this process on its own display;
//! - a launch from a Mind's display (the shell's `open_app` for a mind, which starts the app on
//!   Mind View's display with [`crate::for_mind::FOR_MIND_VAR`] set) opens a window of that
//!   Mind's, and never touches the person's.
//!
//! Who asked is read from the asking process's own environment, by the kernel's word on its pid
//! (`SO_PEERCRED`), and what it claims can only make it a Mind's. A window of a Mind's is masked
//! and read-only under #959's rules ([`crate::for_mind`]); the id says so
//! ([`crate::WindowId::for_mind`]), so a person's window is never masked, whatever else is open.
//!
//! # Two displays, one process
//!
//! winit opens one event loop per process, on one Wayland display, the one `WAYLAND_DISPLAY`
//! named when the loop was built: `EventLoopBuilder::build` refuses a second (`RecreationAttempt`),
//! and Slint's winit backend is built on that one loop. So the process cannot put a window on a
//! Mind's display itself. A Mind's window is drawn here instead with Slint's software renderer,
//! into a window adapter of this crate's ([`streamed`]); its pixels go to a **view helper** on the
//! Mind's display and its pointer and keys come back ([`wire`], [`link`]).
//!
//! The view helper is the Mind's own launch, kept alive ([`helper`]): the process the shell
//! started on Mind View's display finds the instance, hands its launch over with the path of a
//! socket it listens on, and draws what it is sent. It opens no store and runs no model; the one
//! writer is still the instance. A Mind's launch that finds no instance starts one on the
//! person's display first, with no window ([`spawn`]), and becomes its helper.
//!
//! The trade-off is in the Mind's window alone: it is software-rendered and sent as pixels, so it
//! costs some CPU while it changes, and takes pointer, scroll and plain keys, with no IME or
//! clipboard. The person's windows are native, as before.

pub mod asker;
#[cfg(unix)]
pub mod helper;
pub mod link;
#[cfg(unix)]
pub mod mind_launch;
pub mod platform;
pub mod privacy;
pub mod report;
pub mod spawn;
pub mod streamed;
pub mod wire;

pub use asker::{asking, Asker};

/// What a Mind's launch is told of an app that does not mask for a Mind while the person has it
/// open (#972 H3): the next step, by the app's surface id.
pub fn refused_beside_the_person(id: &str) -> String {
    format!(
        "{id} is open on the person's screen and cannot also open in your view; use its actions and os_describe {id} instead."
    )
}

/// Whose a window is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// The person's: drawn by this process, on its own display.
    Person,
    /// A Mind's, on the Mind's display (`None`: it did not say which), drawn there by its helper.
    Mind { display: Option<String> },
}

impl Owner {
    pub fn is_mind(&self) -> bool {
        matches!(self, Owner::Mind { .. })
    }
}

/// Set on the instance a Mind's first launch starts on the person's display ([`spawn`]): it
/// starts with no window and waits for the launch that started it to hand over.
pub const WINDOWLESS_VAR: &str = "YANTRIK_APP_WINDOWLESS";

/// How long an instance started with no window waits for its first window before it ends.
pub const WINDOWLESS_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The start of every view helper's socket name. Not `app-`, so the mind door, which admits only
/// harness and `app-*` sockets, never serves one.
pub const VIEW_PREFIX: &str = "view-";
