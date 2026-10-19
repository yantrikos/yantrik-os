//! How the picture on screen is turned and mirrored, and the pixels that makes.
//!
//! The rotate and flip buttons set three properties and the screen drew only one of them: the
//! rotation went to `transform-rotation`, and the flips went nowhere. The Slint file said they
//! were "pre-processed in the backend", and no backend ever did it, so both flip buttons lit up
//! and left the picture exactly as it was. The transform is now applied here, to the pixels,
//! which works the same on every renderer and lets a quarter-turned photo fit the window by its
//! new shape rather than overflowing it.
//!
//! Nothing here touches the file. There is no Save in this viewer.

/// The view's orientation: the stored picture turned clockwise by `rotation`, then mirrored
/// left-to-right (`flip_h`) and top-to-bottom (`flip_v`) as the screen sees it.
///
/// Every button acts on what is on screen, so pressing rotate after a flip still turns the
/// picture clockwise and pressing flip after a turn still mirrors it left-to-right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Orientation {
    /// Degrees clockwise: 0, 90, 180 or 270.
    pub rotation: i32,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Orientation {
    /// Shown as stored.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Turn what is on screen a quarter clockwise.
    pub fn rotate_right(self) -> Self {
        self.turned(90)
    }

    /// Turn what is on screen a quarter anticlockwise.
    pub fn rotate_left(self) -> Self {
        self.turned(270)
    }

    /// Mirror what is on screen left-to-right.
    pub fn flip_horizontal(self) -> Self {
        Self { flip_h: !self.flip_h, ..self }
    }

    /// Mirror what is on screen top-to-bottom.
    pub fn flip_vertical(self) -> Self {
        Self { flip_v: !self.flip_v, ..self }
    }

    /// A screen turn of `degrees` clockwise, carried past the mirrors to the stored rotation.
    ///
    /// One mirror reverses the sense of a turn (a mirrored clockwise turn is an anticlockwise
    /// one); two mirrors are a half turn and leave it alone.
    fn turned(self, degrees: i32) -> Self {
        let step = if self.flip_h != self.flip_v { 360 - degrees } else { degrees };
        Self { rotation: (self.rotation + step).rem_euclid(360), ..self }
    }

    /// The picture as it should be drawn: `pixels` is `width` × `height`, row by row. Returns
    /// the new pixels and the new width and height (swapped by a quarter turn).
    pub fn apply<P: Copy>(&self, pixels: &[P], width: usize, height: usize) -> (Vec<P>, usize, usize) {
        debug_assert_eq!(pixels.len(), width * height);
        let quarter = self.rotation.rem_euclid(360) / 90;
        let (out_w, out_h) = if quarter % 2 == 1 { (height, width) } else { (width, height) };
        let mut out = Vec::with_capacity(pixels.len());
        for y in 0..out_h {
            let ry = if self.flip_v { out_h - 1 - y } else { y };
            for x in 0..out_w {
                let rx = if self.flip_h { out_w - 1 - x } else { x };
                // Where (rx, ry) of the turned picture comes from in the stored one.
                let (sx, sy) = match quarter {
                    1 => (ry, height - 1 - rx),
                    2 => (width - 1 - rx, height - 1 - ry),
                    3 => (width - 1 - ry, rx),
                    _ => (rx, ry),
                };
                out.push(pixels[sy * width + sx]);
            }
        }
        (out, out_w, out_h)
    }
}

#[cfg(test)]
mod tests {
    use super::Orientation;

    /// A 3 × 2 picture whose every pixel is different:
    ///
    /// ```text
    /// 1 2 3
    /// 4 5 6
    /// ```
    fn picture() -> (Vec<u8>, usize, usize) {
        (vec![1, 2, 3, 4, 5, 6], 3, 2)
    }

    /// What a person sees happen, done directly to the pixels on screen.
    fn screen_cw((p, w, h): (Vec<u8>, usize, usize)) -> (Vec<u8>, usize, usize) {
        let mut out = Vec::new();
        for y in 0..w {
            for x in 0..h {
                out.push(p[(h - 1 - x) * w + y]);
            }
        }
        (out, h, w)
    }
    fn screen_mirror_h((p, w, h): (Vec<u8>, usize, usize)) -> (Vec<u8>, usize, usize) {
        let out = (0..h).flat_map(|y| (0..w).rev().map(move |x| (y, x))).map(|(y, x)| p[y * w + x]).collect();
        (out, w, h)
    }
    fn screen_mirror_v((p, w, h): (Vec<u8>, usize, usize)) -> (Vec<u8>, usize, usize) {
        let out = (0..h).rev().flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| p[y * w + x]).collect();
        (out, w, h)
    }

    fn drawn(o: Orientation) -> (Vec<u8>, usize, usize) {
        let (p, w, h) = picture();
        o.apply(&p, w, h)
    }

    /// All eight ways a picture can lie, reached by pressing the buttons.
    fn every_orientation() -> Vec<Orientation> {
        let mut all = Vec::new();
        for turns in 0..4 {
            for (h, v) in [(false, false), (true, false), (false, true), (true, true)] {
                let mut o = Orientation::default();
                for _ in 0..turns {
                    o = o.rotate_right();
                }
                if h {
                    o = o.flip_horizontal();
                }
                if v {
                    o = o.flip_vertical();
                }
                all.push(o);
            }
        }
        all
    }

    #[test]
    fn a_quarter_turn_right_stands_the_picture_on_its_side() {
        let o = Orientation::default().rotate_right();
        assert_eq!(o.rotation, 90);
        assert_eq!(drawn(o), (vec![4, 1, 5, 2, 6, 3], 2, 3));
    }

    #[test]
    fn a_flip_mirrors_what_is_drawn() {
        assert_eq!(drawn(Orientation::default().flip_horizontal()), (vec![3, 2, 1, 6, 5, 4], 3, 2));
        assert_eq!(drawn(Orientation::default().flip_vertical()), (vec![4, 5, 6, 1, 2, 3], 3, 2));
    }

    #[test]
    fn every_button_does_to_the_screen_what_it_says_whatever_came_before() {
        for o in every_orientation() {
            assert_eq!(drawn(o.rotate_right()), screen_cw(drawn(o)), "rotate right from {o:?}");
            assert_eq!(
                drawn(o.rotate_left()),
                screen_cw(screen_cw(screen_cw(drawn(o)))),
                "rotate left from {o:?}"
            );
            assert_eq!(drawn(o.flip_horizontal()), screen_mirror_h(drawn(o)), "flip horizontal from {o:?}");
            assert_eq!(drawn(o.flip_vertical()), screen_mirror_v(drawn(o)), "flip vertical from {o:?}");
        }
    }

    #[test]
    fn undoing_a_press_puts_the_picture_back() {
        for o in every_orientation() {
            assert_eq!(o.rotate_right().rotate_left(), o);
            assert_eq!(o.flip_horizontal().flip_horizontal(), o);
            assert_eq!(o.flip_vertical().flip_vertical(), o);
        }
        let four = (0..4).fold(Orientation::default(), |o, _| o.rotate_right());
        assert!(four.is_identity(), "four quarter turns are none");
    }

    #[test]
    fn the_stored_rotation_stays_a_quarter() {
        for o in every_orientation() {
            for next in [o.rotate_left(), o.rotate_right(), o.flip_horizontal(), o.flip_vertical()] {
                assert!([0, 90, 180, 270].contains(&next.rotation), "{next:?}");
            }
        }
    }
}
