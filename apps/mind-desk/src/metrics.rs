//! The numbers the desk and the nested compositor must agree on.
//!
//! The compositor reserves the margins in `config/labwc-mind/rc.xml` and finds each bar by its
//! title there; the desk makes its two windows exactly this tall, with these titles. A test
//! (`tests::compositor`) reads the file and fails if either side moves without the other.

/// The header's height, in logical pixels: the state line and the Mind's own line.
pub const HEADER_HEIGHT: u32 = 76;
/// The tool strip's, under the header.
pub const STRIP_HEIGHT: u32 = 40;
/// The top window: the header and the strip. The compositor's top margin.
pub const TOP_HEIGHT: u32 = HEADER_HEIGHT + STRIP_HEIGHT;
/// The bottom window: the footer. The compositor's bottom margin.
pub const BOTTOM_HEIGHT: u32 = 44;

/// The titles the compositor's window rules match (`ui/app.slint` sets them from `role`).
pub const TOP_TITLE: &str = "Yantrik Mind Desk Top";
pub const BOTTOM_TITLE: &str = "Yantrik Mind Desk Bottom";
