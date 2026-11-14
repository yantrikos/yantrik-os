//! Secrets that sit in the path of a URL (#1171), where an HTTP client's error prints them.
//!
//! No key name or prefix marks these, so each is matched by where it sits:
//! - a Telegram bot token, `https://api.telegram.org/bot<id>:<35 characters>/sendMessage`
//!   (the OS ships a Telegram channel);
//! - a Slack incoming-webhook URL, `hooks.slack.com/services/T…/B…/…`;
//! - a Discord webhook URL, `discord.com/api/webhooks/<id>/<token>`.
//!
//! The marker that names the thing stays, the secret goes.

/// A Slack or Discord webhook: whoever holds the URL can post to the channel.
const WEBHOOKS: &[&str] = &["hooks.slack.com/services/", "discord.com/api/webhooks/", "discordapp.com/api/webhooks/"];

/// What can follow a URL in a log line.
fn ends_url(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ',' | '?' | '#')
}

/// `text` with every path secret replaced.
pub(crate) fn redact_url_secrets(text: &str) -> String {
    redact_webhooks(&redact_bot_tokens(text))
}

/// `/bot<digits>:<30 or more of A-Za-z0-9_->` becomes `/bot<redacted>`.
fn redact_bot_tokens(text: &str) -> String {
    const MARK: &str = "/bot";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(MARK) {
        let (head, tail) = rest.split_at(at + MARK.len());
        out.push_str(head);
        let digits = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
        let secret = tail[digits..].strip_prefix(':').map_or(0, |after| {
            after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(after.len())
        });
        if digits >= 5 && secret >= 30 {
            out.push_str("<redacted>");
            rest = &tail[digits + 1 + secret..];
        } else {
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

fn redact_webhooks(text: &str) -> String {
    let mut out = text.to_string();
    for marker in WEBHOOKS {
        let mut next = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(marker) {
            let (head, tail) = rest.split_at(at + marker.len());
            next.push_str(head);
            let secret = tail.find(ends_url).unwrap_or(tail.len());
            if secret > 0 {
                next.push_str("<redacted>");
            }
            rest = &tail[secret..];
        }
        next.push_str(rest);
        out = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_telegram_bot_token_in_a_url_goes() {
        // Assembled here so this file holds no string a secret scanner takes for a bot token.
        let token = ["7123456789", ":", "AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw"].concat();
        let url = format!("error sending request for url (https://api.telegram.org/bot{token}/sendMessage)");
        assert_eq!(redact_url_secrets(&url), "error sending request for url (https://api.telegram.org/bot<redacted>/sendMessage)");
        // A path that merely starts with "bot" is not a token.
        assert_eq!(redact_url_secrets("GET /bots/12345:short"), "GET /bots/12345:short");
        assert_eq!(redact_url_secrets("/bot12345:short"), "/bot12345:short");
    }

    #[test]
    fn slack_and_discord_webhook_urls_go() {
        let slack = ["https://hooks.slack.com/services/", "T00000000/B00000000/", "XXXXXXXXXXXXXXXXXXXXXXXX"].concat();
        assert_eq!(redact_url_secrets(&format!("post to {slack} failed")), "post to https://hooks.slack.com/services/<redacted> failed");
        let discord = "https://discord.com/api/webhooks/1234567890/abcDEF_ghi-JKL?wait=true";
        assert_eq!(redact_url_secrets(discord), "https://discord.com/api/webhooks/<redacted>?wait=true");
        assert_eq!(redact_url_secrets("https://discord.com/api/users"), "https://discord.com/api/users");
    }
}
