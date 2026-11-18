//! Which message a caller means: a row number, or part of a subject or sender.

/// As much of a listed message as naming one needs.
#[derive(Debug, Clone, Default)]
pub struct MessageRow {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
}

impl MessageRow {
    pub fn new(subject: &str, from_name: &str, from_addr: &str) -> Self {
        Self {
            subject: subject.to_string(),
            from_name: from_name.to_string(),
            from_addr: from_addr.to_string(),
        }
    }
}

/// The row a caller meant, or a refusal that names what it asked for.
///
/// A number is a position in the list as it is shown, counting from one, because that is how a
/// caller that has just read `describe.messages` refers to what it read. Text is matched against
/// the subject, the sender's name and their address, exact before partial, because that is how a
/// person names mail.
///
/// No refusal here quotes an empty string. `nothing in this folder matches ""` was the answer to
/// `open_message which=1`, and it is worse than useless: it reports a search for nothing, which
/// is not what was asked, so the caller retries the same call.
pub fn resolve_which(rows: &[MessageRow], which: &str) -> Result<usize, String> {
    let want = which.trim();
    if want.is_empty() {
        return Err("no message was named: give a row number, or part of a subject or sender"
            .to_string());
    }

    // A number is a row number and nothing else. It is deliberately not tried as text
    // afterwards: "9" is a substring of "Invoice R0093", so a caller that asked for the ninth
    // message of a folder holding three would be handed the second one and told nothing. A
    // subject that is only digits is reachable by any other part of it, or by its sender.
    if let Ok(n) = want.parse::<i64>() {
        if n < 1 {
            return Err(format!("row numbers count from 1, so there is no message {n}"));
        }
        let n = n as usize;
        if n <= rows.len() {
            return Ok(n - 1);
        }
        return Err(match rows.len() {
            0 => format!("there is no message {n}: this folder is showing none"),
            1 => format!("there is no message {n}: this folder is showing 1"),
            many => format!("there is no message {n}: this folder is showing {many}"),
        });
    }

    let lower = want.to_lowercase();
    let matches = |row: &MessageRow, exact: bool| {
        let subject = row.subject.to_lowercase();
        let from = row.from_name.to_lowercase();
        let addr = row.from_addr.to_lowercase();
        if exact {
            subject == lower || from == lower || addr == lower
        } else {
            subject.contains(&lower) || from.contains(&lower) || addr.contains(&lower)
        }
    };
    if let Some(at) = rows.iter().position(|r| matches(r, true)) {
        return Ok(at);
    }
    if let Some(at) = rows.iter().position(|r| matches(r, false)) {
        return Ok(at);
    }

    Err(format!("nothing in this folder matches \u{201c}{want}\u{201d}"))
}
