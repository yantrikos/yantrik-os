//! The setup form: what was typed, turned into settings the service can act on.

use yantrik_ipc_contracts::email::AccountSettings;

/// The servers a named provider uses, as `(imap, imap_port, smtp, smtp_port)`.
///
/// The provider buttons on the setup screen hide the server fields, so a form filled in with
/// Gmail selected sends four empty strings for them. Something has to know these; it is here
/// rather than in the service because the service should be given settings, not a brand name.
pub fn servers_for(provider: &str) -> Option<(&'static str, u16, &'static str, u16)> {
    match provider.trim().to_lowercase().as_str() {
        "gmail" => Some(("imap.gmail.com", 993, "smtp.gmail.com", 587)),
        "outlook" => Some(("outlook.office365.com", 993, "smtp.office365.com", 587)),
        "yahoo" => Some(("imap.mail.yahoo.com", 993, "smtp.mail.yahoo.com", 587)),
        "icloud" => Some(("imap.mail.me.com", 993, "smtp.mail.me.com", 587)),
        _ => None,
    }
}

/// Turn what was typed into settings the service can act on, or say what is missing.
///
/// Every refusal here names a field. None of them contains the password — not the value, and not
/// a fragment of it — which is the rule the whole of this app's error handling is written to,
/// and which `tests/email-core` checks by building one with a sentinel and looking for it in
/// every string this module can produce.
#[allow(clippy::too_many_arguments)]
pub fn account_settings_from_form(
    email: &str,
    password: &str,
    display_name: &str,
    provider: &str,
    imap_server: &str,
    imap_port: &str,
    smtp_server: &str,
    smtp_port: &str,
) -> Result<AccountSettings, String> {
    let email = email.trim();
    if email.is_empty() {
        return Err("Enter the email address of the account.".to_string());
    }
    let (user, host) = email.split_once('@').ok_or_else(|| {
        format!("\u{201c}{email}\u{201d} is not an email address \u{2014} it has no @ in it.")
    })?;
    if user.is_empty() || host.is_empty() || !host.contains('.') {
        return Err(format!("\u{201c}{email}\u{201d} is not a complete email address."));
    }
    if password.is_empty() {
        return Err("Enter the password or app password for this account.".to_string());
    }

    let (imap_server, imap_port, smtp_server, smtp_port) = match servers_for(provider) {
        Some((imap, imap_p, smtp, smtp_p)) => {
            (imap.to_string(), imap_p, smtp.to_string(), smtp_p)
        }
        None => {
            let imap = imap_server.trim();
            let smtp = smtp_server.trim();
            if imap.is_empty() || smtp.is_empty() {
                return Err(
                    "Choose a provider, or choose Advanced and enter the IMAP and SMTP server \
                     names."
                        .to_string(),
                );
            }
            (
                imap.to_string(),
                parse_port(imap_port, "IMAP")?,
                smtp.to_string(),
                parse_port(smtp_port, "SMTP")?,
            )
        }
    };

    Ok(AccountSettings {
        email: email.to_string(),
        display_name: display_name.trim().to_string(),
        provider: provider.trim().to_string(),
        imap_server,
        imap_port,
        smtp_server,
        smtp_port,
        password: password.to_string(),
    })
}

fn parse_port(text: &str, which: &str) -> Result<u16, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("Enter the {which} port."));
    }
    text.parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| format!("\u{201c}{text}\u{201d} is not a {which} port number."))
}

/// What the setup screen says once both halves have been tried.
///
/// Both are reported whichever way each went: an account whose IMAP works and whose SMTP does
/// not can read mail and cannot send it, and "Connected!" for that is a smaller version of the
/// same fabrication as "Synced".
pub fn test_summary(imap_ok: bool, imap: &str, smtp_ok: bool, smtp: &str) -> String {
    match (imap_ok, smtp_ok) {
        (true, true) => format!("Signed in. {imap} \u{00b7} {smtp}"),
        (true, false) => format!("Mail can be read but not sent. {smtp}"),
        (false, true) => format!("Mail can be sent but not read. {imap}"),
        (false, false) => format!("{imap} \u{00b7} {smtp}"),
    }
}

/// The sentence the setup screen carries under the password field.
///
/// It is a function rather than a line of Slint so that it changes with the service's own answer:
/// the day a secret store exists, `secrets_are_plaintext` goes false and this stops claiming
/// something that is no longer true.
pub fn password_storage_note(config_path: &str, plaintext: bool) -> String {
    if plaintext {
        format!(
            "This password is stored in clear text in {config_path}, readable only by you \
             (mode 0600). This machine has no secret store the mail service can reach yet."
        )
    } else {
        format!("This password is stored by the mail service, which reads {config_path}.")
    }
}
