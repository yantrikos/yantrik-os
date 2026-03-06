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
pub fn send_email(
    account: &EmailAccountConfig,
    to: &str,
    subject: &str,
    body: &str,
    in_reply_to: Option<&str>,
) -> Result<(), String> {
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
