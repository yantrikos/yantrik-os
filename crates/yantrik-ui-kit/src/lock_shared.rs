//! What the shell and the session-lock client (`yantrik-lock`) both need about the lock screen,
//! once: the avatar's letter, and a picture's size read from its header before anything decodes
//! it. The lock client cannot depend on the shell, and both depend on the kit.

/// The letter in the avatar disc: the first letter or digit of the name, upper-cased; nothing for
/// no name.
pub fn initial_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// Width and height of a PNG or JPEG, from its header alone; `None` for any other format or a
/// header that does not parse. A picture is checked with this before it is decoded, because a
/// small file can claim a picture of billions of pixels and the decoder would try to hold them.
pub fn picture_size(bytes: &[u8]) -> Option<(u32, u32)> {
    png_size(bytes).or_else(|| jpeg_size(bytes))
}

/// From a PNG's IHDR, which by the format is the first chunk.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let be = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some((be(16), be(20)))
}

/// From a JPEG's first start-of-frame marker (SOF0–SOF15, less DHT, JPG and DAC).
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            return None;
        }
        let marker = bytes[at + 1];
        if marker == 0xFF {
            at += 1; // fill byte
            continue;
        }
        let len = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            if at + 9 > bytes.len() {
                return None;
            }
            let h = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]) as u32;
            return Some((w, h));
        }
        if len < 2 {
            return None;
        }
        at += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_avatar_letter_is_the_first_letter_of_the_name() {
        assert_eq!(initial_of("Pranab"), "P");
        assert_eq!(initial_of("  ørjan"), "Ø");
        assert_eq!(initial_of("\"quoted\""), "Q");
        assert_eq!(initial_of(""), "");
    }

    #[test]
    fn a_png_header_gives_its_size() {
        let mut b = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&2560u32.to_be_bytes());
        b.extend_from_slice(&1600u32.to_be_bytes());
        assert_eq!(picture_size(&b), Some((2560, 1600)));
    }

    #[test]
    fn a_jpeg_header_gives_its_size_past_other_segments() {
        // SOI, an APP0 segment of 16 bytes, then SOF0 claiming 65000×40000.
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        b.extend_from_slice(&[0u8; 14]);
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&40000u16.to_be_bytes());
        b.extend_from_slice(&65000u16.to_be_bytes());
        assert_eq!(picture_size(&b), Some((65000, 40000)));
    }

    #[test]
    fn anything_else_has_no_size() {
        assert_eq!(picture_size(b"GIF89a......"), None);
        assert_eq!(picture_size(b""), None);
        assert_eq!(picture_size(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01]), None, "a segment shorter than its own length field");
    }
}
