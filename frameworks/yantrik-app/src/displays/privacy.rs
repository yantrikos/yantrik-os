//! A Mind's window in Private mode (#972 L5): while the person is private, its helper is sent one
//! blank frame in place of what the window draws, and nothing more of it, as `describe` is
//! replaced by its private view (#924). When the person stops being private the window is drawn
//! whole again.
//!
//! Decided on the link's writing thread, where reading the privacy file belongs, so the UI thread
//! never does file IO for it.

use super::wire::ToHelper;

/// What one link has sent while the person was private.
#[derive(Debug, Default)]
pub struct Blanking {
    blanked: bool,
    /// The whole frame's size, from the last frame that went.
    size: Option<(u32, u32)>,
}

/// What the writer does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// Send this body.
    Send(Vec<u8>),
    /// Send nothing.
    Nothing,
    /// The person is no longer private: ask the window to be drawn whole, and send this body.
    Redraw(Option<Vec<u8>>),
}

impl Blanking {
    /// A frame's body, offered while the person is (or is not) `private`.
    pub fn frame(&mut self, body: Vec<u8>, private: bool) -> Next {
        if let Some(size) = size_of(&body) {
            self.size = Some(size);
        }
        match (private, self.blanked) {
            (true, true) => Next::Nothing,
            (true, false) => self.blank(),
            (false, true) => {
                self.blanked = false;
                Next::Redraw(Some(body))
            }
            (false, false) => Next::Send(body),
        }
    }

    /// No frame came for a while: whether to blank now, or redraw, as the person's Private mode
    /// changed while the window drew nothing.
    pub fn idle(&mut self, private: bool) -> Next {
        match (private, self.blanked) {
            (true, false) => self.blank(),
            (false, true) => {
                self.blanked = false;
                Next::Redraw(None)
            }
            _ => Next::Nothing,
        }
    }

    fn blank(&mut self) -> Next {
        self.blanked = true;
        match self.size {
            Some((width, height)) => {
                let pixels = 3 * width as usize * height as usize;
                Next::Send(ToHelper::Frame { width, height, x: 0, y: 0, w: width, h: height, rgb: vec![0; pixels] }.encode())
            }
            None => Next::Nothing,
        }
    }
}

/// The whole frame's size a frame's body names.
fn size_of(body: &[u8]) -> Option<(u32, u32)> {
    if body.first() != Some(&ToHelper::FRAME_TAG) || body.len() < 9 {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    Some((word(1), word(5)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, fill: u8) -> Vec<u8> {
        ToHelper::Frame { width, height, x: 0, y: 0, w: width, h: height, rgb: vec![fill; 3 * (width * height) as usize] }.encode()
    }

    #[test]
    fn in_private_mode_a_minds_window_shows_one_blank_frame_and_nothing_more() {
        let mut b = Blanking::default();
        assert_eq!(b.frame(frame(2, 2, 9), false), Next::Send(frame(2, 2, 9)));
        assert_eq!(b.frame(frame(2, 2, 7), true), Next::Send(frame(2, 2, 0)), "the person's state is not sent");
        assert_eq!(b.frame(frame(2, 2, 7), true), Next::Nothing);
        assert_eq!(b.idle(true), Next::Nothing);
        assert_eq!(b.frame(frame(2, 2, 5), false), Next::Redraw(Some(frame(2, 2, 5))), "drawn whole again after");
    }

    #[test]
    fn private_mode_turned_on_while_the_window_is_still_blanks_it() {
        let mut b = Blanking::default();
        assert_eq!(b.idle(true), Next::Nothing, "nothing was ever shown");
        b.frame(frame(3, 1, 9), false);
        assert_eq!(b.idle(true), Next::Send(frame(3, 1, 0)));
        assert_eq!(b.idle(false), Next::Redraw(None));
        assert_eq!(b.idle(false), Next::Nothing);
    }
}
