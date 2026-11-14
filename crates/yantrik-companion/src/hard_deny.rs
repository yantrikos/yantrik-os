//! Tools no loop may run on its own authority, whatever has been allowed before.
//!
//! This is a list, not a score: it never moves, and nothing the person does or says loosens it.

/// Tools refused outright inside the companion's tool loop.
const NEVER_FROM_THE_LOOP: &[&str] = &["vault_delete", "delete_all_memories"];

/// The reason a tool call is refused, or `None` when the loop may go on to the usual checks.
pub fn refusal(tool: &str) -> Option<String> {
    NEVER_FROM_THE_LOOP
        .contains(&tool)
        .then(|| format!("'{tool}' is on the deny list"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_and_memory_wipes_are_refused_and_the_rest_is_not() {
        assert!(refusal("vault_delete").is_some());
        assert!(refusal("delete_all_memories").is_some());
        assert!(refusal("read_file").is_none());
    }
}
