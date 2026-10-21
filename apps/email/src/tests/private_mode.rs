//! Private mode (#924): Email keeps none of its keys. Every one of them names a folder, an account,
//! a sender or a subject, so in Private mode `describe` says Email is open and nothing more.

#[test]
fn email_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::Email>::new();
    <crate::Email as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
