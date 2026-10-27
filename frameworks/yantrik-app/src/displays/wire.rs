//! What the instance and a view helper say to each other over the helper's socket.
//!
//! The instance sends a Mind's window as it is drawn: its title and sizes, patches of its pixels,
//! and the window manager's requests its own title bar raises (raise, move, close). The helper
//! sends back its window's size and what the pointer and keyboard do on it.
//!
//! Each message is a little-endian `u32` length, then a tag byte and the fields. Everything read is
//! held to bounds before it is used: a message is at most [`MAX_MESSAGE`], a frame at most
//! [`MAX_SIDE`] a side with its patch inside it and exactly its pixels long, a string at most
//! [`MAX_TEXT`]. Either side may be a process the other did not start, so a malformed message
//! ends the link; it is never guessed at.

use std::io::{self, Read, Write};

/// The largest message the helper accepts: one full frame of [`MAX_SIDE`] a side, and room.
pub const MAX_MESSAGE: usize = 3 * (MAX_SIDE as usize) * (MAX_SIDE as usize) + 64;
/// The largest message the instance accepts from a helper: none is longer than a key's text and
/// its fields (#972 L2).
pub const MAX_FROM_HELPER: usize = MAX_TEXT + 64;
/// The largest window side, in physical pixels.
pub const MAX_SIDE: u32 = 4096;
/// The longest title or key text, in bytes.
pub const MAX_TEXT: usize = 1024;

/// What the instance tells the helper.
#[derive(Debug, Clone, PartialEq)]
pub enum ToHelper {
    /// The window's title, its preferred and minimum sizes (logical pixels), and whether it asked
    /// to be minimized or maximized.
    Props { title: String, preferred: (u32, u32), min: (u32, u32), minimized: bool, maximized: bool },
    /// A patch of the frame: the whole frame is `width` by `height`; the patch at `x`, `y` is
    /// `w` by `h`, three bytes (RGB) a pixel, row by row.
    Frame { width: u32, height: u32, x: u32, y: u32, w: u32, h: u32, rgb: Vec<u8> },
    /// Bring the window forward on its display.
    Raise,
    /// The title bar was pressed to move the window: the helper asks its compositor to move it.
    Drag,
    /// The window is closed; the helper ends.
    Close,
}

/// What a pointer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Moved,
    Pressed,
    Released,
    Exited,
}

/// What the helper tells the instance.
#[derive(Debug, Clone, PartialEq)]
pub enum FromHelper {
    /// The helper's window: its physical size and scale factor.
    Size { width: u32, height: u32, scale: f32 },
    /// The pointer, at logical `x`, `y`; `button` 0 left, 1 right, 2 middle.
    Pointer { kind: PointerKind, button: u8, x: f32, y: f32 },
    /// A wheel or touchpad scroll at `x`, `y`, by `dx`, `dy` logical pixels.
    Scroll { x: f32, y: f32, dx: f32, dy: f32 },
    /// A key pressed or released, as the text it stands for (Slint's key codes for named keys).
    Key { pressed: bool, text: String },
    /// The helper's window gained or lost the keyboard.
    Focus(bool),
    /// The helper's window was asked to close.
    CloseRequested,
}

/// Writes one message.
pub fn send(to: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| bad("a message too long to send"))?;
    to.write_all(&len.to_le_bytes())?;
    to.write_all(body)?;
    to.flush()
}

/// Reads one message's body of at most `max` bytes, or `None` at a clean end of the stream.
pub fn receive(from: &mut impl Read, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match from.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > max {
        return Err(bad("a message of a length no side sends"));
    }
    let mut body = vec![0u8; len];
    from.read_exact(&mut body)?;
    Ok(Some(body))
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

impl ToHelper {
    /// The tag a frame's body starts with.
    pub const FRAME_TAG: u8 = 2;

    /// A frame's body up to its pixels, with room for them: the window appends each row's RGB
    /// straight into it, with no copy of the patch on the way (#972 M3).
    pub fn frame_header(width: u32, height: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
        let mut b = Body(Vec::with_capacity(25 + 3 * w as usize * h as usize));
        b.tag(Self::FRAME_TAG).u32(width).u32(height).u32(x).u32(y).u32(w).u32(h);
        b.0
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Body::default();
        match self {
            ToHelper::Props { title, preferred, min, minimized, maximized } => {
                b.tag(1).text(title).u32(preferred.0).u32(preferred.1).u32(min.0).u32(min.1);
                b.u8(u8::from(*minimized)).u8(u8::from(*maximized));
            }
            ToHelper::Frame { width, height, x, y, w, h, rgb } => {
                b.0 = Self::frame_header(*width, *height, *x, *y, *w, *h);
                b.0.extend_from_slice(rgb);
            }
            ToHelper::Raise => {
                b.tag(3);
            }
            ToHelper::Drag => {
                b.tag(4);
            }
            ToHelper::Close => {
                b.tag(5);
            }
        }
        b.0
    }

    pub fn decode(body: &[u8]) -> io::Result<ToHelper> {
        let mut r = Reader(body);
        let message = match r.u8()? {
            1 => ToHelper::Props {
                title: r.text()?,
                preferred: (r.side()?, r.side()?),
                min: (r.side()?, r.side()?),
                minimized: r.u8()? != 0,
                maximized: r.u8()? != 0,
            },
            2 => {
                let (width, height) = (r.side()?, r.side()?);
                let (x, y, w, h) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let inside = x.checked_add(w).is_some_and(|e| e <= width)
                    && y.checked_add(h).is_some_and(|e| e <= height);
                if !inside {
                    return Err(bad("a patch outside its frame"));
                }
                let rgb = r.rest();
                if rgb.len() != 3 * w as usize * h as usize {
                    return Err(bad("a patch whose pixels are not its size"));
                }
                return Ok(ToHelper::Frame { width, height, x, y, w, h, rgb: rgb.to_vec() });
            }
            3 => ToHelper::Raise,
            4 => ToHelper::Drag,
            5 => ToHelper::Close,
            _ => return Err(bad("a message the helper does not know")),
        };
        r.end()?;
        Ok(message)
    }
}

impl FromHelper {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Body::default();
        match self {
            FromHelper::Size { width, height, scale } => {
                b.tag(1).u32(*width).u32(*height).f32(*scale);
            }
            FromHelper::Pointer { kind, button, x, y } => {
                let kind = match kind {
                    PointerKind::Moved => 0,
                    PointerKind::Pressed => 1,
                    PointerKind::Released => 2,
                    PointerKind::Exited => 3,
                };
                b.tag(2).u8(kind).u8(*button).f32(*x).f32(*y);
            }
            FromHelper::Scroll { x, y, dx, dy } => {
                b.tag(3).f32(*x).f32(*y).f32(*dx).f32(*dy);
            }
            FromHelper::Key { pressed, text } => {
                b.tag(4).u8(u8::from(*pressed)).text(text);
            }
            FromHelper::Focus(on) => {
                b.tag(5).u8(u8::from(*on));
            }
            FromHelper::CloseRequested => {
                b.tag(6);
            }
        }
        b.0
    }

    pub fn decode(body: &[u8]) -> io::Result<FromHelper> {
        let mut r = Reader(body);
        let message = match r.u8()? {
            1 => FromHelper::Size { width: r.side()?, height: r.side()?, scale: r.scale()? },
            2 => {
                let kind = match r.u8()? {
                    0 => PointerKind::Moved,
                    1 => PointerKind::Pressed,
                    2 => PointerKind::Released,
                    3 => PointerKind::Exited,
                    _ => return Err(bad("a pointer event the instance does not know")),
                };
                let button = r.u8()?;
                if button > 2 {
                    return Err(bad("a pointer button the instance does not know"));
                }
                FromHelper::Pointer { kind, button, x: r.place()?, y: r.place()? }
            }
            3 => FromHelper::Scroll { x: r.place()?, y: r.place()?, dx: r.place()?, dy: r.place()? },
            4 => FromHelper::Key { pressed: r.u8()? != 0, text: r.text()? },
            5 => FromHelper::Focus(r.u8()? != 0),
            6 => FromHelper::CloseRequested,
            _ => return Err(bad("a message the instance does not know")),
        };
        r.end()?;
        Ok(message)
    }
}

#[derive(Default)]
struct Body(Vec<u8>);

impl Body {
    fn tag(&mut self, t: u8) -> &mut Self {
        self.u8(t)
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn text(&mut self, s: &str) -> &mut Self {
        let mut end = s.len().min(MAX_TEXT);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.u32(end as u32);
        self.0.extend_from_slice(&s.as_bytes()[..end]);
        self
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.0.len() < n {
            return Err(bad("a message cut short"));
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    /// A window side: at most [`MAX_SIDE`].
    fn side(&mut self) -> io::Result<u32> {
        let v = self.u32()?;
        if v > MAX_SIDE {
            return Err(bad("a side larger than any window"));
        }
        Ok(v)
    }
    /// A place or a distance on the window: a finite number, within reach of it.
    fn place(&mut self) -> io::Result<f32> {
        let v = self.f32()?;
        if !v.is_finite() || v.abs() > 4.0 * MAX_SIDE as f32 {
            return Err(bad("a position no window has"));
        }
        Ok(v)
    }
    /// A scale factor a display could have.
    fn scale(&mut self) -> io::Result<f32> {
        let v = self.f32()?;
        if !(0.25..=8.0).contains(&v) {
            return Err(bad("a scale factor no display has"));
        }
        Ok(v)
    }
    fn text(&mut self) -> io::Result<String> {
        let len = self.u32()? as usize;
        if len > MAX_TEXT {
            return Err(bad("a text longer than any title"));
        }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| bad("a text that is not UTF-8"))
    }
    fn rest(&mut self) -> &[u8] {
        std::mem::take(&mut self.0)
    }
    fn end(&self) -> io::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(bad("a message with bytes after its last field"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_to(m: ToHelper) -> ToHelper {
        ToHelper::decode(&m.encode()).unwrap()
    }

    fn round_from(m: FromHelper) -> FromHelper {
        FromHelper::decode(&m.encode()).unwrap()
    }

    #[test]
    fn every_message_reads_back_as_it_was_written() {
        let props = ToHelper::Props { title: "Notes".into(), preferred: (900, 640), min: (400, 300), minimized: false, maximized: true };
        assert_eq!(round_to(props.clone()), props);
        let frame = ToHelper::Frame { width: 4, height: 3, x: 1, y: 1, w: 2, h: 2, rgb: (0..12).collect() };
        assert_eq!(round_to(frame.clone()), frame);
        let mut built = ToHelper::frame_header(4, 3, 1, 1, 2, 2);
        built.extend(0..12u8);
        assert_eq!(built, frame.encode(), "a body built in place is the same body");
        for m in [ToHelper::Raise, ToHelper::Drag, ToHelper::Close] {
            assert_eq!(round_to(m.clone()), m);
        }
        for m in [
            FromHelper::Size { width: 1280, height: 800, scale: 1.5 },
            FromHelper::Pointer { kind: PointerKind::Pressed, button: 0, x: 10.5, y: 20.0 },
            FromHelper::Scroll { x: 1.0, y: 2.0, dx: 0.0, dy: -30.0 },
            FromHelper::Key { pressed: true, text: "a".into() },
            FromHelper::Focus(true),
            FromHelper::CloseRequested,
        ] {
            assert_eq!(round_from(m.clone()), m);
        }
    }

    #[test]
    fn a_patch_must_fit_its_frame_and_be_its_size() {
        let outside = ToHelper::Frame { width: 4, height: 3, x: 3, y: 0, w: 2, h: 1, rgb: vec![0; 6] };
        assert!(ToHelper::decode(&outside.encode()).is_err());
        let short = ToHelper::Frame { width: 4, height: 3, x: 0, y: 0, w: 2, h: 1, rgb: vec![0; 5] };
        assert!(ToHelper::decode(&short.encode()).is_err());
        let huge = ToHelper::Frame { width: MAX_SIDE + 1, height: 1, x: 0, y: 0, w: 0, h: 0, rgb: vec![] };
        assert!(ToHelper::decode(&huge.encode()).is_err());
        let wraps = ToHelper::Frame { width: 4, height: 3, x: u32::MAX, y: 0, w: 2, h: 1, rgb: vec![0; 6] };
        assert!(ToHelper::decode(&wraps.encode()).is_err());
    }

    #[test]
    fn what_no_side_sends_is_refused() {
        assert!(FromHelper::decode(&[9]).is_err(), "an unknown tag");
        assert!(FromHelper::decode(&[]).is_err(), "nothing");
        let mut extra = FromHelper::Focus(true).encode();
        extra.push(0);
        assert!(FromHelper::decode(&extra).is_err(), "a byte after the last field");
        let nan = FromHelper::Pointer { kind: PointerKind::Moved, button: 0, x: f32::NAN, y: 0.0 };
        assert!(FromHelper::decode(&nan.encode()).is_err());
        let scale = FromHelper::Size { width: 10, height: 10, scale: 0.0 };
        assert!(FromHelper::decode(&scale.encode()).is_err());
        let mut button = FromHelper::Pointer { kind: PointerKind::Moved, button: 0, x: 0.0, y: 0.0 }.encode();
        button[2] = 7;
        assert!(FromHelper::decode(&button).is_err());
    }

    #[test]
    fn a_title_is_cut_at_a_character_and_the_stream_frames_messages() {
        let long = "é".repeat(MAX_TEXT);
        let ToHelper::Props { title, .. } = round_to(ToHelper::Props { title: long, preferred: (1, 1), min: (0, 0), minimized: false, maximized: false }) else {
            panic!("not props")
        };
        assert!(title.len() <= MAX_TEXT && title.chars().all(|c| c == 'é'));

        let mut stream = Vec::new();
        send(&mut stream, &ToHelper::Raise.encode()).unwrap();
        send(&mut stream, &ToHelper::Close.encode()).unwrap();
        let mut read = stream.as_slice();
        assert_eq!(ToHelper::decode(&receive(&mut read, MAX_MESSAGE).unwrap().unwrap()).unwrap(), ToHelper::Raise);
        assert_eq!(ToHelper::decode(&receive(&mut read, MAX_MESSAGE).unwrap().unwrap()).unwrap(), ToHelper::Close);
        assert!(receive(&mut read, MAX_MESSAGE).unwrap().is_none(), "a clean end");
        let mut zero: &[u8] = &[0, 0, 0, 0];
        assert!(receive(&mut zero, MAX_MESSAGE).is_err(), "an empty message is not one");
        let mut too_long: &[u8] = &u32::MAX.to_le_bytes();
        assert!(receive(&mut too_long, MAX_MESSAGE).is_err());
        let mut frame_sized: &[u8] = &(MAX_FROM_HELPER as u32 + 1).to_le_bytes();
        assert!(receive(&mut frame_sized, MAX_FROM_HELPER).is_err(), "a helper sends nothing frame-sized");
    }
}
