//! Email's rules, tested without a mailbox, a socket or a desktop.
//!
//! The faults these cover were each invisible from one side alone. The app read a dead service as
//! an unconfigured machine, so an audit of this OS concluded it had no mail account when what it
//! had was a service nothing ever started. `open_message which=1` answered `nothing in this
//! folder matches ""`, because a JSON number read through `as_str()` is the empty string. Closing
//! the composer discarded what was in it. And a password typed into the setup form had to be kept
//! out of every sentence this app can produce, which is the kind of rule that only holds if
//! something checks it.
//!
//! The September rewrite left "Sign in with Google" off the screen rather than wired, because the
//! only OAuth flow in the tree minted the wrong scope into the wrong store. There is a real one
//! now, and it brought its own class of thing that is invisible from either side: a PKCE
//! challenge that is not a hash of anything still looks like a challenge, a callback parser that
//! does not check `state` still returns a code, an access token with no expiry beside it still
//! signs in — until the hour is up. `google.rs` is the fourth module here for that reason.
//!
//! Four modules are included directly, all of them free of Slint, sockets and the network:

/// The app's side: which of three states it is in, which message a caller means, the draft, and
/// what the setup form makes of what was typed.
#[path = "../../apps/email/src/state.rs"]
pub mod state;

/// The service's side: where accounts live, how they are picked, and how they are written.
#[path = "../../services/email-service/src/accounts.rs"]
pub mod accounts;

/// The service's side: turning a mail server's refusal into a sentence that names it.
#[path = "../../services/email-service/src/connect.rs"]
pub mod connect;

/// The service's side: everything about a Google sign-in that is a decision rather than a socket.
/// The flow itself lives in `oauth.rs`, which is not here because all of it opens something.
#[path = "../../services/email-service/src/envelope.rs"]
pub mod envelope;

#[path = "../../services/email-service/src/google.rs"]
pub mod google;

#[cfg(test)]
mod tests {
    use super::accounts::{self, Account};
    use super::connect::{self, Attempt};
    use super::google;
    use super::state::{
        self, Counted, Draft, FolderCounts, GoogleOutcome, MailState, MessageRow, Triage,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yantrik_ipc_contracts::email::{
        without_secret, without_secrets, AccountSettings, AccountsResult, EmailAccountSummary,
        EmailFolder, OAuthBeginResult, OAuthStatus,
    };

    static ID: AtomicUsize = AtomicUsize::new(0);

    /// The password used everywhere below. Nothing this app produces may contain it, and the
    /// last block of tests is nothing but looking for it.
    const SENTINEL: &str = "hunter2-SENTINEL-do-not-print";

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "email-test-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }

        /// The accounts file, under a directory the store has to create for itself: the real one
        /// is `~/.config/yantrik/email.json` and `~/.config/yantrik` may not exist.
        fn config(&self) -> PathBuf {
            self.0.join("config/yantrik/email.json")
        }

        fn draft(&self) -> PathBuf {
            self.0.join("share/yantrik/email/draft.json")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn settings() -> AccountSettings {
        AccountSettings {
            email: "someone@example.com".into(),
            display_name: "Someone".into(),
            provider: "advanced".into(),
            imap_server: "imap.example.com".into(),
            imap_port: 993,
            smtp_server: "smtp.example.com".into(),
            smtp_port: 587,
            password: SENTINEL.into(),
        }
    }

    fn summary(email: &str) -> EmailAccountSummary {
        EmailAccountSummary {
            id: accounts::id_for(email),
            email: email.into(),
            display_name: String::new(),
            provider: "gmail".into(),
            imap_server: "imap.gmail.com".into(),
            imap_port: 993,
            smtp_server: "smtp.gmail.com".into(),
            smtp_port: 587,
            uses_oauth: false,
        }
    }

    fn rows() -> Vec<MessageRow> {
        vec![
            MessageRow::new("Launch review", "Priya Raman", "priya@lumen.dev"),
            MessageRow::new("Invoice R0093 for September", "Hetzner", "billing@hetzner.com"),
            MessageRow::new("Re: Launcher grid", "Ananya Sen", "ananya@lumen.dev"),
        ]
    }

    // ── The three states ─────────────────────────────────────────────
    //
    // The whole point of this file. A dead service and an unconfigured machine used to be one
    // picture, and an audit read that picture as the machine having no account.

    #[test]
    fn a_service_that_could_not_be_reached_is_not_an_absent_account() {
        let s = state::decide(Err("could not start the email service: Binary not found".into()));
        assert_eq!(s.service_word(), "unreachable");
        // Not Some(false). With nothing answering, the app has not been told either way.
        assert_eq!(s.has_account(), None);
        assert!(s.notice().contains("Binary not found"));
        assert!(!s.summary().to_lowercase().contains("no account"));
    }

    #[test]
    fn a_service_that_answered_with_no_accounts_says_so() {
        let s = state::decide(Ok(AccountsResult {
            accounts: Vec::new(),
            config_path: "/home/p/.config/yantrik/email.json".into(),
            secrets_are_plaintext: true,
            google_sign_in: Default::default(),
        }));
        assert_eq!(s.service_word(), "up");
        assert_eq!(s.has_account(), Some(false));
        // Nothing is wrong, so nothing is said.
        assert_eq!(s.notice(), "");
        // And the summary names the file, so a person is told where an account would go.
        assert!(s.summary().contains("/home/p/.config/yantrik/email.json"));
    }

    #[test]
    fn a_service_holding_an_account_is_ready() {
        let s = state::decide(Ok(AccountsResult {
            accounts: vec![summary("someone@example.com")],
            config_path: "/tmp/email.json".into(),
            secrets_are_plaintext: true,
            google_sign_in: Default::default(),
        }));
        assert_eq!(s.service_word(), "up");
        assert_eq!(s.has_account(), Some(true));
        assert_eq!(s.account_name(), "someone@example.com");
        assert_eq!(s.account_id(), "someone-example-com");
        assert_eq!(s.notice(), "");
    }

    #[test]
    fn the_three_states_are_three_different_sentences() {
        let down = state::decide(Err("no socket".into())).summary();
        let empty = state::decide(Ok(AccountsResult {
            accounts: Vec::new(),
            config_path: "/tmp/e.json".into(),
            secrets_are_plaintext: true,
            google_sign_in: Default::default(),
        }))
        .summary();
        let ready = state::decide(Ok(AccountsResult {
            accounts: vec![summary("a@b.com")],
            config_path: "/tmp/e.json".into(),
            secrets_are_plaintext: true,
            google_sign_in: Default::default(),
        }))
        .summary();
        assert_ne!(down, empty);
        assert_ne!(empty, ready);
        assert_ne!(down, ready);
    }

    #[test]
    fn an_unreachable_service_reports_no_account_store_and_no_account() {
        let s = MailState::Unreachable { reason: "refused".into() };
        assert_eq!(s.config_path(), "");
        assert_eq!(s.account_id(), "");
        assert_eq!(s.account_name(), "");
        assert!(!s.secrets_are_plaintext());
    }

    // ── Which message a caller means ─────────────────────────────────

    #[test]
    fn a_row_number_opens_that_row() {
        assert_eq!(state::resolve_which(&rows(), "1"), Ok(0));
        assert_eq!(state::resolve_which(&rows(), "3"), Ok(2));
        // Counting from one, the way `describe.messages` publishes it.
        assert_eq!(state::resolve_which(&rows(), "2"), Ok(1));
    }

    #[test]
    fn a_number_with_spaces_round_it_is_still_a_number() {
        assert_eq!(state::resolve_which(&rows(), "  2  "), Ok(1));
    }

    #[test]
    fn row_numbers_count_from_one() {
        for below in ["0", "-1"] {
            let e = state::resolve_which(&rows(), below).unwrap_err();
            assert!(e.contains("count from 1"), "{e}");
            assert!(!e.contains("\"\""), "{e}");
        }
    }

    #[test]
    fn a_number_is_a_row_number_and_is_not_then_tried_as_text() {
        // "9" is a substring of "Invoice R0093 for September". A caller asking for the ninth
        // message of a folder holding three must not be handed the second one.
        assert!(state::resolve_which(&rows(), "9").is_err());
        assert!(state::resolve_which(&rows(), "0093").is_err());
    }

    #[test]
    fn an_exact_subject_beats_a_partial_one() {
        let rows = vec![
            MessageRow::new("Launch review notes", "A", "a@x.com"),
            MessageRow::new("Launch", "B", "b@x.com"),
        ];
        assert_eq!(state::resolve_which(&rows, "Launch"), Ok(1));
    }

    #[test]
    fn a_partial_subject_a_sender_and_an_address_all_resolve() {
        assert_eq!(state::resolve_which(&rows(), "invoice"), Ok(1));
        assert_eq!(state::resolve_which(&rows(), "Ananya Sen"), Ok(2));
        assert_eq!(state::resolve_which(&rows(), "priya@lumen.dev"), Ok(0));
    }

    #[test]
    fn a_number_past_the_end_says_how_many_there_are() {
        let e = state::resolve_which(&rows(), "9").unwrap_err();
        assert!(e.contains("no message 9"), "{e}");
        assert!(e.contains('3'), "{e}");
        assert!(!e.contains("\"\""), "{e}");
    }

    #[test]
    fn asking_for_row_one_of_an_empty_folder_is_refused_without_a_quoted_nothing() {
        // The exact call the probe makes on a machine with no mail. It used to answer
        // `nothing in this folder matches ""`, which reports a search for nothing.
        let e = state::resolve_which(&[], "1").unwrap_err();
        assert!(e.contains("no message 1"), "{e}");
        assert!(e.contains("none"), "{e}");
        assert!(!e.contains("\"\""), "{e}");
    }

    #[test]
    fn text_that_matches_nothing_names_what_was_asked_for() {
        let e = state::resolve_which(&rows(), "zebra").unwrap_err();
        assert!(e.contains("zebra"), "{e}");
        assert!(!e.contains("\"\""), "{e}");
    }

    #[test]
    fn an_empty_which_is_refused_without_quoting_it() {
        for empty in ["", "   "] {
            let e = state::resolve_which(&rows(), empty).unwrap_err();
            assert!(e.contains("row number"), "{e}");
            assert!(!e.contains("\"\""), "{e}");
            assert!(!e.contains("\u{201c}\u{201d}"), "{e}");
        }
    }

    // ── The triage tabs ──────────────────────────────────────────────

    #[test]
    fn the_triage_tabs_are_three_and_filter_what_is_in_hand() {
        assert_eq!(Triage::from_index(0), Some(Triage::All));
        assert_eq!(Triage::from_index(1), Some(Triage::Unread));
        assert_eq!(Triage::from_index(2), Some(Triage::Flagged));
        // There is no fourth. Priority was a tab over a quality no message carries.
        assert_eq!(Triage::from_index(3), None);
        assert_eq!(Triage::from_index(-1), None);
    }

    #[test]
    fn each_tab_keeps_what_it_says_it_keeps() {
        // (is_read, is_flagged)
        assert!(Triage::All.keeps(true, false));
        assert!(Triage::All.keeps(false, true));
        assert!(Triage::Unread.keeps(false, false));
        assert!(!Triage::Unread.keeps(true, true));
        assert!(Triage::Flagged.keeps(true, true));
        assert!(!Triage::Flagged.keeps(false, false));
    }

    #[test]
    fn a_tab_index_survives_the_round_trip() {
        for t in [Triage::All, Triage::Unread, Triage::Flagged] {
            assert_eq!(Triage::from_index(t.index()), Some(t));
        }
    }

    // ── What the header says about the open folder ───────────────────
    //
    // "Email — INBOX, 9 unread of 21" over a folder list saying INBOX held 35 with none unread:
    // the header counted the page in hand, the list carried the server's count, and one reply
    // gave two answers to one question (#74, #123). The header now reads the list.

    const REFUSED: &str = "the mail server did not report it: No Response: STATUS failed";

    fn listed() -> Vec<EmailFolder> {
        vec![
            EmailFolder::counted("INBOX", 12, 35),
            EmailFolder::counted("[Gmail]/Spam", 29, 29),
            EmailFolder::counted("[Gmail]/Sent Mail", 0, 5),
            EmailFolder::uncounted("Archive", REFUSED),
            EmailFolder::counted("Drafts", 0, 0),
        ]
    }

    #[test]
    fn the_header_and_the_folder_list_are_the_same_numbers() {
        // One page of the inbox is in hand — 21 rows, 9 of them unread — and the list says the
        // folder holds 35, 12 unread. The header says what the list says.
        let counts = Counted::of(&listed(), "INBOX", 9, 21);
        assert_eq!(counts, Counted::Known(FolderCounts { unread: 12, total: 35 }));
        let line = state::folder_summary("INBOX", &counts, None);
        assert_eq!(line, "Email — INBOX, 12 unread of 35");
        assert!(!line.contains("21"), "{line} counts the page, not the folder");
    }

    #[test]
    fn the_folder_is_found_however_it_was_capitalised() {
        assert_eq!(Counted::of(&listed(), "inbox", 0, 0).known().map(|c| c.total), Some(35));
    }

    #[test]
    fn a_folder_the_server_did_not_list_is_counted_from_what_is_in_hand() {
        // No entry, so no server count, and nothing in the list for the header to contradict.
        let counts = Counted::of(&listed(), "Receipts", 2, 7);
        assert_eq!(counts, Counted::Known(FolderCounts { unread: 2, total: 7 }));
    }

    // ── A folder the server would not count ──────────────────────────
    //
    // The list wrote `0/0` for a folder whose STATUS the server refused or never answered, and
    // the header built from it said "0 unread of 0": a transient failure presented as a fact
    // about the mailbox, and the same fact an empty folder presents (#131).

    #[test]
    fn a_folder_the_server_would_not_count_is_not_an_empty_folder() {
        // The list has Archive, uncounted, and 7 rows of it are in hand. The header does not
        // say "0 unread of 0", and it does not offer the page in hand as the server's answer
        // either: the server was asked and did not say.
        let counts = Counted::of(&listed(), "Archive", 2, 7);
        assert_eq!(counts, Counted::Unavailable { reason: REFUSED.into() });
        assert_eq!(counts.known(), None);
        let line = state::folder_summary("Archive", &counts, None);
        assert!(!line.contains("0 unread of 0"), "{line} reports a refusal as an empty folder");
        assert!(line.contains("counts unavailable"), "{line} does not say the counts are missing");
        assert!(line.contains("STATUS failed"), "{line} does not say why");
    }

    #[test]
    fn a_folder_the_server_counted_as_empty_is_empty() {
        // Zero from the server is zero; only zero invented for a missing answer is the defect.
        let counts = Counted::of(&listed(), "Drafts", 0, 0);
        assert_eq!(counts, Counted::Known(FolderCounts { unread: 0, total: 0 }));
        assert_eq!(state::folder_summary("Drafts", &counts, None), "Email — Drafts, 0 unread of 0");
    }

    #[test]
    fn counts_that_were_never_read_stay_unread_after_a_change() {
        // A message in an uncounted folder was read. One fewer than "not known" is still not
        // known; the header must not turn that into "-1 unread of -1" or into "0 unread of 0".
        let counts = Counted::Unavailable { reason: REFUSED.into() };
        let after = counts.clone().after(|c| c.after_read_change(false, true));
        assert_eq!(after, counts);
        let known = Counted::Known(FolderCounts { unread: 12, total: 35 });
        assert_eq!(
            known.after(|c| c.after_removal(false)),
            Counted::Known(FolderCounts { unread: 11, total: 34 })
        );
    }

    #[test]
    fn the_wire_says_uncounted_as_null_and_a_reason_not_as_zeros() {
        // What `email.list_folders` sends and `describe` prints: `counts: null` with a reason,
        // and never `{"unread": 0, "total": 0}` for a folder that was not counted.
        let wire = serde_json::to_value(EmailFolder::uncounted("Archive", REFUSED)).unwrap();
        assert_eq!(wire["counts"], serde_json::Value::Null);
        assert_eq!(wire["reason"], REFUSED);
        let wire = serde_json::to_value(EmailFolder::counted("Drafts", 0, 0)).unwrap();
        assert_eq!(wire["counts"], serde_json::json!({ "unread": 0, "total": 0 }));
        assert!(wire.get("reason").is_none(), "a counted folder has no reason: {wire}");
        // And it reads back as it was sent.
        let back: EmailFolder = serde_json::from_value(wire).unwrap();
        assert_eq!(back, EmailFolder::counted("Drafts", 0, 0));
    }

    #[test]
    fn reading_a_message_takes_one_off_the_folders_unread() {
        let counts = FolderCounts { unread: 12, total: 35 };
        assert_eq!(counts.after_read_change(false, true), FolderCounts { unread: 11, total: 35 });
        assert_eq!(counts.after_read_change(true, false), FolderCounts { unread: 13, total: 35 });
        // Marking read what was read already changes nothing.
        assert_eq!(counts.after_read_change(true, true), counts);
        assert_eq!(counts.after_read_change(false, false), counts);
    }

    #[test]
    fn a_message_that_leaves_the_folder_leaves_both_counts() {
        let counts = FolderCounts { unread: 12, total: 35 };
        assert_eq!(counts.after_removal(false), FolderCounts { unread: 11, total: 34 });
        assert_eq!(counts.after_removal(true), FolderCounts { unread: 12, total: 34 });
        assert_eq!(counts.after_arrival(false), FolderCounts { unread: 13, total: 36 });
        assert_eq!(counts.after_arrival(true), FolderCounts { unread: 12, total: 36 });
    }

    #[test]
    fn a_count_does_not_go_below_nothing() {
        // The list said none unread and a row in hand was unread anyway — an older service, or
        // a folder that changed under us. The header must not say -1.
        let counts = FolderCounts { unread: 0, total: 1 };
        assert_eq!(counts.after_read_change(false, true).unread, 0);
        assert_eq!(counts.after_removal(false), FolderCounts { unread: 0, total: 0 });
        assert_eq!(counts.after_removal(true).total, 0);
    }

    #[test]
    fn with_a_search_on_the_header_describes_the_results_not_the_folder() {
        let counts = Counted::Known(FolderCounts { unread: 12, total: 35 });
        assert_eq!(
            state::folder_summary("INBOX", &counts, Some(("invoice", 3))),
            "Email — INBOX, 3 results for \u{201c}invoice\u{201d}"
        );
        assert_eq!(
            state::folder_summary("INBOX", &counts, Some(("invoice", 1))),
            "Email — INBOX, 1 result for \u{201c}invoice\u{201d}"
        );
    }

    // ── The draft ────────────────────────────────────────────────────

    #[test]
    fn a_draft_survives_being_written_and_read_back() {
        let f = Fixture::new();
        let draft = Draft {
            to: "priya@lumen.dev".into(),
            cc: "ananya@lumen.dev".into(),
            bcc: String::new(),
            subject: "Re: launch".into(),
            body: "Half a sentence and then the".into(),
        };
        state::save_draft(&f.draft(), &draft).unwrap();
        assert_eq!(state::load_draft(&f.draft()).unwrap(), Some(draft));
    }

    #[test]
    fn the_draft_directory_is_made_if_it_is_not_there() {
        let f = Fixture::new();
        assert!(!f.draft().parent().unwrap().exists());
        state::save_draft(&f.draft(), &Draft { body: "x".into(), ..Draft::default() }).unwrap();
        assert!(f.draft().exists());
    }

    #[test]
    fn no_draft_file_is_no_draft_and_not_an_error() {
        let f = Fixture::new();
        assert_eq!(state::load_draft(&f.draft()).unwrap(), None);
    }

    #[test]
    fn a_draft_of_nothing_is_not_a_draft() {
        let f = Fixture::new();
        let blank = Draft { to: "  ".into(), ..Draft::default() };
        assert!(blank.is_empty());
        state::save_draft(&f.draft(), &blank).unwrap();
        assert_eq!(state::load_draft(&f.draft()).unwrap(), None);
    }

    #[test]
    fn clearing_a_draft_that_is_not_there_is_not_an_error() {
        let f = Fixture::new();
        state::clear_draft(&f.draft()).unwrap();
        state::save_draft(&f.draft(), &Draft { body: "x".into(), ..Draft::default() }).unwrap();
        state::clear_draft(&f.draft()).unwrap();
        assert_eq!(state::load_draft(&f.draft()).unwrap(), None);
    }

    #[test]
    fn a_draft_file_that_is_not_a_draft_is_an_error_rather_than_a_shrug() {
        let f = Fixture::new();
        std::fs::create_dir_all(f.draft().parent().unwrap()).unwrap();
        std::fs::write(f.draft(), "{ this is not json").unwrap();
        let e = state::load_draft(&f.draft()).unwrap_err();
        assert!(e.contains("not a draft"), "{e}");
    }

    // ── The setup form ───────────────────────────────────────────────

    #[test]
    fn a_provider_fills_in_its_own_servers() {
        let s = state::account_settings_from_form(
            "someone@gmail.com",
            SENTINEL,
            "Someone",
            "gmail",
            "",
            "993",
            "",
            "587",
        )
        .unwrap();
        assert_eq!(s.imap_server, "imap.gmail.com");
        assert_eq!(s.smtp_server, "smtp.gmail.com");
        assert_eq!(s.imap_port, 993);
        assert_eq!(s.smtp_port, 587);
    }

    #[test]
    fn every_named_provider_has_servers() {
        for provider in ["gmail", "outlook", "yahoo", "icloud"] {
            assert!(state::servers_for(provider).is_some(), "{provider}");
        }
        assert!(state::servers_for("advanced").is_none());
        assert!(state::servers_for("").is_none());
    }

    #[test]
    fn advanced_takes_what_was_typed() {
        let s = state::account_settings_from_form(
            "someone@example.com",
            SENTINEL,
            "",
            "advanced",
            " mail.example.com ",
            "1993",
            "smtp.example.com",
            "2587",
        )
        .unwrap();
        assert_eq!(s.imap_server, "mail.example.com");
        assert_eq!(s.imap_port, 1993);
        assert_eq!(s.smtp_port, 2587);
    }

    #[test]
    fn advanced_with_no_servers_is_refused_in_words() {
        let e = state::account_settings_from_form(
            "someone@example.com",
            SENTINEL,
            "",
            "advanced",
            "",
            "993",
            "",
            "587",
        )
        .unwrap_err();
        assert!(e.contains("Advanced"), "{e}");
    }

    #[test]
    fn an_address_that_is_not_one_is_refused() {
        for bad in ["", "someone", "someone@", "@example.com", "someone@localhost"] {
            assert!(
                state::account_settings_from_form(
                    bad, SENTINEL, "", "gmail", "", "993", "", "587"
                )
                .is_err(),
                "{bad} was accepted"
            );
        }
    }

    #[test]
    fn a_form_with_no_password_is_refused_rather_than_saved() {
        let e =
            state::account_settings_from_form("a@b.com", "", "", "gmail", "", "993", "", "587")
                .unwrap_err();
        assert!(e.to_lowercase().contains("password"), "{e}");
    }

    #[test]
    fn a_port_that_is_not_a_number_names_the_field() {
        let e = state::account_settings_from_form(
            "a@b.com", SENTINEL, "", "advanced", "i.b.com", "nine", "s.b.com", "587",
        )
        .unwrap_err();
        assert!(e.contains("IMAP"), "{e}");
        let e = state::account_settings_from_form(
            "a@b.com", SENTINEL, "", "advanced", "i.b.com", "993", "s.b.com", "0",
        )
        .unwrap_err();
        assert!(e.contains("SMTP"), "{e}");
    }

    #[test]
    fn both_halves_of_a_connection_test_are_reported() {
        let one_sided = state::test_summary(true, "IMAP ok", false, "SMTP refused");
        assert!(one_sided.contains("SMTP refused"), "{one_sided}");
        assert!(one_sided.to_lowercase().contains("not sent") || one_sided.contains("not sent"));
        let other = state::test_summary(false, "IMAP refused", true, "SMTP ok");
        assert!(other.contains("IMAP refused"), "{other}");
        let both = state::test_summary(true, "IMAP ok", true, "SMTP ok");
        assert!(both.starts_with("Signed in"), "{both}");
    }

    #[test]
    fn the_storage_note_says_plainly_what_happens_to_the_password() {
        let note = state::password_storage_note("/home/p/.config/yantrik/email.json", true);
        assert!(note.contains("clear text"), "{note}");
        assert!(note.contains("0600"), "{note}");
        assert!(note.contains("/home/p/.config/yantrik/email.json"), "{note}");
        // And stops saying it the day it stops being true.
        let other = state::password_storage_note("/tmp/e.json", false);
        assert!(!other.contains("clear text"), "{other}");
    }

    // ── Where accounts live ──────────────────────────────────────────

    #[test]
    fn an_account_survives_being_written_and_read_back() {
        let f = Fixture::new();
        let mut all = accounts::load(&f.config()).unwrap();
        assert!(all.is_empty());
        let id = accounts::upsert(&mut all, &settings());
        accounts::save(&f.config(), &all).unwrap();

        let back = accounts::load(&f.config()).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, id);
        assert_eq!(back[0].email, "someone@example.com");
        assert_eq!(back[0].imap_server, "imap.example.com");
        assert_eq!(back[0].password, SENTINEL);
    }

    #[test]
    fn saving_the_same_address_twice_edits_one_account() {
        let f = Fixture::new();
        let mut all = Vec::new();
        accounts::upsert(&mut all, &settings());
        let changed = AccountSettings { imap_server: "imap2.example.com".into(), ..settings() };
        accounts::upsert(&mut all, &changed);
        accounts::save(&f.config(), &all).unwrap();

        let back = accounts::load(&f.config()).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].imap_server, "imap2.example.com");
    }

    #[test]
    fn a_missing_accounts_file_is_no_accounts_and_not_an_error() {
        let f = Fixture::new();
        assert_eq!(accounts::load(&f.config()).unwrap().len(), 0);
    }

    #[test]
    fn an_accounts_file_that_is_broken_is_an_error_rather_than_no_account() {
        // This is the distinction the whole app turns on: a config with a typo in it must not
        // read as a machine nobody has configured.
        let f = Fixture::new();
        std::fs::create_dir_all(f.config().parent().unwrap()).unwrap();
        std::fs::write(f.config(), "{ not a list }").unwrap();
        let e = accounts::load(&f.config()).unwrap_err();
        assert!(e.contains("not a list of accounts"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn the_accounts_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let mut all = Vec::new();
        accounts::upsert(&mut all, &settings());
        accounts::save(&f.config(), &all).unwrap();

        let mode = std::fs::metadata(f.config()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the file holding a password was {mode:o}");
        let dir = std::fs::metadata(f.config().parent().unwrap()).unwrap().permissions().mode()
            & 0o777;
        assert_eq!(dir, 0o700, "the directory holding it was {dir:o}");
    }

    #[test]
    fn an_account_is_picked_by_id_by_address_or_by_being_the_only_one() {
        let mut all = Vec::new();
        accounts::upsert(&mut all, &settings());
        accounts::upsert(
            &mut all,
            &AccountSettings { email: "other@example.com".into(), ..settings() },
        );

        assert_eq!(accounts::pick(&all, Some("other-example-com")).unwrap().email, "other@example.com");
        assert_eq!(accounts::pick(&all, Some("OTHER@example.com")).unwrap().email, "other@example.com");
        // The bug: the app sent nothing, the service substituted "default", found no account
        // with that id, and answered "Unknown account: default" — which the app drew as having
        // no account at all.
        assert_eq!(accounts::pick(&all, Some("default")).unwrap().email, "someone@example.com");
        assert_eq!(accounts::pick(&all, None).unwrap().email, "someone@example.com");
        assert!(accounts::pick(&[], Some("default")).is_none());
    }

    #[test]
    fn an_id_is_derived_from_the_address_so_it_is_stable() {
        assert_eq!(accounts::id_for("Someone@Example.COM"), "someone-example-com");
        assert_eq!(accounts::id_for(""), "account");
        assert_eq!(accounts::id_for("@@@"), "account");
    }

    #[test]
    fn settings_that_cannot_work_are_refused_before_a_server_is_dialled() {
        accounts::refuse_bad_settings(&settings()).unwrap();
        for bad in [
            AccountSettings { email: "nope".into(), ..settings() },
            AccountSettings { password: String::new(), ..settings() },
            AccountSettings { imap_server: "  ".into(), ..settings() },
            AccountSettings { smtp_server: String::new(), ..settings() },
            AccountSettings { imap_port: 0, ..settings() },
        ] {
            assert!(accounts::refuse_bad_settings(&bad).is_err());
        }
    }

    #[test]
    fn a_summary_carries_no_secret_at_all() {
        let mut all = Vec::new();
        accounts::upsert(&mut all, &settings());
        let s = all[0].summary();
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains(SENTINEL), "{json}");
        assert!(!json.contains("password"), "{json}");
        assert_eq!(accounts::summaries(&all).len(), 1);
    }

    // ── Naming a connection failure ──────────────────────────────────

    fn imap() -> Attempt<'static> {
        Attempt::new("IMAP", "imap.example.com", 993)
    }

    #[test]
    fn a_name_that_does_not_resolve_is_said_as_that() {
        let said = connect::name_failure(
            &imap(),
            "failed to lookup address information: Name or service not known",
            SENTINEL,
        );
        assert!(said.contains("no such host"), "{said}");
        assert!(said.contains("imap.example.com"), "{said}");
    }

    #[test]
    fn a_closed_port_is_said_as_that_and_names_the_port() {
        let said = connect::name_failure(&imap(), "Connection refused (os error 111)", SENTINEL);
        assert!(said.contains("refused"), "{said}");
        assert!(said.contains("993"), "{said}");
    }

    #[test]
    fn a_silent_server_is_a_timeout_not_a_generic_failure() {
        let said =
            connect::name_failure(&imap(), "connection timed out (os error 110)", SENTINEL);
        assert!(said.contains("timed out"), "{said}");
    }

    #[test]
    fn a_certificate_problem_is_said_as_tls() {
        let said = connect::name_failure(
            &imap(),
            "TLS handshake failed: certificate verify failed: self signed certificate",
            SENTINEL,
        );
        assert!(said.contains("TLS"), "{said}");
        assert!(said.contains("self signed"), "{said}");
    }

    #[test]
    fn a_refused_password_is_said_as_a_refused_sign_in() {
        // Gmail's actual reply to an account password where an app password is needed.
        let said = connect::name_failure(
            &imap(),
            "[AUTHENTICATIONFAILED] Invalid credentials (Failure)",
            SENTINEL,
        );
        assert!(said.contains("rejected"), "{said}");
        assert!(said.contains("AUTHENTICATIONFAILED"), "{said}");
    }

    #[test]
    fn a_smtp_refusal_names_smtp_and_its_port() {
        let said = connect::name_failure(
            &Attempt::new("SMTP", "smtp.example.com", 587),
            "535 5.7.8 Authentication credentials invalid",
            SENTINEL,
        );
        assert!(said.starts_with("SMTP smtp.example.com:587"), "{said}");
    }

    #[test]
    fn an_unfamiliar_reply_is_passed_through_in_the_servers_own_words() {
        let said = connect::name_failure(&imap(), "SERVERBUG: try again later", SENTINEL);
        assert!(said.contains("SERVERBUG"), "{said}");
    }

    #[test]
    fn a_paragraph_of_a_reply_becomes_one_line() {
        let said = connect::name_failure(
            &imap(),
            "Web login required.\nSee https://support.example.com/mail/answer/78754\n",
            SENTINEL,
        );
        assert!(!said.contains('\n'), "{said}");
        assert!(said.contains("Web login required"), "{said}");
    }

    #[test]
    fn a_very_long_reply_is_cut_rather_than_filling_the_strip() {
        let said = connect::name_failure(&imap(), &"x".repeat(1000), SENTINEL);
        assert!(said.chars().count() < 250, "{} chars", said.chars().count());
    }

    #[test]
    fn a_success_is_as_specific_as_a_failure() {
        let said = connect::name_success(&imap());
        assert!(said.contains("imap.example.com:993"), "{said}");
        assert!(said.contains("signed in"), "{said}");
    }

    // ── The password appears in nothing ──────────────────────────────
    //
    // The rule that only holds if something checks it. A mail server is free to quote back the
    // line it was sent, and one echo would put a credential in a notice, in `app.describe`, and
    // in the transcript a mind is reading.

    #[test]
    fn a_server_that_echoes_the_password_does_not_get_it_onto_the_screen() {
        let echoed = format!("BAD Invalid command: LOGIN someone@example.com {SENTINEL}");
        let said = connect::name_failure(&imap(), &echoed, SENTINEL);
        assert!(!said.contains(SENTINEL), "{said}");
        assert!(said.contains("<redacted>"), "{said}");
    }

    #[test]
    fn redaction_of_an_empty_secret_does_not_shred_the_message() {
        // `replace("", …)` splices the marker between every character.
        assert_eq!(without_secret("hello", ""), "hello");
    }

    #[test]
    fn the_debug_of_settings_is_not_a_way_to_print_a_password() {
        let printed = format!("{:?}", settings());
        assert!(!printed.contains(SENTINEL), "{printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
        // And the address is still there, so the redaction has not made it useless.
        assert!(printed.contains("someone@example.com"), "{printed}");
    }

    #[test]
    fn the_debug_of_a_stored_account_is_not_either() {
        let mut all = Vec::new();
        accounts::upsert(&mut all, &settings());
        let printed = format!("{:?}", all[0]);
        assert!(!printed.contains(SENTINEL), "{printed}");
    }

    #[test]
    fn an_oauth_token_is_redacted_the_same_way() {
        let account = Account {
            oauth_token: Some("ya29.SENTINEL-TOKEN".into()),
            use_oauth: true,
            ..Account::default()
        };
        let printed = format!("{account:?}");
        assert!(!printed.contains("ya29"), "{printed}");
    }

    #[test]
    fn nothing_the_setup_form_can_say_contains_the_password() {
        // Every refusal the form produces, built with the sentinel in the password field.
        let mut said: Vec<String> = Vec::new();
        for (email, password, provider, imap, imap_port, smtp, smtp_port) in [
            ("", SENTINEL, "gmail", "", "993", "", "587"),
            ("nope", SENTINEL, "gmail", "", "993", "", "587"),
            ("a@b", SENTINEL, "gmail", "", "993", "", "587"),
            ("a@b.com", "", "gmail", "", "993", "", "587"),
            ("a@b.com", SENTINEL, "advanced", "", "993", "", "587"),
            ("a@b.com", SENTINEL, "advanced", "i.b.com", "nine", "s.b.com", "587"),
            ("a@b.com", SENTINEL, "advanced", "i.b.com", "993", "s.b.com", ""),
        ] {
            match state::account_settings_from_form(
                email, password, "Name", provider, imap, imap_port, smtp, smtp_port,
            ) {
                Ok(settings) => said.push(format!("{settings:?}")),
                Err(e) => said.push(e),
            }
        }

        // Every state the app can be in, every sentence it can produce about one.
        for s in [
            MailState::Unreachable { reason: format!("connect: LOGIN x {SENTINEL}") },
            state::decide(Ok(AccountsResult {
                accounts: Vec::new(),
                config_path: "/tmp/e.json".into(),
                secrets_are_plaintext: true,
                google_sign_in: Default::default(),
            })),
            state::decide(Ok(AccountsResult {
                accounts: vec![summary("a@b.com")],
                config_path: "/tmp/e.json".into(),
                secrets_are_plaintext: true,
                google_sign_in: Default::default(),
            })),
        ] {
            said.push(s.summary());
            said.push(s.notice());
            said.push(s.account_name());
            said.push(s.config_path().to_string());
        }

        // And the two sentences the setup screen carries.
        said.push(state::test_summary(
            false,
            &connect::name_failure(&imap(), &format!("BAD LOGIN {SENTINEL}"), SENTINEL),
            false,
            &connect::name_failure(
                &Attempt::new("SMTP", "smtp.b.com", 587),
                &format!("535 rejected {SENTINEL}"),
                SENTINEL,
            ),
        ));
        said.push(state::password_storage_note("/tmp/e.json", true));

        // The one place the sentinel is allowed: the `Unreachable` reason above is a string this
        // test wrote itself, standing in for a transport error. It is redacted at the point a
        // server's words enter, not afterwards — so what is checked here is that nothing the
        // app *formats* adds one.
        for sentence in &said {
            if sentence.contains("connect: LOGIN") {
                continue;
            }
            assert!(!sentence.contains(SENTINEL), "a password reached: {sentence}");
        }
    }

    #[test]
    fn a_saved_account_is_json_and_nothing_else_reads_as_a_summary() {
        // The file on disk does hold the password — that is the decision recorded in
        // `design/email-2026-09-20.md` and in `accounts.rs`. What must never happen is that file
        // being confused with what the service answers with.
        let f = Fixture::new();
        let mut all = Vec::new();
        accounts::upsert(&mut all, &settings());
        accounts::save(&f.config(), &all).unwrap();

        let on_disk = std::fs::read_to_string(f.config()).unwrap();
        assert!(on_disk.contains(SENTINEL), "the service could not sign in with this account");

        let answered = serde_json::to_string(&AccountsResult {
            accounts: accounts::summaries(&all),
            config_path: f.config().display().to_string(),
            secrets_are_plaintext: true,
            google_sign_in: Default::default(),
        })
        .unwrap();
        assert!(!answered.contains(SENTINEL), "{answered}");
    }

    // ── The wire, as both ends build it ──────────────────────────────
    //
    // The payload tests. The calendar's two ends each spelled their own parameter names and
    // disagreed, so every listing failed while both files looked right on their own page.

    #[test]
    fn the_settings_the_form_builds_parse_as_the_service_parses_them() {
        let built = state::account_settings_from_form(
            "someone@gmail.com",
            SENTINEL,
            "Someone",
            "gmail",
            "",
            "993",
            "",
            "587",
        )
        .unwrap();
        let sent = serde_json::to_value(&built).unwrap();
        let received: AccountSettings = serde_json::from_value(sent).unwrap();
        accounts::refuse_bad_settings(&received).unwrap();
        assert_eq!(received.imap_server, "imap.gmail.com");
        assert_eq!(received.password, SENTINEL);
    }

    #[test]
    fn the_accounts_answer_parses_into_the_state_the_app_decides_from() {
        let mut all = Vec::new();
        accounts::upsert(&mut all, &settings());
        let answered = serde_json::to_value(AccountsResult {
            accounts: accounts::summaries(&all),
            config_path: "/tmp/e.json".into(),
            secrets_are_plaintext: true,
            google_sign_in: Default::default(),
        })
        .unwrap();
        let parsed: AccountsResult = serde_json::from_value(answered).unwrap();
        assert_eq!(state::decide(Ok(parsed)).has_account(), Some(true));
    }

    #[test]
    fn an_account_file_written_by_hand_in_the_old_shape_still_loads() {
        // The file predates this pass and deployments have one. It had no `display_name` and no
        // `provider`, and its id could be anything.
        let f = Fixture::new();
        std::fs::create_dir_all(f.config().parent().unwrap()).unwrap();
        std::fs::write(
            f.config(),
            r#"[{"id":"work","email":"a@b.com","password":"x",
                "imap_server":"imap.b.com","smtp_server":"smtp.b.com"}]"#,
        )
        .unwrap();
        let all = accounts::load(&f.config()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "work");
        assert_eq!(all[0].imap_port, 993);
        assert_eq!(all[0].smtp_port, 587);
        // And the app finds it, although its id is not the address slug.
        assert_eq!(accounts::pick(&all, Some("default")).unwrap().email, "a@b.com");
    }

    // ── Sign in with Google ──────────────────────────────────────────
    //
    // The button that was removed in September for calling a stub. What is checked here is the
    // half of the flow that is a decision rather than a socket, because that is the half where a
    // fault is invisible: a PKCE challenge that is not a hash of anything still looks like a
    // challenge, a callback parser that skips the `state` check still returns a code, and an
    // access token with no expiry stored beside it signs in perfectly — for an hour.
    //
    // Nothing here touches Google. A test that needed a live account, a rate limit and a person
    // at a browser is a test nobody runs.

    /// RFC 7636 appendix B, the worked example. The point of a published vector is that it
    /// catches the fallback the companion's helper has: when its `sha256sum` subprocess fails it
    /// returns base64 of the *verifier*, which is a valid-looking S256 challenge that is not a
    /// hash of anything, and a flow using it has PKCE's protection taken out with no symptom.
    const RFC_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const RFC_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    /// Stand-ins for the three Google credentials. None of them may appear in anything this code
    /// produces, exactly as the password sentinel may not.
    const ACCESS_SENTINEL: &str = "ya29.ACCESS-SENTINEL-do-not-print";
    const REFRESH_SENTINEL: &str = "1//REFRESH-SENTINEL-do-not-print";
    const CODE_SENTINEL: &str = "4/0AX4-CODE-SENTINEL-do-not-print";
    const CLIENT_SECRET_SENTINEL: &str = "GOCSPX-SECRET-SENTINEL";

    fn pkce_of(verifier: &str, state: &str) -> google::Pkce {
        google::Pkce {
            challenge: google::code_challenge(verifier),
            verifier: verifier.to_string(),
            state: state.to_string(),
        }
    }

    fn a_client() -> google::GoogleClient {
        google::GoogleClient {
            id: "1234.apps.googleusercontent.com".into(),
            secret: Some(CLIENT_SECRET_SENTINEL.into()),
            source: "a test".into(),
        }
    }

    #[test]
    fn the_code_challenge_is_a_real_sha256_of_the_verifier() {
        assert_eq!(google::code_challenge(RFC_VERIFIER), RFC_CHALLENGE);
        // And specifically not base64 of the verifier itself, which is the fallback next door.
        assert_ne!(
            google::code_challenge(RFC_VERIFIER),
            google::base64url(RFC_VERIFIER.as_bytes())
        );
    }

    #[test]
    fn the_challenge_is_base64url_with_no_padding() {
        let challenge = google::code_challenge("anything at all");
        assert!(!challenge.contains('='), "{challenge}");
        assert!(!challenge.contains('+'), "{challenge}");
        assert!(!challenge.contains('/'), "{challenge}");
    }

    #[test]
    fn two_sign_ins_do_not_share_a_verifier_or_a_state() {
        let one = google::Pkce::new().expect("this machine has no /dev/urandom");
        let two = google::Pkce::new().expect("this machine has no /dev/urandom");
        assert_ne!(one.verifier, two.verifier);
        assert_ne!(one.state, two.state);
        assert_eq!(one.challenge, google::code_challenge(&one.verifier));
        // Long enough to be worth having: RFC 7636 wants 43 characters minimum.
        assert!(one.verifier.len() >= 43, "{}", one.verifier.len());
    }

    #[test]
    fn the_consent_url_asks_for_the_scope_imap_actually_needs() {
        let pkce = pkce_of(RFC_VERIFIER, "STATE-1");
        let url = google::auth_url("client-1", &google::redirect_uri(41234), &pkce);

        // The whole reason this flow exists rather than the companion's: `gmail.readonly` is the
        // Gmail HTTP API and XOAUTH2 will not take it.
        assert!(url.contains("https%3A%2F%2Fmail.google.com%2F"), "{url}");
        assert!(!url.contains("gmail.readonly"), "{url}");
        // The address comes from Google rather than from a box a person typed into.
        assert!(url.contains("openid"), "{url}");
        assert!(url.contains("code_challenge_method=S256"), "{url}");
        assert!(url.contains(&format!("code_challenge={RFC_CHALLENGE}")), "{url}");
        assert!(url.contains("state=STATE-1"), "{url}");
        // Both, or Google returns no refresh token and the account dies in an hour.
        assert!(url.contains("access_type=offline"), "{url}");
        assert!(url.contains("prompt=consent"), "{url}");
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A41234"), "{url}");
        assert!(url.starts_with(google::AUTH_ENDPOINT), "{url}");
    }

    #[test]
    fn the_verifier_is_not_in_the_url_the_browser_is_given() {
        let pkce = pkce_of(RFC_VERIFIER, "STATE-1");
        let url = google::auth_url("client-1", &google::redirect_uri(1), &pkce);
        // The whole of PKCE: the challenge travels and the verifier does not.
        assert!(!url.contains(RFC_VERIFIER), "{url}");
    }

    #[test]
    fn the_redirect_is_loopback_and_nothing_else() {
        // Not `localhost`, which resolves through whatever the machine's hosts file says, and
        // not a public address. The socket is bound to 127.0.0.1 and this must name it.
        assert_eq!(google::redirect_uri(8080), "http://127.0.0.1:8080");
    }

    // ── What came back to the loopback socket ────────────────────────

    #[test]
    fn a_callback_with_the_right_state_is_the_code() {
        let request = "GET /?code=abc123&state=STATE-1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            google::parse_callback(request, "STATE-1"),
            google::Callback::Code("abc123".into())
        );
    }

    #[test]
    fn a_percent_encoded_code_arrives_decoded() {
        // Google's codes contain `/` and are percent-encoded in the query.
        let request = "GET /?code=4%2F0AX4&state=S HTTP/1.1\r\n\r\n";
        assert_eq!(google::parse_callback(request, "S"), google::Callback::Code("4/0AX4".into()));
    }

    #[test]
    fn declining_in_the_browser_is_said_as_declining() {
        let request = "GET /?error=access_denied&state=S HTTP/1.1\r\n\r\n";
        match google::parse_callback(request, "S") {
            google::Callback::Refused(why) => {
                assert!(why.to_lowercase().contains("declined"), "{why}");
                // And it says nothing was saved, because that is the question a person has.
                assert!(why.to_lowercase().contains("nothing was saved"), "{why}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_code_under_the_wrong_state_is_refused_and_the_code_is_not_quoted() {
        // Any page in any browser on this machine can GET 127.0.0.1:<port>?code=… . The state is
        // the only thing that says the reply belongs to the flow this service started.
        let request = format!("GET /?code={CODE_SENTINEL}&state=SOMEONE-ELSE HTTP/1.1\r\n\r\n");
        match google::parse_callback(&request, "OURS") {
            google::Callback::Refused(why) => {
                assert!(why.contains("did not match"), "{why}");
                assert!(!why.contains(CODE_SENTINEL), "the code reached a notice: {why}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_callback_with_no_state_at_all_is_refused() {
        let request = "GET /?code=abc123 HTTP/1.1\r\n\r\n";
        assert!(matches!(
            google::parse_callback(request, "OURS"),
            google::Callback::Refused(_)
        ));
    }

    #[test]
    fn a_browser_asking_for_a_favicon_does_not_end_the_sign_in() {
        // A flow that failed because the browser was tidy would be a sign-in that works on some
        // browsers and not others, for no reason anyone could find.
        for request in [
            "GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "GET / HTTP/1.1\r\n\r\n",
            "",
            "\r\n\r\n",
        ] {
            assert_eq!(
                google::parse_callback(request, "S"),
                google::Callback::Ignore,
                "{request:?}"
            );
        }
    }

    #[test]
    fn google_error_values_are_named_rather_than_echoed() {
        assert!(google::name_consent_failure("admin_policy_enforced").contains("administrator"));
        assert!(google::name_consent_failure("redirect_uri_mismatch").contains("Desktop app"));
        // An unfamiliar one is passed through in Google's own words rather than replaced with a
        // sentence that says nothing — the same rule `connect::name_failure` follows.
        assert!(google::name_consent_failure("some_new_thing").contains("some_new_thing"));
    }

    #[test]
    fn the_browser_page_escapes_what_it_is_given() {
        let page = google::browser_page("Not signed in", "<script>alert(1)</script>");
        assert!(!page.contains("<script>alert"), "{page}");
        assert!(page.contains("&lt;script&gt;"), "{page}");
    }

    // ── When a token has run out ─────────────────────────────────────

    #[test]
    fn an_access_token_with_no_expiry_beside_it_is_treated_as_spent() {
        // An account written before this service stored an expiry, or one whose token endpoint
        // did not say. Refreshing a token that was still good costs one HTTPS round trip; not
        // refreshing one that was not costs a sign-in failure that reads as a broken account.
        assert!(google::needs_refresh(None, 1_000_000));
    }

    #[test]
    fn a_token_with_an_hour_left_is_not_refreshed() {
        let now = 1_000_000;
        assert!(!google::needs_refresh(Some(now + 3600), now));
    }

    #[test]
    fn a_token_about_to_expire_is_refreshed_before_it_does() {
        let now = 1_000_000;
        // Inside the skew: it would expire between this check and the IMAP greeting, which a
        // person experiences as mail that stops working sometimes.
        assert!(google::needs_refresh(Some(now + 30), now));
        assert!(google::needs_refresh(Some(now), now));
        assert!(google::needs_refresh(Some(now - 1), now));
        // And just outside it is left alone.
        assert!(!google::needs_refresh(Some(now + google::EXPIRY_SKEW_SECS + 1), now));
    }

    #[test]
    fn an_expiry_is_stored_as_an_absolute_time() {
        let json = serde_json::json!({ "access_token": "a", "expires_in": 3599 });
        let tokens = google::tokens_from_json(&json, 1_700_000_000).unwrap();
        assert_eq!(tokens.expires_at, 1_700_003_599);
    }

    #[test]
    fn a_token_answer_with_no_access_token_is_an_error_rather_than_an_empty_one() {
        let json = serde_json::json!({ "expires_in": 3600 });
        assert!(google::tokens_from_json(&json, 0).is_err());
    }

    #[test]
    fn a_token_answer_with_no_expiry_lands_already_expired() {
        // Rather than a guessed 3600. The next use refreshes, which is a round trip; assuming an
        // hour would be a sign-in failure at the mail server instead.
        let json = serde_json::json!({ "access_token": "a" });
        let tokens = google::tokens_from_json(&json, 1_700_000_000).unwrap();
        assert!(google::needs_refresh(Some(tokens.expires_at), 1_700_000_000));
    }

    // ── A refusal from Google's token endpoint ───────────────────────

    #[test]
    fn a_revoked_refresh_token_is_named_and_is_not_called_an_auth_failure() {
        let body =
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#;
        let said = google::name_token_failure(400, body);
        assert!(said.contains("Google sign-in expired"), "{said}");
        assert!(said.contains("sign in again"), "{said}");
        // Not this. There is no password on the account and nothing in the settings to correct,
        // so sending someone to check credentials is sending them nowhere.
        assert!(!said.to_lowercase().contains("authentication failed"), "{said}");
        assert!(!said.to_lowercase().contains("check your password"), "{said}");
    }

    #[test]
    fn a_wrong_client_id_points_at_the_client_id() {
        let said = google::name_token_failure(401, r#"{"error":"invalid_client"}"#);
        assert!(said.contains("GOOGLE_CLIENT_ID"), "{said}");
    }

    #[test]
    fn a_refused_scope_points_at_the_scope() {
        let said = google::name_token_failure(400, r#"{"error":"invalid_scope"}"#);
        assert!(said.contains("mail.google.com"), "{said}");
    }

    #[test]
    fn an_unfamiliar_refusal_keeps_googles_own_words_and_the_status() {
        let said = google::name_token_failure(503, "backend unavailable, try later");
        assert!(said.contains("503"), "{said}");
        assert!(said.contains("backend unavailable"), "{said}");
    }

    #[test]
    fn a_very_long_refusal_is_cut_rather_than_pasted_into_a_notice() {
        let said = google::name_token_failure(400, &"x".repeat(4000));
        assert!(said.chars().count() < 400, "{}", said.chars().count());
    }

    #[test]
    fn a_rejected_token_at_the_mail_server_is_named_as_a_sign_in_to_redo() {
        // Gmail's own words when XOAUTH2 is refused. The password classifier would call this
        // "the sign-in was rejected", which is what a wrong password is called.
        let raw =
            format!("NO [AUTHENTICATIONFAILED] Invalid credentials (Failure) {ACCESS_SENTINEL}");
        let said = connect::name_oauth_failure(
            &Attempt::new("IMAP", "imap.gmail.com", 993),
            &raw,
            &[ACCESS_SENTINEL, REFRESH_SENTINEL],
        );
        assert!(said.contains("Google sign-in expired"), "{said}");
        assert!(said.contains("sign in again"), "{said}");
        assert!(!said.contains(ACCESS_SENTINEL), "the token reached a notice: {said}");
        assert!(said.contains("<redacted>"), "{said}");
    }

    #[test]
    fn everything_that_is_not_an_auth_failure_is_named_the_same_way_for_both_kinds_of_account() {
        let where_ = Attempt::new("IMAP", "imap.gmail.com", 993);
        let raw = "Connection refused (os error 111)";
        assert_eq!(
            connect::name_oauth_failure(&where_, raw, &["t"]),
            connect::name_failure_secrets(&where_, raw, &["t"]),
        );
    }

    // ── Which client id this machine signs in with ───────────────────

    #[test]
    fn the_environment_wins_over_the_file() {
        // The shell sets GOOGLE_CLIENT_ID from its own config before it starts any service, and
        // a service started on demand inherits it. A machine configured once there does not need
        // a second file.
        let chosen = google::choose_client(
            Some("from-env".into()),
            Some("secret-env".into()),
            Some(a_client()),
        )
        .unwrap();
        assert_eq!(chosen.id, "from-env");
        assert_eq!(chosen.secret.as_deref(), Some("secret-env"));
        assert!(chosen.source.contains("GOOGLE_CLIENT_ID"));
    }

    #[test]
    fn an_empty_environment_variable_is_not_a_client_id() {
        // An exported-but-empty GOOGLE_CLIENT_ID is what a shell script that read a missing key
        // produces, and treating it as configured means every sign-in fails at Google.
        let chosen = google::choose_client(Some("   ".into()), None, Some(a_client())).unwrap();
        assert_eq!(chosen.id, a_client().id);
    }

    #[test]
    fn with_neither_the_refusal_says_where_to_put_one() {
        let why = google::choose_client(None, None, None).unwrap_err();
        assert!(why.contains("GOOGLE_CLIENT_ID"), "{why}");
        assert!(why.contains("google-oauth.json"), "{why}");
    }

    #[test]
    fn a_client_id_file_that_is_not_there_is_not_an_error() {
        let f = Fixture::new();
        assert!(google::client_in_file(&f.0.join("nothing.json")).unwrap().is_none());
    }

    #[test]
    fn a_client_id_file_that_is_broken_is_an_error_rather_than_a_shrug() {
        // The same rule as the accounts file: a typo silently read as "nothing is configured" is
        // how a working setup comes to look like an absent one.
        let f = Fixture::new();
        let path = f.0.join("google-oauth.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        assert!(google::client_in_file(&path).is_err());

        std::fs::write(&path, r#"{"google_client_secret":"s"}"#).unwrap();
        let why = google::client_in_file(&path).unwrap_err();
        assert!(why.contains("google_client_id"), "{why}");
    }

    #[test]
    fn a_client_id_file_round_trips() {
        let f = Fixture::new();
        let path = f.0.join("google-oauth.json");
        std::fs::write(
            &path,
            r#"{"google_client_id":"abc.apps.googleusercontent.com",
                "google_client_secret":"GOCSPX-x"}"#,
        )
        .unwrap();
        let found = google::client_in_file(&path).unwrap().unwrap();
        assert_eq!(found.id, "abc.apps.googleusercontent.com");
        assert_eq!(found.secret.as_deref(), Some("GOCSPX-x"));
        assert!(found.source.contains("google-oauth.json"));
    }

    #[test]
    fn a_build_with_no_client_id_says_so_and_says_what_works_instead() {
        // The screen draws this instead of a button. "No button and no reason" is the state that
        // makes a person think the app is broken when the truth is that this build has no client.
        let told = google::availability(&google::choose_client(None, None, None));
        assert!(!told.available);
        assert!(told.note.contains("App Password"), "{}", told.note);
        assert!(told.note.contains("not available"), "{}", told.note);
    }

    #[test]
    fn a_build_with_a_client_id_names_where_it_came_from() {
        let told = google::availability(&Ok(a_client()));
        assert!(told.available);
        assert!(told.note.contains("a test"), "{}", told.note);
        // And never the secret.
        assert!(!told.note.contains(CLIENT_SECRET_SENTINEL), "{}", told.note);
    }

    // ── Which address signed in ──────────────────────────────────────

    #[test]
    fn the_address_comes_out_of_the_id_token() {
        // A JWT with an unsigned-looking signature: this code reads the payload and does not
        // verify, deliberately, because the token came back over TLS from Google's own endpoint
        // rather than from a browser.
        let payload = google::base64url(br#"{"email":"someone@gmail.com","email_verified":true}"#);
        let jwt = format!("header.{payload}.signature");
        assert_eq!(google::email_from_id_token(&jwt).as_deref(), Some("someone@gmail.com"));
    }

    #[test]
    fn an_id_token_that_says_nothing_useful_is_none_rather_than_a_guess() {
        let no_email = format!("h.{}.s", google::base64url(br#"{"sub":"12345"}"#));
        let not_an_address = format!("h.{}.s", google::base64url(br#"{"email":"nope"}"#));
        for bad in ["", "not-a-jwt", "a.b.c", no_email.as_str(), not_an_address.as_str()] {
            assert_eq!(google::email_from_id_token(bad), None, "{bad}");
        }
    }

    // ── What is written down ─────────────────────────────────────────

    #[test]
    fn a_google_account_is_stored_with_gmails_servers_and_no_password() {
        let mut all = Vec::new();
        let id = accounts::upsert_google(
            &mut all,
            "someone@gmail.com",
            ACCESS_SENTINEL,
            REFRESH_SENTINEL,
            1_700_000_000,
        );
        assert_eq!(id, "someone-gmail-com");
        let account = &all[0];
        assert!(account.use_oauth);
        assert_eq!(account.imap_server, "imap.gmail.com");
        assert_eq!(account.imap_port, 993);
        assert_eq!(account.smtp_server, "smtp.gmail.com");
        assert_eq!(account.smtp_port, 587);
        assert_eq!(account.oauth_token.as_deref(), Some(ACCESS_SENTINEL));
        assert_eq!(account.oauth_refresh_token.as_deref(), Some(REFRESH_SENTINEL));
        assert_eq!(account.oauth_expires_at, Some(1_700_000_000));
        // A credential kept after it stops being needed is a credential kept for no reason.
        assert_eq!(account.password, "");
    }

    #[test]
    fn signing_in_with_google_over_an_app_password_account_keeps_the_name_and_drops_the_password() {
        let mut all = Vec::new();
        let mut typed = settings();
        typed.email = "someone@gmail.com".into();
        typed.display_name = "Pranab".into();
        accounts::upsert(&mut all, &typed);
        assert_eq!(all.len(), 1);

        accounts::upsert_google(&mut all, "someone@gmail.com", "a", "r", 1);
        // One account, not two: the id is derived from the address either way.
        assert_eq!(all.len(), 1);
        // The name is a thing the person typed and a sign-in does not know it.
        assert_eq!(all[0].display_name, "Pranab");
        assert_eq!(all[0].password, "");
    }

    #[test]
    fn a_refresh_writes_the_tokens_and_touches_nothing_else() {
        let mut all = Vec::new();
        accounts::upsert_google(&mut all, "someone@gmail.com", "old-access", "old-refresh", 1);
        all[0].display_name = "Work".into();

        assert!(accounts::store_refreshed(
            &mut all,
            "someone-gmail-com",
            "new-access",
            "new-refresh",
            1_700_000_000
        ));
        assert_eq!(all[0].oauth_token.as_deref(), Some("new-access"));
        assert_eq!(all[0].oauth_refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(all[0].oauth_expires_at, Some(1_700_000_000));
        // A function that rebuilt the record would be one edit from resetting these on every poll.
        assert_eq!(all[0].display_name, "Work");
        assert_eq!(all[0].imap_server, "imap.gmail.com");
    }

    #[test]
    fn a_refresh_for_an_account_somebody_removed_reports_that_rather_than_inventing_one() {
        let mut all: Vec<Account> = Vec::new();
        assert!(!accounts::store_refreshed(&mut all, "gone", "a", "r", 1));
        assert!(all.is_empty());
    }

    #[test]
    fn a_google_account_survives_the_round_trip_through_the_file() {
        let f = Fixture::new();
        let mut all = Vec::new();
        accounts::upsert_google(
            &mut all,
            "someone@gmail.com",
            ACCESS_SENTINEL,
            REFRESH_SENTINEL,
            1_700_000_000,
        );
        accounts::save(&f.config(), &all).unwrap();

        let read_back = accounts::load(&f.config()).unwrap();
        assert_eq!(read_back.len(), 1);
        assert!(read_back[0].use_oauth);
        assert_eq!(read_back[0].oauth_refresh_token.as_deref(), Some(REFRESH_SENTINEL));
        assert_eq!(read_back[0].oauth_expires_at, Some(1_700_000_000));
    }

    #[test]
    fn an_account_file_written_before_oauth_had_an_expiry_still_loads() {
        // Deployments have one. Every new field defaults, and a missing expiry means "refresh
        // before you use it" rather than a parse error.
        let f = Fixture::new();
        std::fs::create_dir_all(f.config().parent().unwrap()).unwrap();
        std::fs::write(
            f.config(),
            r#"[{"id":"g","email":"a@gmail.com","imap_server":"imap.gmail.com",
                "smtp_server":"smtp.gmail.com","use_oauth":true,"oauth_token":"ya29.old"}]"#,
        )
        .unwrap();
        let all = accounts::load(&f.config()).unwrap();
        assert_eq!(all[0].oauth_expires_at, None);
        assert_eq!(all[0].oauth_refresh_token, None);
        assert!(google::needs_refresh(all[0].oauth_expires_at, 0));
    }

    // ── What the window does with an answer ──────────────────────────

    #[test]
    fn a_sign_in_that_is_still_going_keeps_the_window_waiting() {
        assert_eq!(state::google_outcome(Ok(OAuthStatus::Waiting)), GoogleOutcome::Waiting);
    }

    #[test]
    fn a_finished_sign_in_carries_the_address_google_chose() {
        let outcome = state::google_outcome(Ok(OAuthStatus::Done {
            account: summary("someone@gmail.com"),
        }));
        assert_eq!(outcome, GoogleOutcome::SignedIn("someone@gmail.com".into()));
    }

    #[test]
    fn a_failed_sign_in_carries_the_reason_to_the_screen() {
        let outcome = state::google_outcome(Ok(OAuthStatus::Failed {
            reason: "The Google sign-in was declined in the browser.".into(),
        }));
        match outcome {
            GoogleOutcome::Stopped(why) => assert!(why.contains("declined"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_mail_service_that_stopped_answering_ends_the_wait_rather_than_spinning() {
        // A window polling a service that is not there until its own deadline, and then saying
        // nothing useful about why, is the silence this app spent September removing.
        match state::google_outcome(Err("connection refused".into())) {
            GoogleOutcome::Stopped(why) => {
                assert!(why.contains("connection refused"), "{why}");
                assert!(why.contains("mail service"), "{why}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_machine_with_no_browser_is_given_the_address_instead_of_a_dead_end() {
        let said = state::browser_failed_note("xdg-open exited 3");
        assert!(said.contains("xdg-open exited 3"), "{said}");
        assert!(said.contains("below"), "{said}");
    }

    #[test]
    fn an_unreachable_service_is_not_the_same_as_a_build_with_no_google_client() {
        // Both draw no button, and only one of them is fixed by setting a client id.
        let down = MailState::Unreachable { reason: "no socket".into() }.google_sign_in();
        assert!(!down.available);
        assert!(down.note.contains("mail service"), "{}", down.note);

        let up = state::decide(Ok(AccountsResult {
            accounts: Vec::new(),
            config_path: "/tmp/e.json".into(),
            secrets_are_plaintext: true,
            google_sign_in: google::availability(&Ok(a_client())),
        }));
        assert!(up.google_sign_in().available);
        assert_ne!(down.note, up.google_sign_in().note);
    }

    // ── And the tokens, held to the password's rule ──────────────────

    #[test]
    fn nothing_a_google_sign_in_can_say_contains_a_token() {
        let all_secrets =
            [ACCESS_SENTINEL, REFRESH_SENTINEL, CODE_SENTINEL, CLIENT_SECRET_SENTINEL];
        let mut said: Vec<String> = Vec::new();

        // Every string the service builds about a token failure, from a body that quotes the
        // credential back — which Google's `error_description` is free to do.
        let echoing = format!(
            "{{\"error\":\"invalid_grant\",\"error_description\":\"bad token {REFRESH_SENTINEL}\"}}"
        );
        said.push(without_secrets(&google::name_token_failure(400, &echoing), &all_secrets));

        // A mail server echoing the AUTHENTICATE line it was sent — the XOAUTH2 equivalent of the
        // LOGIN echo the password rule was written for.
        let imap_echo = format!(
            "BAD Invalid command: AUTHENTICATE XOAUTH2 user=a@b.com auth=Bearer {ACCESS_SENTINEL}"
        );
        said.push(connect::name_oauth_failure(
            &Attempt::new("IMAP", "imap.gmail.com", 993),
            &imap_echo,
            &all_secrets,
        ));
        said.push(connect::name_failure_secrets(
            &Attempt::new("SMTP", "smtp.gmail.com", 587),
            &format!("535-5.7.8 Username and Password not accepted {ACCESS_SENTINEL}"),
            &all_secrets,
        ));

        // The callback parser, handed a code and the wrong state.
        if let google::Callback::Refused(why) = google::parse_callback(
            &format!("GET /?code={CODE_SENTINEL}&state=X HTTP/1.1\r\n\r\n"),
            "OURS",
        ) {
            said.push(why);
        }

        // The three `Debug`s. A `{:?}` in a tracing line is a file on disk that outlives the
        // session, which is the whole reason none of these is derived.
        let pkce = pkce_of(RFC_VERIFIER, "S");
        said.push(format!("{pkce:?}"));
        said.push(format!("{:?}", a_client()));
        said.push(format!(
            "{:?}",
            google::tokens_from_json(
                &serde_json::json!({
                    "access_token": ACCESS_SENTINEL,
                    "refresh_token": REFRESH_SENTINEL,
                    "id_token": "header.payload.signature",
                    "expires_in": 3600,
                }),
                0,
            )
            .unwrap()
        ));

        // The stored account, and what the service answers with about it.
        let mut all = Vec::new();
        accounts::upsert_google(&mut all, "someone@gmail.com", ACCESS_SENTINEL, REFRESH_SENTINEL, 1);
        said.push(format!("{:?}", all[0]));
        said.push(serde_json::to_string(&all[0].summary()).unwrap());
        said.push(
            serde_json::to_string(&AccountsResult {
                accounts: accounts::summaries(&all),
                config_path: "/tmp/e.json".into(),
                secrets_are_plaintext: true,
                google_sign_in: google::availability(&Ok(a_client())),
            })
            .unwrap(),
        );

        // And the two wire types the flow answers with. An `OAuthBeginResult` carries the URL a
        // browser is given: the challenge is in it and must be, and the verifier must not.
        said.push(
            serde_json::to_string(&OAuthBeginResult {
                flow_id: "f1".into(),
                auth_url: google::auth_url("c", &google::redirect_uri(1), &pkce),
                expires_in_secs: 300,
            })
            .unwrap(),
        );
        said.push(
            serde_json::to_string(&OAuthStatus::Done { account: all[0].summary() }).unwrap(),
        );

        // And what the window does with each of those, which is where they reach a screen.
        for reason in said.clone() {
            if let GoogleOutcome::Stopped(text) = state::google_outcome(Err(reason)) {
                said.push(text);
            }
        }

        for sentence in &said {
            for (name, secret) in [
                ("the access token", ACCESS_SENTINEL),
                ("the refresh token", REFRESH_SENTINEL),
                ("the authorization code", CODE_SENTINEL),
                ("the PKCE verifier", RFC_VERIFIER),
                ("the client secret", CLIENT_SECRET_SENTINEL),
            ] {
                assert!(!sentence.contains(secret), "{name} reached: {sentence}");
            }
        }
    }

    #[test]
    fn the_stored_file_holds_the_tokens_and_the_answer_does_not() {
        // The same shape as the password test above: the file on disk *does* hold them, because
        // the service has to sign in with them tomorrow. What must never happen is that file
        // being confused with what the service answers with.
        let f = Fixture::new();
        let mut all = Vec::new();
        accounts::upsert_google(&mut all, "someone@gmail.com", ACCESS_SENTINEL, REFRESH_SENTINEL, 1);
        accounts::save(&f.config(), &all).unwrap();

        let on_disk = std::fs::read_to_string(f.config()).unwrap();
        assert!(on_disk.contains(REFRESH_SENTINEL), "the service could not renew this account");

        let answered = serde_json::to_string(&accounts::summaries(&all)).unwrap();
        assert!(!answered.contains(ACCESS_SENTINEL), "{answered}");
        assert!(!answered.contains(REFRESH_SENTINEL), "{answered}");
        // And it still says the useful thing: this account signs in with Google.
        assert!(answered.contains("\"uses_oauth\":true"), "{answered}");
    }

    #[test]
    fn an_accounts_answer_from_an_older_service_still_parses() {
        // `google_sign_in` is `#[serde(default)]`, so a binary that predates this change parses
        // as "Google sign-in is not available" — which is the truth about that binary.
        let older = serde_json::json!({
            "accounts": [],
            "config_path": "/tmp/e.json",
            "secrets_are_plaintext": true,
        });
        let parsed: AccountsResult = serde_json::from_value(older).unwrap();
        assert!(!parsed.google_sign_in.available);
        assert!(!state::decide(Ok(parsed)).google_sign_in().available);
    }

    #[test]
    fn the_flow_answers_round_trip_between_the_service_and_the_window() {
        // The payload test, for the new pair. The calendar's two ends each spelled their own
        // parameter names and disagreed, and every listing failed while both files looked right.
        let begun = OAuthBeginResult {
            flow_id: "f1".into(),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth?x=1".into(),
            expires_in_secs: 300,
        };
        let sent = serde_json::to_value(&begun).unwrap();
        let received: OAuthBeginResult = serde_json::from_value(sent).unwrap();
        assert_eq!(received.flow_id, "f1");
        assert_eq!(received.expires_in_secs, 300);

        for status in [
            OAuthStatus::Waiting,
            OAuthStatus::Done { account: summary("a@gmail.com") },
            OAuthStatus::Failed { reason: "declined".into() },
        ] {
            let sent = serde_json::to_value(&status).unwrap();
            let received: OAuthStatus = serde_json::from_value(sent).unwrap();
            // The three arms stay three arms through the wire, which is the only thing the
            // window's polling loop depends on.
            assert_eq!(
                std::mem::discriminant(&status),
                std::mem::discriminant(&received)
            );
        }
    }

    // ── The HTML original, as the browser gets it ────────────────────
    //
    // "Open original" writes the sender's HTML to a file and hands it to a browser (#275). A
    // click that only meant "let me see the layout" must not tell the sender the mail was
    // opened, when, and from which address — which is exactly what the invisible pictures in
    // notification mail exist to report. What the cleaning keeps and drops is a decision, and
    // the decisions live in `state.rs` so they can be checked here without a window.

    #[test]
    fn every_image_that_would_fetch_is_gone_before_the_browser_gets_the_mail() {
        let html = "<p>Read:</p>\
             <img src=\"https://track.example/open.gif?to=someone@example.com\">\
             <img src='http://track.example/pixel.png'>\
             <img src=\"//track.example/relative.gif\">\
             <img src=\"local.png\" srcset=\"https://cdn.example/big.png 2x\">";
        let (cleaned, removed) = state::strip_remote_images(html);
        assert_eq!(removed, 4, "{removed} of 4 fetches found in:\n{cleaned}");
        for gone in ["track.example", "cdn.example", "open.gif", "big.png"] {
            assert!(!cleaned.contains(gone), "{gone} survived:\n{cleaned}");
        }
        // What the mail says is not an image and stays.
        assert!(cleaned.contains("<p>Read:</p>"), "{cleaned}");
    }

    #[test]
    fn images_the_mail_carries_itself_stay_and_so_does_every_other_tag() {
        // `cid:` is a part of this very message and `data:` is inline bytes; neither leaves the
        // machine. Links stay too: an `<a>` fetches nothing until it is clicked, and the issue
        // is about what a page load reports.
        let html = "<a href=\"https://news.example/story\">story</a>\
             <img src=\"cid:logo@example\">\
             <img src=\"data:image/png;base64,iVBOR\">\
             <table><tr><td>24 upvotes</td></tr></table>";
        let (cleaned, removed) = state::strip_remote_images(html);
        assert_eq!(removed, 0);
        assert_eq!(cleaned, html);
    }

    #[test]
    fn a_tag_is_judged_by_its_own_attributes_not_by_a_lookalike() {
        // `data-src` is a lazy-load placeholder: as the tag stands the browser fetches nothing
        // from it, so it does not decide the tag's fate — and a boundary check is what stops
        // `data-src` being read as `src`.
        let html = "<img data-src=\"https://cdn.example/late.png\" src=\"cid:part1\">\
             <imgx src=\"https://not-an-img.example/x.gif\">";
        let (cleaned, removed) = state::strip_remote_images(html);
        assert_eq!(removed, 0, "lookalikes were judged:\n{cleaned}");
        assert!(cleaned.contains("cid:part1"), "{cleaned}");
        assert!(cleaned.contains("<imgx"), "{cleaned}");
    }

    #[test]
    fn the_scanner_reads_a_tag_the_way_a_browser_would() {
        // Uppercase, unquoted values, and a `>` sitting inside an attribute: a scanner that
        // ended the tag at that `>` would leave half a fetch behind in the file.
        let html = "<IMG SRC=https://TRACK.EXAMPLE/P.GIF>\
             <img alt=\"a > b\" src=\"https://track.example/q.gif\">";
        let (cleaned, removed) = state::strip_remote_images(html);
        assert_eq!(removed, 2, "{removed} of 2 found in:\n{cleaned}");
        assert!(!cleaned.to_lowercase().contains("track.example"), "{cleaned}");
        assert!(!cleaned.contains("p.gif") && !cleaned.contains("q.gif"), "{cleaned}");
    }

    #[test]
    fn an_id_from_the_wire_never_becomes_a_path() {
        // The message id is the server's string, and the file goes where a browser can open it.
        // A separator or a `..` segment surviving into the name would let a hostile id choose
        // the location of the file.
        let dir = PathBuf::from("/home/p/.cache/yantrik/email");
        let escaped = state::original_html_file(&dir, "../../etc/passwd");
        assert_eq!(escaped.parent(), Some(dir.as_path()), "{escaped:?} left the directory");
        assert_eq!(escaped.file_name().unwrap(), "etcpasswd.html");
        assert_eq!(
            state::original_html_file(&dir, "<17e0.9c2@news.example>").file_name().unwrap(),
            "17e09c2newsexample.html"
        );
        // An id that was all punctuation still gets a file rather than a bare ".html".
        assert_eq!(
            state::original_html_file(&dir, "///").file_name().unwrap(),
            "message.html"
        );
    }

    #[test]
    fn the_notice_says_what_was_taken_out_of_the_mail() {
        // The issue asks for this sentence: a person who clicked "Open original" is told the
        // remote images went, so the click is not silently a phone-home.
        let clean = state::original_opened_note(0);
        assert!(clean.contains("no remote images"), "{clean}");
        let one = state::original_opened_note(1);
        assert!(one.contains("1 remote image removed"), "{one}");
        assert!(one.contains("sender is not told"), "{one}");
        let many = state::original_opened_note(7);
        assert!(many.contains("7 remote images removed"), "{many}");
        assert!(many.contains("sender is not told"), "{many}");
    }
}
