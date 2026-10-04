//! Whether a name a sender gave borrows the desktop's own.
//!
//! Only the desktop may be called `Yantrik` (#114). Refusing the exact word left "Yantrik
//! Security" and "Yаntrik" with a Cyrillic а (security review of #614). The first version
//! dropped every character it did not know, so fullwidth "Ｙａｎｔｒｉｋ", mathematical "𝐘𝐚𝐧𝐭𝐫𝐢𝐤",
//! accented "Ýantrik" and Latin "ɑ" passed (third-pass review). So a name is first brought to
//! a skeleton:
//!
//! 1. NFKD, which turns fullwidth and mathematical letters into plain ones and splits an
//!    accented letter into its base and a combining mark;
//! 2. combining marks dropped, so "Ý" is "Y";
//! 3. Cyrillic, Greek and Latin letters drawn like a letter of "yantrik" mapped to it;
//! 4. case folded, and everything that is not a letter or digit dropped, separators and
//!    zero-width characters included.
//!
//! This is a small, purpose-built confusables fold for one word, not a full UTS #39 skeleton.
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// The desktop's own name, as its skeleton.
const DESKTOP: &str = "yantrik";

/// Does `claim` borrow the desktop's name? True when its skeleton starts with "yantrik" (a name
/// that only mentions the desktop after its own, "Notes for Yantrik", is the caller's), or when
/// it spells "yantrik" anywhere with letters that are not plain ASCII.
pub fn borrows_the_desktops_name(claim: &str) -> bool {
    let mut skeleton = String::new();
    let mut lookalike = false;
    for c in claim.nfkd().filter(|c| !is_combining_mark(*c)) {
        if c.is_ascii_alphanumeric() {
            skeleton.push(match c.to_ascii_lowercase() {
                '1' | 'l' => 'i',
                other => other,
            });
        } else if let Some(latin) = confusable(c) {
            lookalike = true;
            skeleton.push(latin);
        }
        // Everything else -- spaces, punctuation, zero-width characters -- is dropped.
    }
    // A name that needed folding at all (fullwidth, mathematical, accented) is a lookalike too.
    let folded = claim.chars().any(|c| !c.is_ascii());
    skeleton.starts_with(DESKTOP) || ((lookalike || folded) && skeleton.contains(DESKTOP))
}

/// The Latin letter of "yantrik" a non-ASCII letter is drawn like, if it is one.
fn confusable(c: char) -> Option<char> {
    Some(match c {
        'у' | 'У' | 'ү' | 'Ү' | 'γ' | 'Υ' | 'ʏ' | 'ý' | 'ÿ' => 'y',
        'а' | 'А' | 'α' | 'Α' | 'ɑ' | 'ᴀ' => 'a',
        'п' | 'η' | 'Ν' | 'ո' | 'ɴ' => 'n',
        'т' | 'Т' | 'τ' | 'Τ' | 'ᴛ' => 't',
        'г' | 'ʀ' | 'ᴦ' => 'r',
        'і' | 'І' | 'ї' | 'Ї' | 'ι' | 'Ι' | 'ı' | 'ӏ' | 'Ӏ' | 'ɪ' => 'i',
        'к' | 'К' | 'κ' | 'Κ' | 'ᴋ' => 'k',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktops_name_in_any_dress_is_borrowed() {
        for claim in [
            "Yantrik",
            "Yantrik Security",
            "YANTRIK-update",
            "Y a n t r i k",
            "Yan\u{200B}trik",
            "Yantr1k",
            "Y\u{0430}ntrik",                 // Cyrillic а
            "\u{03A5}antrik Alerts",          // Greek Υ
            "Your Y\u{0430}ntrik bill",       // a lookalike anywhere
            "\u{0423}\u{0430}\u{043F}\u{0442}\u{0433}\u{0456}\u{043A}", // all Cyrillic
            // The third-pass review's four:
            "\u{FF39}\u{FF41}\u{FF4E}\u{FF54}\u{FF52}\u{FF49}\u{FF4B}", // fullwidth Ｙａｎｔｒｉｋ
            "\u{1D418}\u{1D41A}\u{1D427}\u{1D42D}\u{1D42B}\u{1D422}\u{1D424}", // bold 𝐘𝐚𝐧𝐭𝐫𝐢𝐤
            "\u{00DD}antrik",                 // Ý, precomposed
            "Y\u{0251}ntrik",                 // Latin ɑ
            "Ya\u{0301}ntrik",                // a + combining acute
            "Notes for Y\u{FF41}ntrik",       // folded letters anywhere
        ] {
            assert!(borrows_the_desktops_name(claim), "{claim:?}");
        }
    }

    #[test]
    fn a_name_of_its_own_is_not_borrowed() {
        for claim in ["Studio", "Downloads", "Notes for Yantrik", "Tantrik", "Yanni", "Café", "notify-send", ""] {
            assert!(!borrows_the_desktops_name(claim), "{claim:?}");
        }
    }
}
