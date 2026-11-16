//! The places the person has: what an unattended weather lookup may name.
//!
//! The weather app keeps the places the person saved (it alone reads and writes its settings, and
//! says them in its `describe`), and the companion knows where the person lives (`user_location`).
//! With nobody there, `get_weather` and the weather watch send only one of those to the weather
//! service: text the companion chose, however plain it reads (`rust news`, a contact's name), is for a
//! person's own turn to type.

use std::sync::Mutex;

/// Where the person lives, as the companion knows it (set once the companion has loaded it).
static USER_LOCATION: Mutex<String> = Mutex::new(String::new());

/// Note the companion's `user_location`, which an unattended lookup may name. What the companion
/// loaded can be a sentence remembered from a chat; only something that reads as a place is kept.
pub fn remember_user_location(place: &str) {
    *USER_LOCATION.lock().unwrap_or_else(|e| e.into_inner()) =
        yantrik_ipc_contracts::weather::named_place(place).unwrap_or_default();
}

/// Whether `place` is a name in `saved` (the weather app's saved places, as it describes them) or is
/// the place the person lives. Names compare without case.
pub fn is_theirs(place: &str, saved: &[String]) -> bool {
    let place = place.trim();
    if place.is_empty() {
        return false;
    }
    let home = USER_LOCATION.lock().unwrap_or_else(|e| e.into_inner()).clone();
    (!home.is_empty() && home.eq_ignore_ascii_case(place)) || saved.iter().any(|name| !name.is_empty() && name.trim().eq_ignore_ascii_case(place))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Vec<String> {
        vec!["Oslo".into(), "Paris, France".into()]
    }

    #[test]
    fn a_saved_place_or_the_home_is_theirs() {
        remember_user_location("  Lisbon ");
        for place in ["Oslo", " oslo ", "Paris, France", "lisbon"] {
            assert!(is_theirs(place, &saved()), "{place}");
        }
        remember_user_location("");
    }

    #[test]
    fn anything_else_is_text_somebody_chose() {
        remember_user_location("a house near the big lake in Oslo, where she lives");
        for place in ["rust news", "Priya Nair", "Oslo lawyer", "59.9139,10.7522", "Rome", "", "  ", "a house near the big lake in Oslo, where she lives"] {
            assert!(!is_theirs(place, &saved()), "{place:?}");
        }
        assert!(!is_theirs("Oslo", &[]), "the weather app did not say, and no home is remembered");
    }
}
