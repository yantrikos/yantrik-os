//! What the reader says on its standard error, kept to what is safe to keep (#1256).
//!
//! The reader says why it could not start or read there (a model it cannot load, an image it
//! cannot open). Nothing else: the text it reads from the screen goes out on its standard
//! output as answers, and is never logged here. Even so, a line from it is bounded in length
//! (a model's path is fine, a megabyte is not) and stripped of control characters before it
//! is logged or put in an error.

use std::io::BufRead;

/// The most of one line that is kept.
pub const MOST_LINE: usize = 200;

/// Hand `each` every non-empty line of `source`, cut to [`MOST_LINE`] bytes. The rest of a
/// longer line is read and dropped, not held.
pub fn each_line(mut source: impl BufRead, mut each: impl FnMut(String)) {
    let mut kept: Vec<u8> = Vec::new();
    loop {
        let (used, ended) = {
            let Ok(buffered) = source.fill_buf() else { break };
            if buffered.is_empty() {
                break;
            }
            match buffered.iter().position(|byte| *byte == b'\n') {
                Some(end) => {
                    keep(&mut kept, &buffered[..end]);
                    (end + 1, true)
                }
                None => {
                    keep(&mut kept, buffered);
                    (buffered.len(), false)
                }
            }
        };
        source.consume(used);
        if ended {
            emit(&mut kept, &mut each);
        }
    }
    emit(&mut kept, &mut each);
}

fn keep(kept: &mut Vec<u8>, more: &[u8]) {
    let room = MOST_LINE.saturating_sub(kept.len());
    kept.extend_from_slice(&more[..room.min(more.len())]);
}

fn emit(kept: &mut Vec<u8>, each: &mut impl FnMut(String)) {
    let text: String = String::from_utf8_lossy(kept).chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    kept.clear();
    let text = text.trim();
    if !text.is_empty() {
        each(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(input: &[u8]) -> Vec<String> {
        let mut got = Vec::new();
        each_line(std::io::BufReader::with_capacity(16, input), |line| got.push(line));
        got
    }

    #[test]
    fn lines_come_out_whole_and_blank_ones_do_not_come_out() {
        assert_eq!(lines(b"yantrik-ocr: cannot load /opt/x.rten\n\n  \nsecond\nno newline at the end"), ["yantrik-ocr: cannot load /opt/x.rten", "second", "no newline at the end"]);
    }

    #[test]
    fn a_line_is_cut_to_its_bound_and_the_rest_is_not_held() {
        let long = vec![b'x'; 1_000_000];
        let mut input = long.clone();
        input.extend_from_slice(b"\nnext\n");
        let got = lines(&input);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].len(), MOST_LINE);
        assert_eq!(got[1], "next");
    }

    #[test]
    fn control_characters_are_not_kept() {
        assert_eq!(lines(b"a\x1b[31mred\tb\r\n"), ["a [31mred b"]);
    }
}
