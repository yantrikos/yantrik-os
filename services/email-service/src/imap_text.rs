//! Text that comes from a caller and ends up in an IMAP or SMTP command, held to one rule.
//!
//! The mail methods are reached by the Email app and by the companion's mail tools, whose
//! arguments a model chose after reading mail from strangers. Anything in a command line that a
//! caller could shape is therefore hostile until checked. The rule, for every `email.*` method:
//!
//! - **Never a command by concatenation.** A string goes into a command only as a quoted string
//!   with `\` and `"` escaped ([`quoted`]), or through the IMAP library's own quoting.
//! - **No control characters, ever.** CR, LF, NUL and every other control character are refused
//!   ([`plain`]): a CR or LF ends the command and starts another one, which is how a search for
//!   "invoices" became "expunge the folder" ([`find_uids`] is where that one was).
//! - **Bounded.** A query, a folder name, a message id, an account id each have a length cap.
//! - **Numbers are numbers.** A message id is parsed as a UID before it is used.
//! - **A `Message-ID` is a token.** [`thread_id`] accepts `<local@host>` with no space or
//!   control character, and anything else threads nothing instead of reaching a header.
//!
//! [`check_params`] applies the first three to every text parameter the mail methods take, before
//! an account is looked up or a socket opened. No sockets in the rules themselves, so they are
//! tested as they stand; [`find_uids`] runs over any stream, so a scripted server can watch what
//! it sends.

use std::io::{Read, Write};

use yantrik_ipc_contracts::email::ServiceError;

/// The longest folder name, account id or message id accepted. IMAP mailbox names are shorter in
/// practice; this is a ceiling, not a target.
pub const MAX_NAME: usize = 255;
/// The longest search text accepted.
pub const MAX_QUERY: usize = 256;
/// The longest `Message-ID` header value threaded under (RFC 5322 line length).
const MAX_MESSAGE_ID: usize = 998;

fn refused(what: &str, why: &str) -> ServiceError {
    // Fixed text: the offending value is never echoed back, it may be an attack.
    ServiceError { code: -32602, message: format!("`{what}` {why}") }
}

/// `value` if it is fit to go into a command: no control character (CR, LF and NUL included), and
/// at most `max` characters.
pub fn plain<'a>(what: &str, value: &'a str, max: usize) -> Result<&'a str, ServiceError> {
    if value.chars().any(char::is_control) {
        return Err(refused(what, "may not contain a line break or other control character"));
    }
    if value.chars().count() > max {
        return Err(refused(what, "is too long"));
    }
    Ok(value)
}

/// `value` as an IMAP quoted string: checked by [`plain`], then `\` and `"` escaped, inside quotes.
pub fn quoted(what: &str, value: &str, max: usize) -> Result<String, ServiceError> {
    let value = plain(what, value, max)?;
    Ok(format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The SEARCH criteria for a free-text query: subject or sender contains it. One line, always.
pub fn search_criteria(query: &str) -> Result<String, ServiceError> {
    let text = quoted("query", query, MAX_QUERY)?;
    Ok(format!("OR SUBJECT {text} FROM {text}"))
}

/// Every text parameter the mail methods put into a command, checked before anything else
/// happens. Absent parameters pass; a non-string where text is expected is left to the method's
/// own parsing.
pub fn check_params(params: &serde_json::Value) -> Result<(), ServiceError> {
    for (key, max) in [
        ("account_id", MAX_NAME),
        ("folder", MAX_NAME),
        ("target_folder", MAX_NAME),
        ("message_id", MAX_NAME),
        ("query", MAX_QUERY),
    ] {
        if let Some(text) = params[key].as_str() {
            plain(key, text, max)?;
        }
    }
    if let Some(ids) = params["message_ids"].as_array() {
        for id in ids.iter().filter_map(|v| v.as_str()) {
            plain("message_ids", id, MAX_NAME)?;
        }
    }
    Ok(())
}

/// A `Message-ID` fit to go into `In-Reply-To` and `References`: `<…>` with no space and no
/// control character. Anything else is not threaded under (and not refused: a reply to a message
/// with an odd id should still go).
pub fn thread_id(parent: &str) -> Option<String> {
    let parent = parent.trim();
    let inner = parent.strip_prefix('<')?.strip_suffix('>')?;
    let ok = !inner.is_empty()
        && parent.len() <= MAX_MESSAGE_ID
        && !inner.contains(['<', '>'])
        && inner.chars().all(|c| c.is_ascii_graphic());
    ok.then(|| parent.to_string())
}

/// What is said when the server cannot take text outside ASCII.
const NO_LITERALS: &str = "This mail server cannot search for text with accents or other non-English letters.";

/// UIDs in the selected folder whose subject or sender contains `query`.
///
/// The one place a free-text query becomes an IMAP command. The query is checked and quoted
/// ([`search_criteria`]) before the folder is even selected, so a refused query sends nothing.
///
/// **Non-ASCII text.** An IMAP quoted string is 7-bit, so `"café"` is not a command a server will
/// parse ("Could not parse command"). Such a query is sent as `SEARCH CHARSET UTF-8` with the text
/// as an IMAP literal (RFC 3501 section 4.3, `{n+}` from LITERAL+ or LITERAL-, RFC 7888): the
/// literal carries its byte count, so what is inside it cannot end the command, and CR, LF and NUL
/// are refused before any of this ([`plain`]) all the same. A server that offers neither LITERAL+
/// nor LITERAL- cannot be sent a literal without waiting for its `+` (which the library does not
/// let a caller do), so it is told no in words. ASCII queries are sent exactly as before.
pub fn find_uids<T: Read + Write>(
    session: &mut imap::Session<T>,
    folder: &str,
    query: &str,
) -> Result<Vec<u32>, ServiceError> {
    let criteria = search_criteria(query)?;
    plain("folder", folder, MAX_NAME)?;
    session.select(folder).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {folder} failed: {e}"),
    })?;
    if query.is_ascii() {
        let found = session
            .search(&criteria)
            .map_err(|e| ServiceError { code: -32000, message: format!("IMAP SEARCH failed: {e}") })?;
        return Ok(found.into_iter().collect());
    }
    let literals = session
        .capabilities()
        .map(|caps| caps.has_str("LITERAL+") || caps.has_str("LITERAL-"))
        .map_err(|e| ServiceError { code: -32000, message: format!("IMAP CAPABILITY failed: {e}") })?;
    if !literals {
        return Err(ServiceError { code: -32000, message: NO_LITERALS.to_string() });
    }
    let reply = session
        .run_command_and_read_response(utf8_search_command(query)?)
        .map_err(|e| ServiceError { code: -32000, message: format!("IMAP SEARCH failed: {e}") })?;
    Ok(search_reply(&reply))
}

/// The command for a query with non-ASCII text: `SEARCH CHARSET UTF-8 OR SUBJECT {n+}` and the
/// text, then ` FROM {n+}` and the text again. `n` is the text's length in bytes, which is what
/// makes the literal safe: the server reads exactly that many bytes, whatever they are.
pub fn utf8_search_command(query: &str) -> Result<String, ServiceError> {
    let text = plain("query", query, MAX_QUERY)?;
    let n = text.len();
    Ok(format!("SEARCH CHARSET UTF-8 OR SUBJECT {{{n}+}}\r\n{text} FROM {{{n}+}}\r\n{text}"))
}

/// The numbers in a server's `* SEARCH 2 3 9` answer. Anything else in the reply is ignored.
pub fn search_reply(reply: &[u8]) -> Vec<u32> {
    String::from_utf8_lossy(reply)
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix("* SEARCH"))
        .flat_map(|numbers| numbers.split_whitespace().filter_map(|n| n.parse().ok()).collect::<Vec<u32>>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    /// What a hostile caller would send: a quote to leave the string, a line break to start a
    /// command of its own, and the commands that would empty a mailbox.
    const INJECTION: &str = "x\" FROM \"x\r\nZ1 UID STORE 1:* +FLAGS.SILENT (\\Deleted)\r\nZ2 EXPUNGE\r\nZ3 NOOP \"";

    #[test]
    fn a_line_break_nul_or_any_control_character_is_refused_and_never_echoed() {
        for bad in ["a\r\nb", "a\nb", "a\rb", "a\0b", "a\u{7f}b", "a\tb", "a\u{1b}[2Jb", "a\u{85}b"] {
            let e = plain("folder", bad, MAX_NAME).unwrap_err();
            assert_eq!(e.code, -32602, "{bad:?}");
            assert!(!e.message.contains(['\r', '\n', '\0']), "the value was echoed: {}", e.message);
        }
        assert_eq!(plain("folder", "[Gmail]/Sent Mail", MAX_NAME).unwrap(), "[Gmail]/Sent Mail");
        assert_eq!(plain("folder", "Boîte de réception", MAX_NAME).unwrap(), "Boîte de réception");
    }

    #[test]
    fn length_is_capped() {
        assert!(plain("query", &"a".repeat(MAX_QUERY), MAX_QUERY).is_ok());
        assert_eq!(plain("query", &"a".repeat(MAX_QUERY + 1), MAX_QUERY).unwrap_err().code, -32602);
    }

    #[test]
    fn a_quoted_string_cannot_be_left_by_a_quote_or_a_backslash() {
        assert_eq!(quoted("q", r#"say "hi" \o/"#, MAX_QUERY).unwrap(), r#""say \"hi\" \\o/""#);
        assert_eq!(quoted("q", "plain", MAX_QUERY).unwrap(), "\"plain\"");
        // A trailing backslash must not escape the closing quote.
        assert_eq!(quoted("q", "end\\", MAX_QUERY).unwrap(), "\"end\\\\\"");
    }

    #[test]
    fn the_search_criteria_are_one_line_and_the_injection_is_refused() {
        assert_eq!(search_criteria("invoice").unwrap(), "OR SUBJECT \"invoice\" FROM \"invoice\"");
        let e = search_criteria(INJECTION).unwrap_err();
        assert_eq!(e.code, -32602);
        // The same quotes and backslashes with no line break are only text.
        let quote_only = search_criteria("x\" FROM \"x").unwrap();
        assert!(!quote_only.contains(['\r', '\n']));
        assert_eq!(quote_only, "OR SUBJECT \"x\\\" FROM \\\"x\" FROM \"x\\\" FROM \\\"x\"");
    }

    #[test]
    fn every_text_parameter_the_mail_methods_take_is_checked() {
        for key in ["account_id", "folder", "target_folder", "message_id", "query"] {
            let params = serde_json::json!({ key: INJECTION });
            assert_eq!(check_params(&params).unwrap_err().code, -32602, "{key}");
        }
        let many = serde_json::json!({ "message_ids": ["1", INJECTION] });
        assert_eq!(check_params(&many).unwrap_err().code, -32602);
        let fine = serde_json::json!({ "account_id": "a", "folder": "INBOX", "target_folder": "Archive", "message_id": "7", "query": "dinner" });
        assert!(check_params(&fine).is_ok());
        assert!(check_params(&serde_json::json!({})).is_ok());
    }

    #[test]
    fn a_thread_id_is_one_angle_bracketed_token() {
        assert_eq!(thread_id(" <a.1@example.com> ").as_deref(), Some("<a.1@example.com>"));
        for bad in ["a@b", "<>", "<a b@c>", "<a@b>\r\nBcc: x@y", "<a<b@c>", "<a\u{e9}@c>", "", "<a@b"] {
            assert_eq!(thread_id(bad), None, "{bad:?}");
        }
        assert_eq!(thread_id(&format!("<{}>", "a".repeat(1000))), None);
    }

    /// A server that says what the protocol needs and writes down every line it is sent. `caps` is
    /// what it answers to CAPABILITY. A command whose line ends in a literal marker (`{5+}`) goes on
    /// in the next line, as a non-synchronising literal does, and is answered when it is complete.
    fn scripted_server(caps: &'static str) -> (TcpStream, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut first: Option<String> = None;
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { return };
                log.lock().unwrap().push(line.clone());
                let head = first.get_or_insert_with(|| line.clone()).clone();
                if line.ends_with("+}") {
                    continue; // the literal's bytes are the next line
                }
                first = None;
                let (tag, command) = head.split_once(' ').unwrap_or((&head, ""));
                let reply = match command.split(' ').next().unwrap_or("") {
                    "SELECT" => format!("* 3 EXISTS\r\n{tag} OK selected\r\n"),
                    "SEARCH" => format!("* SEARCH 2 3\r\n{tag} OK done\r\n"),
                    "CAPABILITY" => format!("* CAPABILITY {caps}\r\n{tag} OK done\r\n"),
                    _ => format!("{tag} OK done\r\n"),
                };
                if writer.write_all(reply.as_bytes()).is_err() {
                    return;
                }
            }
        });
        (TcpStream::connect(address).unwrap(), seen)
    }

    /// A session on a server that offers LITERAL+, as Dovecot does.
    fn session() -> (imap::Session<TcpStream>, Arc<Mutex<Vec<String>>>) {
        session_with("IMAP4rev1 LITERAL+")
    }

    fn session_with(caps: &'static str) -> (imap::Session<TcpStream>, Arc<Mutex<Vec<String>>>) {
        let (stream, seen) = scripted_server(caps);
        let client = imap::Client::new(stream);
        let session = client.login("u", "p").map_err(|e| e.0).unwrap();
        (session, seen)
    }

    /// What the server was sent after LOGIN, without the tags.
    fn commands(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        let lines = seen.lock().unwrap().clone();
        let mut out = Vec::new();
        for (n, line) in lines.iter().enumerate().skip(1) {
            // A line after a literal marker is the literal's bytes: it carries no tag.
            let continues = lines[n - 1].ends_with("+}");
            out.push(if continues { line.clone() } else { line.split_once(' ').map(|(_, c)| c).unwrap_or(line).to_string() });
        }
        out
    }

    #[test]
    fn a_hostile_search_sends_nothing_at_all() {
        let (mut session, seen) = session();
        let before = seen.lock().unwrap().len();
        let refused = find_uids(&mut session, "INBOX", INJECTION).unwrap_err();
        assert_eq!(refused.code, -32602);
        assert_eq!(seen.lock().unwrap().len(), before, "a refused query reached the server: {:?}", seen.lock().unwrap());
    }

    #[test]
    fn a_hostile_folder_sends_nothing_at_all() {
        let (mut session, seen) = session();
        let before = seen.lock().unwrap().len();
        assert_eq!(find_uids(&mut session, "INBOX\r\nZ9 EXPUNGE", "x").unwrap_err().code, -32602);
        assert_eq!(seen.lock().unwrap().len(), before);
    }

    #[test]
    fn a_quote_in_a_search_is_one_command_with_the_quote_escaped() {
        let (mut session, seen) = session();
        let uids = find_uids(&mut session, "INBOX", "x\" FROM \"x").unwrap();
        assert_eq!(uids.len(), 2);
        let lines = seen.lock().unwrap().clone();
        let commands: Vec<&str> = lines.iter().map(|l| l.split_once(' ').map(|(_, c)| c).unwrap_or("")).collect();
        // LOGIN, SELECT, SEARCH: three commands for three calls, and the search is one line.
        assert_eq!(commands.len(), 3, "{lines:?}");
        assert_eq!(commands[2], "SEARCH OR SUBJECT \"x\\\" FROM \\\"x\" FROM \"x\\\" FROM \\\"x\"");
        assert!(!lines.iter().any(|l| l.contains("EXPUNGE") || l.contains("STORE")), "{lines:?}");
    }

    #[test]
    fn an_ascii_search_is_sent_exactly_as_it_was_and_asks_for_no_capabilities() {
        let (mut session, seen) = session();
        assert_eq!(find_uids(&mut session, "INBOX", "invoice").unwrap().len(), 2);
        assert_eq!(commands(&seen), ["SELECT \"INBOX\"", "SEARCH OR SUBJECT \"invoice\" FROM \"invoice\""]);
    }

    #[test]
    fn a_utf8_search_is_charset_utf8_with_literals_of_the_right_byte_count() {
        // "café" is four characters and five bytes: the count must be bytes.
        let (mut session, seen) = session();
        let mut uids = find_uids(&mut session, "INBOX", "café").unwrap();
        uids.sort();
        assert_eq!(uids, [2, 3]);
        assert_eq!(
            commands(&seen),
            ["SELECT \"INBOX\"", "CAPABILITY", "SEARCH CHARSET UTF-8 OR SUBJECT {5+}", "café FROM {5+}", "café"]
        );
    }

    #[test]
    fn a_server_with_literal_minus_gets_the_same_search() {
        let (mut session, seen) = session_with("IMAP4rev1 UTF8=ACCEPT LITERAL-");
        assert_eq!(find_uids(&mut session, "INBOX", "日本語").unwrap().len(), 2);
        assert!(commands(&seen).contains(&"SEARCH CHARSET UTF-8 OR SUBJECT {9+}".to_string()), "{:?}", commands(&seen));
    }

    #[test]
    fn a_server_that_takes_no_literals_is_told_no_in_words_and_sent_no_search() {
        let (mut session, seen) = session_with("IMAP4rev1");
        let e = find_uids(&mut session, "INBOX", "café").unwrap_err();
        assert!(e.message.contains("cannot search for text with accents"), "{}", e.message);
        assert!(!commands(&seen).iter().any(|c| c.contains("SEARCH")), "{:?}", commands(&seen));
    }

    #[test]
    fn hostile_text_is_refused_in_utf8_too_and_nothing_is_sent() {
        // CR, LF and NUL beside a non-ASCII letter, and the control character U+0085 (next line).
        for bad in ["café\r\nZ1 EXPUNGE", "café\nZ1 EXPUNGE", "café\0", "caf\u{e9}\u{85}x"] {
            let (mut session, seen) = session();
            let before = seen.lock().unwrap().len();
            assert_eq!(find_uids(&mut session, "INBOX", bad).unwrap_err().code, -32602, "{bad:?}");
            assert_eq!(seen.lock().unwrap().len(), before, "{bad:?} reached the server");
            assert!(utf8_search_command(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn u2028_is_text_not_a_line_break_and_travels_inside_its_byte_count() {
        // U+2028 is three bytes in UTF-8 and no IMAP line ending; inside a literal it is only data.
        let command = utf8_search_command("a\u{2028}b").unwrap();
        assert_eq!(command, "SEARCH CHARSET UTF-8 OR SUBJECT {5+}\r\na\u{2028}b FROM {5+}\r\na\u{2028}b");
        assert_eq!(command.matches("\r\n").count(), 2, "only the two the protocol needs");
    }

    #[test]
    fn the_search_reply_is_read_for_its_numbers_only() {
        assert_eq!(search_reply(b"* SEARCH 4 7 9\r\na1 OK done\r\n"), [4, 7, 9]);
        assert_eq!(search_reply(b"* SEARCH\r\na1 OK done\r\n"), Vec::<u32>::new());
        assert_eq!(search_reply(b"* 3 EXISTS\r\n* SEARCH 1\r\n* SEARCH 2 x 3\r\n"), [1, 2, 3]);
    }
}
