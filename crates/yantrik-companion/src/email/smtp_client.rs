//! SMTP client — send emails via SMTP + STARTTLS.

use lettre::{
    Message, SmtpTransport, Transport,
    message::{header::ContentType, Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::{Credentials, Mechanism},
};
use crate::config::EmailAccountConfig;
use super::resolve_smtp_server;

/// Send an email (multipart/alternative with plain text + HTML).
/// Supports both password and XOAUTH2 authentication.
///
/// **Asks the person first**, in every mode but their full bypass (the desktop's own card, the
/// shell's approvals module): a send is outward and cannot be undone. On VM 520 the companion
/// replied to three emails from the person's account with no card in `auto` and in `ask`, because
/// nothing between the tool and this function asked. Every email tool that sends ends here, so the
/// question is asked here, once, before anything is built or dialled; `tests::` pins that it comes
/// first and that no other file speaks SMTP.
pub fn send_email(
    account: &EmailAccountConfig,
    to: &str,
    subject: &str,
    body: &str,
    in_reply_to: Option<&str>,
) -> Result<(), String> {
    yantrik_companion_tools::outward::ask_person_before_sending(&yantrik_companion_tools::outward::OutgoingMail {
        account: &account.email,
        to: &[to.to_string()],
        cc: &[],
        bcc: &[],
        subject,
        body,
    })?;
    let (server, port) = resolve_smtp_server(account);
    if server.is_empty() {
        return Err("No SMTP server configured".to_string());
    }

    let from_mailbox: Mailbox = account.email.parse()
        .map_err(|e| format!("Invalid from address: {}", e))?;
    let to_mailbox: Mailbox = to.parse()
        .map_err(|e| format!("Invalid to address: {}", e))?;

    let mut builder = Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(subject);

    if let Some(reply_id) = in_reply_to {
        let in_reply_to_header: lettre::message::header::InReplyTo = reply_id.to_string().into();
        builder = builder.header(in_reply_to_header);
    }

    // Build multipart/alternative with plain text + HTML
    let html_body = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8">
<style>body {{ font-family: sans-serif; font-size: 14px; line-height: 1.6; color: #333; }}</style>
</head><body>{}</body></html>"#,
        body.replace('\n', "<br>\n")
    );

    let email = builder
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(body.to_string())
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html_body)
                )
        )
        .map_err(|e| format!("Build email failed: {}", e))?;

    let is_oauth = account.auth_method.as_deref() == Some("oauth2");

    let mailer = if is_oauth {
        let token = account.oauth_access_token.as_deref().unwrap_or("");
        if token.is_empty() {
            return Err("OAuth2 account has no access token".to_string());
        }
        let creds = Credentials::new(account.email.clone(), token.to_string());
        SmtpTransport::starttls_relay(&server)
            .map_err(|e| format!("SMTP connect failed: {}", e))?
            .port(port)
            .credentials(creds)
            .authentication(vec![Mechanism::Xoauth2])
            .build()
    } else {
        let creds = Credentials::new(account.email.clone(), account.password.clone());
        SmtpTransport::starttls_relay(&server)
            .map_err(|e| format!("SMTP connect failed: {}", e))?
            .port(port)
            .credentials(creds)
            .build()
    };

    mailer.send(&email)
        .map_err(|e| format!("Send failed: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> EmailAccountConfig {
        // No SMTP server: were the question not asked first, the answer would be
        // "No SMTP server configured".
        serde_json::from_value(serde_json::json!({
            "name": "Ada", "email": "ada@example.com", "provider": "imap", "password": "not-a-real-password"
        }))
        .unwrap()
    }

    /// With no desktop to ask, nothing is sent: an unreachable shell is a no.
    #[test]
    fn a_send_with_nobody_to_ask_sends_nothing() {
        let err = send_email(&account(), "priya@example.com", "Hi", "Hello", None).unwrap_err();
        assert!(err.to_lowercase().contains("nothing was sent"), "{err}");
        assert!(!err.contains("SMTP server"), "the question was not asked first: {err}");
    }

    /// The question is the first thing `send_email` does, and no other file in the companion
    /// speaks SMTP, so a new tool cannot send around it.
    #[test]
    fn the_question_comes_first_and_no_other_file_sends() {
        let source = include_str!("smtp_client.rs");
        let body = source.split("pub fn send_email(").nth(1).expect("send_email");
        let asked = body.find("ask_person_before_sending").expect("send_email asks");
        let dialled = body.find("resolve_smtp_server").expect("send_email dials");
        assert!(asked < dialled, "send_email builds or dials before it asks");

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("email/smtp_client.rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let production = text.split("#[cfg(test)]").next().unwrap_or(&text);
                    for forbidden in ["lettre::", "SmtpTransport"] {
                        assert!(!production.contains(forbidden), "{} speaks SMTP ({forbidden}) outside smtp_client", path.display());
                    }
                }
            }
        }
    }
}
