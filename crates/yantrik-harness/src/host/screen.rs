//! The query screen: whether a search query can be shown to a person exactly as it will be sent.
//!
//! A card asks the person to allow one search in someone else's words. It is consent only if what
//! the person reads is what goes to the web, byte for byte. So a query passes only when every
//! character draws as itself, in one place on its line, and nothing in it reads as another
//! string; anything else is refused with the reason, never cleaned up, because a cleaned query is
//! not the one that was asked.
//!
//! [`query`] is the whole screen. It is self-contained on purpose: it uses only `std`,
//! `unicode-normalization`, `unicode-properties` (feature `general-category`), `unicode-script`
//! and `unicode-security`, pinned to exact versions, so another repository can copy this file as
//! it is.
//!
//! What is refused, in the order it is checked:
//!
//! 1. Space around the query, an empty query, or more than [`MOST_QUERY_CHARS`] characters.
//! 2. Characters that draw as nothing or as something else ([`unshowable`]): Cc, Cf, Zl, Zp, Co,
//!    Cn, Cs, every space but U+0020, variation selectors, tag characters, the other
//!    default-ignorables, U+2800 BRAILLE PATTERN BLANK, U+FFFC, U+FFFD, and the card's own quote
//!    marks.
//! 3. A query that is not its own NFKC form. That one rule refuses NFD (`cafe` + U+0301 against
//!    `café`), ligatures (`ﬁ`), fullwidth forms, mathematical alphanumerics and Arabic
//!    presentation forms: each draws like another string with other bytes.
//! 4. Marks that can hide or cover ([`marks`]), counted on the canonical decomposition: any Me
//!    (enclosing mark), the overlay marks, a nonspacing mark (Mn) with no letter under it, more
//!    than [`MOST_MARKS`] Mn on one base, the same Mn twice on one base, and a dot above on a
//!    letter that already has its dot (`i` + U+0307).
//! 5. Search-engine syntax ([`engine_syntax`]): a word that starts with `!`, `:` or `<`.
//! 6. Scripts ([`mixed_script_word`], [`scripts`]): a word mixing scripts; a query with Latin,
//!    Common and Inherited and more than one other script (Han with Hiragana and Katakana, Han
//!    with Bopomofo and Han with Hangul each count as one); a Latin letter outside Basic Latin,
//!    Latin-1 Supplement, Latin Extended-A and -B and Latin Extended Additional ([`latin_ok`],
//!    which refuses the IPA block U+0250–02AF); and a word that is not ASCII but whose UTS #39
//!    skeleton is ([`lookalike_word`]): Cyrillic `расе`, Armenian `օ`, Cherokee `Ꭺ`, an en dash.
//! 7. Right-to-left letters (bidi class R or AL) with digits or Latin letters ([`rtl_mixed`]),
//!    whose order on the line is not the order they are sent in.

use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
use unicode_script::{Script, UnicodeScript};
use unicode_security::confusable_detection::skeleton;

/// The longest query a card shows.
pub const MOST_QUERY_CHARS: usize = 300;

/// The most nonspacing marks one base may carry (Vietnamese `ệ` is two).
pub const MOST_MARKS: usize = 2;

/// Whether `q` may be shown on a card as the exact search; why not, if not.
pub fn query(q: &str) -> Result<(), String> {
    if q.trim_matches(' ').is_empty() || q.trim() != q {
        return Err("the query is the exact search, with no space around it".into());
    }
    if q.chars().count() > MOST_QUERY_CHARS {
        return Err(format!("a query is at most {MOST_QUERY_CHARS} characters"));
    }
    if let Some((c, why)) = q.chars().find_map(|c| unshowable(c).map(|why| (c, why))) {
        return Err(format!(
            "the query holds U+{:04X} ({why}), which the card cannot show as it is; a query is shown exactly or not at all",
            c as u32
        ));
    }
    if !q.nfkc().eq(q.chars()) {
        return Err(format!(
            "the query is not in NFKC form (it would be {:?}): it holds characters that draw like others \
             with other bytes, such as decomposed accents, ligatures, fullwidth or styled letters; send it \
             as NFKC",
            q.nfkc().collect::<String>()
        ));
    }
    marks(q)?;
    if let Some(word) = engine_syntax(q) {
        return Err(format!(
            "the query holds search-engine syntax (`{word}`) that would choose another service or setting; \
             a granted search goes to the web as written"
        ));
    }
    if let Some((word, scripts)) = mixed_script_word(q) {
        return Err(format!(
            "the query mixes {} letters in the word {:?}, which can read as another word; refused",
            scripts.join(" and "),
            word
        ));
    }
    scripts(q)?;
    if rtl_mixed(q) {
        return Err("the query mixes right-to-left letters with digits or Latin letters, which the card would \
                    draw in another order than they are sent; send them as separate searches"
            .into());
    }
    Ok(())
}

/// Why `c` may not be in a query the card shows, if it may not: what it is, for the refusal.
///
/// An allowlist by category: letters, marks, numbers, punctuation, symbols and the one space
/// U+0020. Refused are the categories that draw as nothing or as something else (Cc control,
/// Cf format, Zl and Zp separators, Co private use, Cn unassigned, Cs surrogates), every space
/// but U+0020, the variation selectors, the tag characters, the other default-ignorable marks
/// that draw as nothing, the blank braille pattern, the object and replacement characters, and
/// the card's own quote marks, which would close its quote.
pub fn unshowable(c: char) -> Option<&'static str> {
    match c {
        ' ' => return None,
        '\u{201c}' | '\u{201d}' | '"' => return Some("a quote mark, which would end the card's quote"),
        '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}' => return Some("a variation selector"),
        '\u{e0000}'..='\u{e007f}' => return Some("a tag character"),
        // Default-ignorable, so drawn as nothing, though not Cf: the combining grapheme joiner,
        // the Hangul fillers, the Khmer inherent vowels and the Mongolian variation selectors.
        '\u{034f}' | '\u{115f}' | '\u{1160}' | '\u{17b4}' | '\u{17b5}' | '\u{180b}'..='\u{180f}' | '\u{3164}' | '\u{ffa0}' => {
            return Some("a character that draws as nothing")
        }
        // A symbol, so not whitespace, that draws as a blank.
        '\u{2800}' => return Some("the blank braille pattern, which draws as a space"),
        // Some fonts draw the object replacement character as nothing; the replacement character
        // stands for bytes that were lost, so it is not the query either.
        '\u{fffc}' => return Some("the object replacement character"),
        '\u{fffd}' => return Some("the replacement character"),
        _ => {}
    }
    match c.general_category() {
        GeneralCategory::Control => Some("a control character"),
        GeneralCategory::Format => Some("an invisible format character"),
        GeneralCategory::LineSeparator => Some("a line separator"),
        GeneralCategory::ParagraphSeparator => Some("a paragraph separator"),
        GeneralCategory::PrivateUse => Some("a private-use character"),
        GeneralCategory::Unassigned => Some("an unassigned code point"),
        GeneralCategory::Surrogate => Some("a surrogate"),
        GeneralCategory::SpaceSeparator => Some("a space other than U+0020"),
        _ if c.is_whitespace() => Some("whitespace other than U+0020"),
        _ => None,
    }
}

/// Marks drawn through or over their letter, which can strike out or cover what is beside them.
fn overlay(c: char) -> bool {
    matches!(c, '\u{0334}'..='\u{0338}' | '\u{20d2}'..='\u{20d3}' | '\u{20d8}'..='\u{20da}' | '\u{20e5}'..='\u{20ea}')
}

/// Letters that already carry a dot, so a dot above them draws as nothing new.
fn soft_dotted(c: char) -> bool {
    matches!(c, 'i' | 'j' | '\u{456}' | '\u{458}' | '\u{12f}' | '\u{249}' | '\u{268}')
}

/// The combining marks in `q`, on its canonical decomposition, are ones that draw visibly and
/// within their line: no enclosing or overlay mark, no mark on nothing, at most [`MOST_MARKS`]
/// nonspacing marks on one base, never the same one twice, and no dot above a dotted letter.
/// Spacing marks (Mc) take their own room and are not counted.
pub fn marks(q: &str) -> Result<(), String> {
    let mut base: Option<char> = None;
    let mut on_base: Vec<char> = Vec::new();
    for c in q.nfd() {
        let category = c.general_category();
        if category == GeneralCategory::EnclosingMark {
            return Err(format!("the query holds the enclosing mark U+{:04X}, which can draw over its neighbours", c as u32));
        }
        if overlay(c) {
            return Err(format!("the query holds the overlay mark U+{:04X}, which draws through its letter", c as u32));
        }
        if category != GeneralCategory::NonspacingMark {
            base = (c != ' ').then_some(c);
            on_base.clear();
            continue;
        }
        let Some(letter) = base else {
            return Err(format!("the query holds the mark U+{:04X} with no letter under it", c as u32));
        };
        if on_base.contains(&c) {
            return Err(format!("the query holds the mark U+{:04X} twice on one letter, which draws as once", c as u32));
        }
        if c == '\u{0307}' && soft_dotted(letter) && canonical_combining_class(c) == 230 {
            return Err(format!("the query puts a dot above {letter:?}, which already has one"));
        }
        on_base.push(c);
        if on_base.len() > MOST_MARKS {
            return Err(format!(
                "the query stacks more than {MOST_MARKS} marks on one letter, which can draw outside its line"
            ));
        }
    }
    Ok(())
}

/// The first word (split at U+0020) that a search engine reads as syntax rather than words:
/// `!` (a bang that picks another engine, `!wp`, `!!g`), `:` (a language, `:fr`) and `<` (a
/// timeout). A grant is for a search on the web as written, not for whichever service a word
/// names. `site:x` and `a!b` do not start with one, so they are words.
pub fn engine_syntax(q: &str) -> Option<&str> {
    q.split(' ').find(|w| w.starts_with(['!', ':', '<']))
}

/// The writing systems that mix scripts by nature: each is one, as UTS #39 has it.
const ONE_SYSTEM: [&[Script]; 3] =
    [&[Script::Han, Script::Hiragana, Script::Katakana], &[Script::Han, Script::Bopomofo], &[Script::Han, Script::Hangul]];

/// Whether `seen`, scripts other than Latin, Common and Inherited, are one writing system.
fn one_system(seen: &[Script]) -> bool {
    seen.len() <= 1 || ONE_SYSTEM.iter().any(|system| seen.iter().all(|s| system.contains(s)))
}

/// The scripts of `word`'s characters, in order, without Common, Inherited and Unknown.
fn scripts_of(word: &str) -> Vec<Script> {
    let mut seen: Vec<Script> = Vec::new();
    for c in word.chars() {
        let script = c.script();
        if !matches!(script, Script::Common | Script::Inherited | Script::Unknown) && !seen.contains(&script) {
            seen.push(script);
        }
    }
    seen
}

/// The first word (split at U+0020) whose letters come from more than one script, with the
/// scripts' names: `раypal` (Cyrillic and Latin), `ΑpplΕ` (Greek and Latin). Common and
/// inherited characters (digits, punctuation, marks) belong to every script. Han with Hiragana
/// and Katakana, Han with Bopomofo, and Han with Hangul are each one writing system, so Japanese,
/// Chinese and Korean words pass.
pub fn mixed_script_word(q: &str) -> Option<(String, Vec<&'static str>)> {
    q.split(' ').find_map(|word| {
        let seen = scripts_of(word);
        (seen.len() > 1 && !one_system(&seen)).then(|| (word.to_string(), seen.iter().map(|s| s.full_name()).collect()))
    })
}

/// Letters that read as Latin ones in common fonts but that the UTS #39 skeleton leaves as they
/// are, each with the Latin letter it reads as. Cyrillic `Ԛ` (U+051A) draws as `Q`.
pub const LATIN_LOOKALIKES: &[(char, char)] = &[('\u{51a}', 'Q')];

/// What `word` reads as: its UTS #39 skeleton, with [`LATIN_LOOKALIKES`] put in.
pub fn reads_as(word: &str) -> String {
    skeleton(word).map(|c| LATIN_LOOKALIKES.iter().find(|(l, _)| *l == c).map_or(c, |(_, a)| *a)).collect()
}

/// The first word (split at U+0020) that is not plain ASCII but [`reads_as`] plain ASCII: a word
/// the person would read as an ASCII word that it is not, such as Cyrillic `расе` ("pace"),
/// Armenian `օ`, Cherokee `Ꭺ` or an en dash. The comparison is over whole words, so a word with
/// a letter that draws as itself (`Москва`, `και`, `café`) passes.
pub fn lookalike_word(q: &str) -> Option<&str> {
    q.split(' ').find(|word| !word.is_ascii() && reads_as(word).is_ascii())
}

/// Whether `c`, if it is a Latin letter, is in Basic Latin, Latin-1 Supplement, Latin Extended-A
/// or -B, or Latin Extended Additional: the letters of the languages written in Latin. The IPA
/// block (U+0250–02AF), Latin Extended-C, -D and -E and the rest hold phonetic and lookalike
/// letters (`ɡ` draws as `g`), so they are refused.
pub fn latin_ok(c: char) -> bool {
    !(c.script() == Script::Latin && c.is_alphabetic())
        || matches!(c, '\u{0}'..='\u{24f}' | '\u{1e00}'..='\u{1eff}')
}

/// The query's scripts: Latin, Common and Inherited, and at most one other writing system; Latin
/// letters only from the blocks [`latin_ok`] allows; and no word that reads as an ASCII word it is
/// not ([`lookalike_word`]).
pub fn scripts(q: &str) -> Result<(), String> {
    let others: Vec<Script> = scripts_of(q).into_iter().filter(|s| *s != Script::Latin).collect();
    if !one_system(&others) {
        let names: Vec<&str> = others.iter().map(|s| s.full_name()).collect();
        return Err(format!(
            "the query mixes {} letters; a query may use Latin and at most one other script",
            names.join(" and ")
        ));
    }
    if let Some(c) = q.chars().find(|c| !latin_ok(*c)) {
        return Err(format!(
            "the query holds the Latin letter U+{:04X}, which is outside the blocks languages are written \
             in (Basic Latin, Latin-1, Latin Extended-A and -B, Latin Extended Additional) and can read as \
             another letter; refused",
            c as u32
        ));
    }
    if let Some(word) = lookalike_word(q) {
        return Err(format!(
            "the word {word:?} is not ASCII but reads as the ASCII word {:?} (its UTS #39 skeleton), so the \
             card cannot show which was sent; refused",
            reads_as(word)
        ));
    }
    Ok(())
}

/// Whether `c` is a letter of bidi class R or AL: a letter in the ranges DerivedBidiClass.txt gives
/// R or AL (Hebrew, Arabic, Syriac, Thaana, NKo, Samaritan, Mandaic and the RTL blocks of the
/// supplementary planes). Every letter in them is R or AL; their digits are AN or EN, their
/// marks NSM, and neither is a letter.
pub fn rtl_letter(c: char) -> bool {
    let rtl_block = matches!(c,
        '\u{0590}'..='\u{08ff}' | '\u{fb1d}'..='\u{fdff}' | '\u{fe70}'..='\u{feff}'
        | '\u{10800}'..='\u{10fff}' | '\u{1e800}'..='\u{1efff}');
    rtl_block
        && matches!(
            c.general_category(),
            GeneralCategory::UppercaseLetter
                | GeneralCategory::LowercaseLetter
                | GeneralCategory::TitlecaseLetter
                | GeneralCategory::ModifierLetter
                | GeneralCategory::OtherLetter
        )
}

/// Whether `q` holds right-to-left letters with digits (any Nd) or Latin letters: the card's bidi
/// layout would draw them in another order than they are sent. A query purely right-to-left, or
/// right-to-left with spaces and punctuation, is not mixed.
pub fn rtl_mixed(q: &str) -> bool {
    q.chars().any(rtl_letter)
        && q.chars().any(|c| {
            c.general_category() == GeneralCategory::DecimalNumber || (c.script() == Script::Latin && c.is_alphabetic())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(q: &str) -> bool {
        query(q).is_err()
    }

    fn why(q: &str) -> String {
        query(q).unwrap_err()
    }

    #[test]
    fn a_plain_query_passes_and_space_around_it_or_too_long_does_not() {
        assert_eq!(query("rust 1.97 release notes"), Ok(()));
        assert!(refused(""));
        assert!(refused(" "));
        assert!(refused(" padded "));
        assert!(refused(&"x".repeat(MOST_QUERY_CHARS + 1)));
        assert_eq!(query(&"x".repeat(MOST_QUERY_CHARS)), Ok(()));
    }

    #[test]
    fn ordinary_queries_pass() {
        for ok in [
            "weather in Pune",
            "café crème brûlée recipe",
            "Москва погода",
            "Москва weather",
            "αβγ decay",
            "東京 天気",
            "ラーメン屋 渋谷",
            "日本語の文法",
            "서울 날씨",
            "北京 天气 注音ㄅㄆ",
            "naïve Bayes 2027",
            "C++ std::vector",
            "pizza 🍕 near me",
            "it's 3/4 - ok?",
            "crème brûlée",
            "Größe Straße",
            "Müller über naïve",
            "élève à l'école",
            "Москва погода",
            "Ελλάδα και Κύπρος",
            "नई दिल्ली में मौसम कैसा है",
            "क्षेत्रफल",
            "site:docs.rs a!b",
            "हिन्दी समाचार",
            "tiếng Việt ệ",
            "שלום עולם",
            "مرحبا، العالم!",
        ] {
            assert_eq!(query(ok), Ok(()), "{ok}");
        }
    }

    // ── What draws as nothing or as something else (round 1) ──

    #[test]
    fn control_characters_cc_are_refused() {
        for q in ["two\nlines", "a\tb", "bell\u{7}", "nel\u{85}x", "del\u{7f}x"] {
            assert!(refused(q), "{q:?}");
        }
    }

    #[test]
    fn format_characters_cf_are_refused() {
        // U+200B, U+200D, U+2060, U+FEFF, the soft hyphen, the Arabic letter mark, the bidi controls.
        for c in ['\u{200b}', '\u{200d}', '\u{2060}', '\u{feff}', '\u{ad}', '\u{61c}', '\u{200e}', '\u{200f}', '\u{202e}', '\u{2066}', '\u{2069}'] {
            let q = format!("rust{c}editions");
            assert!(why(&q).contains(&format!("U+{:04X}", c as u32)), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn line_and_paragraph_separators_zl_zp_are_refused() {
        assert!(refused("a\u{2028}b"));
        assert!(refused("a\u{2029}b"));
        // A fake closing quote, a new paragraph, the card's own words.
        assert!(refused("foo\u{201d} \u{2029}Always: until you revoke it"));
        assert!(refused("foo \u{2029}Always: until you revoke it"), "the separator alone");
    }

    #[test]
    fn private_use_co_and_unassigned_cn_are_refused() {
        for c in ['\u{e000}', '\u{f8ff}', '\u{f0000}', '\u{10fffd}'] {
            assert!(refused(&format!("x{c}y")), "U+{:04X}", c as u32);
        }
        for c in ['\u{378}', '\u{fffe}', '\u{e0080}'] {
            assert_eq!(unshowable(c), Some("an unassigned code point"), "U+{:04X}", c as u32);
            assert!(refused(&format!("x{c}y")));
        }
    }

    #[test]
    fn variation_selectors_are_refused() {
        for c in ['\u{fe00}', '\u{fe0f}', '\u{e0100}', '\u{e01ef}'] {
            assert_eq!(unshowable(c), Some("a variation selector"), "U+{:04X}", c as u32);
        }
        assert!(refused("heart \u{2764}\u{fe0f}"));
    }

    #[test]
    fn tag_characters_are_refused() {
        let smuggled: String = "rust editions".chars().chain("ssn 123".chars().map(|c| char::from_u32(0xe0000 + c as u32).unwrap())).collect();
        assert!(refused(&smuggled));
        for c in ['\u{e0001}', '\u{e0020}', '\u{e0041}', '\u{e007f}'] {
            assert!(refused(&format!("x{c}")), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn whitespace_other_than_the_space_is_refused() {
        for c in ['\u{a0}', '\u{2002}', '\u{2003}', '\u{2009}', '\u{200a}', '\u{202f}', '\u{205f}', '\u{3000}', '\u{1680}', '\u{b}', '\u{c}', '\r'] {
            assert!(refused(&format!("a{c}b")), "U+{:04X}", c as u32);
        }
        assert!(!refused("a b c"), "U+0020 is the one space");
        assert!(!refused("a  b"), "two of them are still spaces, shown as they are");
    }

    #[test]
    fn other_marks_that_draw_as_nothing_are_refused() {
        for c in ['\u{34f}', '\u{115f}', '\u{1160}', '\u{17b4}', '\u{180e}', '\u{3164}', '\u{ffa0}'] {
            assert!(refused(&format!("a{c}b")), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn the_cards_own_quote_marks_are_refused() {
        for q in ["foo\u{201d} bar", "\u{201c}foo", "say \"hi\""] {
            assert!(why(q).contains("quote"), "{q}");
        }
        assert!(!refused("it's «ok»"), "other quote marks cannot close the card's");
    }

    #[test]
    fn a_word_mixing_scripts_is_refused_with_its_scripts_named() {
        let w = why("\u{440}\u{430}ypal login");
        assert!(w.contains("Cyrillic") && w.contains("Latin"), "{w}");
        let w = why("g\u{3bf}ogle");
        assert!(w.contains("Greek") && w.contains("Latin"), "{w}");
        assert!(refused("Αpple"), "Greek capital alpha");
        assert!(refused("abcабв"));
        assert!(refused("東京tokyo"), "Han and Latin in one word");
        assert!(refused("rust\u{662}\u{660}"), "Arabic-Indic digits are Arabic");
        assert_eq!(mixed_script_word("Москва weather"), None, "two words, one script each");
    }

    // ── Round 2: each case in the review ──

    #[test]
    fn nfd_cafe_is_refused_and_nfc_cafe_passes() {
        assert!(why("cafe\u{301}").contains("NFKC"));
        assert_eq!(query("caf\u{e9}"), Ok(()));
    }

    #[test]
    fn stacked_marks_are_refused() {
        // A second acute on é, and a dot over the dot of i.
        assert!(why("caf\u{e9}\u{301}").contains("twice"));
        assert!(why("i\u{307}nfo").contains("dot above"));
        assert!(marks("e\u{301}\u{301}x").unwrap_err().contains("twice"));
        // Three different marks on one base, as sent (NFC, so the screen reaches the count) and
        // as counted.
        assert!(why("q\u{300}\u{301}\u{302}").contains("stacks"));
        assert!(marks("o\u{300}\u{301}\u{302}").unwrap_err().contains("stacks"));
        // A mark on nothing.
        assert!(marks(" \u{301}x").unwrap_err().contains("no letter"));
        assert!(marks("\u{301}x").is_err());
        // Two different marks on one base is as far as it goes: Vietnamese.
        assert_eq!(marks("\u{1ec7}"), Ok(()));
    }

    #[test]
    fn a_braille_blank_word_is_refused() {
        assert!(why("rust \u{2800}\u{2800}\u{2800}").contains("U+2800"));
        assert!(refused("rust\u{2800}x"));
        assert_eq!(query("braille \u{2801}"), Ok(()), "a braille pattern with dots draws as itself");
    }

    #[test]
    fn a_whole_cyrillic_word_of_latin_lookalikes_is_refused() {
        // `расе` reads as "pace".
        assert!(why("rust \u{440}\u{430}\u{441}\u{435}").contains("reads as the ASCII word"));
        assert_eq!(query("rust pace"), Ok(()));
    }

    #[test]
    fn a_lone_cyrillic_a_is_refused() {
        assert!(why("rust \u{430} b").contains("reads as the ASCII word"));
        assert!(refused("\u{430}"));
        assert!(refused("rust \u{3bf}"), "a lone Greek omicron too");
        assert_eq!(query("rust \u{436}"), Ok(()), "ж reads as nothing Latin");
        assert!(refused("rust \u{431}"), "a lone б reads as the digit 6");
    }

    // ── Round 3: lookalikes from any script, by the UTS #39 skeleton ──

    #[test]
    fn armenian_cherokee_and_ipa_lookalikes_are_refused() {
        // Armenian օ (U+0585) draws as o, Cherokee Ꭺ (U+13AA) as A.
        assert!(why("rust \u{585}").contains("reads as the ASCII word \"o\""));
        assert!(why("\u{13aa}").contains("reads as the ASCII word \"A\""));
        assert!(refused("\u{13aa}pple"), "Cherokee and Latin in one word");
        assert!(refused("g\u{585}\u{585}gle"));
        // IPA ɡ (U+0261) draws as g: outside the Latin blocks, alone or in a word.
        assert!(why("\u{261}").contains("U+0261"));
        assert!(why("\u{261}oogle").contains("U+0261"));
        assert!(!latin_ok('\u{250}') && !latin_ok('\u{2af}') && !latin_ok('\u{259}'));
        assert!(latin_ok('\u{24f}') && latin_ok('\u{1ec7}') && latin_ok('\u{df}') && latin_ok('\u{3b1}'));
    }

    #[test]
    fn a_word_is_refused_only_when_its_skeleton_is_ascii_and_it_is_not() {
        assert_eq!(lookalike_word("Москва погода"), None);
        assert_eq!(lookalike_word("caf\u{e9} na\u{ef}ve"), None, "an accent draws as itself");
        assert_eq!(lookalike_word("rust 1.97"), None, "an ASCII word is what it reads as");
        assert_eq!(lookalike_word("\u{3ba}\u{3b1}\u{3b9}"), None, "Greek και: κ is not k");
        assert_eq!(lookalike_word("x \u{51a}"), Some("\u{51a}"), "Cyrillic Ԛ, left by the skeleton");
        assert_eq!(query("Ελλάδα και Κύπρος"), Ok(()));
    }

    #[test]
    fn punctuation_that_reads_as_ascii_is_refused_too() {
        // Known over-refusals: the en dash, curly single quotes, the multiplication sign and primes
        // each read as ASCII punctuation they are not, so a word of them carries a hidden bit.
        for q in ["it's 3/4 \u{2013} ok?", "\u{2018}fine\u{2019}", "1920\u{d7}1080", "5\u{2032}"] {
            assert!(why(q).contains("reads as the ASCII word"), "{q}");
        }
    }

    #[test]
    fn mathematical_alphanumerics_are_refused() {
        // 𝗉𝖺𝗒𝗉𝖺𝗅, and 𝗉aypal.
        assert!(why("\u{1d5c9}\u{1d5ba}\u{1d5d2}\u{1d5c9}\u{1d5ba}\u{1d5c5}").contains("NFKC"));
        assert!(refused("\u{1d5c9}aypal"));
    }

    #[test]
    fn a_ligature_is_refused() {
        assert!(why("\u{fb01}nd rust").contains("NFKC"));
        assert_eq!(query("find rust"), Ok(()));
    }

    #[test]
    fn fullwidth_is_refused() {
        assert!(why("\u{ff52}\u{ff55}\u{ff53}\u{ff54}").contains("NFKC"));
    }

    #[test]
    fn arabic_presentation_forms_are_refused() {
        assert!(why("\u{fe8d}\u{fee0}").contains("NFKC"));
        assert_eq!(query("\u{627}\u{644}"), Ok(()), "the letters themselves pass");
    }

    #[test]
    fn the_long_stroke_overlay_u0336_is_refused() {
        assert!(why("r\u{336}u\u{336}s\u{336}t\u{336}").contains("overlay"));
        for c in ['\u{334}', '\u{335}', '\u{337}', '\u{338}', '\u{20d2}', '\u{20d3}', '\u{20d8}', '\u{20d9}', '\u{20da}'] {
            assert!(refused(&format!("a{c}")), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn the_enclosing_mark_u20e5_and_any_me_are_refused() {
        // U+20E5 is an overlay mark (Mn); U+20DD and U+20E3 are enclosing (Me).
        assert!(why("a\u{20e5}").contains("overlay"));
        for c in ['\u{20e6}', '\u{20e7}', '\u{20e8}', '\u{20e9}', '\u{20ea}'] {
            assert!(refused(&format!("a{c}")), "U+{:04X}", c as u32);
        }
        assert!(why("a\u{20dd}").contains("enclosing"));
        assert!(why("1\u{20e3}").contains("enclosing"));
        assert!(refused("\u{488}x"), "Cyrillic hundred thousands sign, Me");
    }

    #[test]
    fn two_hundred_fifty_stacked_marks_are_refused() {
        let tower: String = std::iter::once('a').chain(std::iter::repeat_n('\u{300}', 250)).collect();
        assert!(refused(&tower));
        let mixed: String = std::iter::once('a').chain((0..250).map(|i| char::from_u32(0x300 + (i % 0x30)).unwrap())).collect();
        assert!(refused(&mixed));
    }

    #[test]
    fn the_object_and_replacement_characters_are_refused() {
        assert!(why("rust \u{fffc}").contains("U+FFFC"));
        assert!(why("rust \u{fffd}").contains("U+FFFD"));
    }

    #[test]
    fn one_other_script_per_query_beside_latin() {
        assert_eq!(query("Москва weather"), Ok(()));
        assert!(why("Москва αβγ").contains("Cyrillic and Greek"));
        assert!(refused("東京 Москва"));
        assert!(refused("שלום مرحبا"), "Hebrew and Arabic");
        assert_eq!(query("東京 ひらがな カタカナ"), Ok(()), "Han with kana is one system");
        assert_eq!(query("서울 漢字"), Ok(()), "Han with Hangul is one system");
        assert!(refused("ひらがな 서울"), "kana with Hangul is not");
    }

    // ── Search-engine syntax ──

    #[test]
    fn search_engine_syntax_is_refused() {
        for q in ["!wp foo", "!!g x", ":fr x", "<3 x", ":)", "rust !g", "x :de"] {
            let w = why(q);
            assert!(w.contains("search-engine syntax"), "{q}: {w}");
        }
        assert_eq!(
            why("!wp foo"),
            "the query holds search-engine syntax (`!wp`) that would choose another service or setting; \
             a granted search goes to the web as written"
        );
        for ok in ["site:x", "filetype:pdf", "a!b", "C++", "rust site:docs.rs", "x<y", "say: hi"] {
            assert_eq!(query(ok), Ok(()), "{ok}");
        }
    }

    // ── Right to left ──

    #[test]
    fn rtl_letters_with_digits_or_latin_are_refused() {
        assert!(why("\u{5d0} 100 200").contains("right-to-left"));
        assert!(refused("שלום world"));
        assert!(refused("مرحبا 2027"));
        assert!(refused("مرحبا \u{662}\u{660}\u{662}\u{667}"), "Arabic-Indic digits are digits too");
        assert_eq!(query("שלום עולם"), Ok(()));
        assert_eq!(query("مرحبا، العالم؟"), Ok(()), "with punctuation");
        assert_eq!(query("hello 2027"), Ok(()), "no RTL letters, nothing to reorder");
        assert!(rtl_letter('\u{5d0}') && rtl_letter('\u{628}') && rtl_letter('\u{710}') && rtl_letter('\u{1e900}'));
        assert!(!rtl_letter('\u{660}') && !rtl_letter('\u{5b0}') && !rtl_letter('a'));
    }
}
