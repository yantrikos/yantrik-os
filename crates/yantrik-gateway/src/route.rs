//! A requested model id, `<account>/<model>`: the account is everything up to the first slash,
//! the model the rest, slashes and all (`free-groq/openai/gpt-oss-120b`).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub account: String,
    pub model: String,
}

/// Split a model id. `None` when there is no account, no model, or the account is not a word a
/// person could have been shown (lowercase letters, digits, dashes).
pub fn parse(id: &str) -> Option<Route> {
    let (account, model) = id.trim().split_once('/')?;
    let ok = !account.is_empty()
        && account.len() <= 64
        && account.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok || model.trim().is_empty() || model.len() > 256 || model.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(Route { account: account.to_string(), model: model.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_is_up_to_the_first_slash_and_the_model_keeps_its_own() {
        assert_eq!(parse("ollama-cloud/deepseek-v4.1-flash"), Some(Route { account: "ollama-cloud".into(), model: "deepseek-v4.1-flash".into() }));
        assert_eq!(parse("free-groq/openai/gpt-oss-120b").unwrap().model, "openai/gpt-oss-120b");
        for bad in ["gpt-4o", "/gpt-4o", "openai/", "Open AI/x", "a b/c", "openai/x\ny"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
