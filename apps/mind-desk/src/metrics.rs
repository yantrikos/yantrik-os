//! The numbers the desk and the nested compositor must agree on.
//!
//! The desk is one window, a bar along the top of Mind View: the header, the tool strip and the
//! footer, one under the other. The compositor reserves the margin in `config/labwc-mind/rc.xml`
//! and finds the bar by its title there; the desk makes its window exactly this tall, with this
//! title. A test (`tests::compositor`) reads the file and fails if either side moves without the
//! other, and `tests/mind-desk-placement` measures where labwc really puts the bar.
//!
//! # Why one bar, and why it overlaps the apps by a pixel
//!
//! labwc 0.8 cannot hold a fixed-height bar to the bottom edge of a display that is resized: a
//! rule can place a window at an absolute position (`MoveTo`) and have it follow the display's
//! width (`ToggleMaximize direction="horizontal"`), but a window kept at the display's width is
//! put back from its stored ("natural") position whenever the display changes, and the stored
//! position of a window in the top row is not at the top: labwc re-centres the window if that
//! position does not touch the usable area, which the reserved margin makes it not. So the bar
//! is as tall as the margin plus one pixel: the pixel is inside the usable area, labwc keeps the
//! stored position, and the bar stays at the top at every size. The pixel is the bar's own bottom
//! edge, a hairline drawn over the first row of the apps' area.
//!
//! The footer is under the strip because nothing in labwc's rules anchors a window to the bottom
//! at any size (`MoveToEdge` does nothing when a rule runs at a window's first map).

/// The header: the Mind's name and state, and what the person may do about it.
pub const HEADER_HEIGHT: u32 = 76;
/// The tool strip, under the header.
pub const STRIP_HEIGHT: u32 = 40;
/// The footer, under the strip: the apps open, the safety line, the model.
pub const FOOTER_HEIGHT: u32 = 44;

/// How far the bar reaches into the usable area: its bottom hairline.
pub const OVERLAP: u32 = 1;

/// The compositor's top margin: what the apps are kept below.
pub const MARGIN_TOP: u32 = HEADER_HEIGHT + STRIP_HEIGHT + FOOTER_HEIGHT;

/// The window's height: the margin and the hairline over the apps' first row.
pub const BAR_HEIGHT: u32 = MARGIN_TOP + OVERLAP;

/// The title the compositor's window rule matches (`ui/app.slint` sets it).
pub const TITLE: &str = "Yantrik Mind Desk";
