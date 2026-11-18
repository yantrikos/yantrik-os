//! The saved places: the rules for adding, removing and choosing one, on the prefs alone. Pure,
//! so a refusal leaves the prefs as they were, and the one path the panel and the intents share.

use yantrik_app::Refused;

use crate::prefs::{Prefs, SavedLocation};

/// Keeps `loc`, makes it the one shown, and says where it went.
pub fn add(prefs: &mut Prefs, loc: SavedLocation) -> Result<usize, Refused> {
    if let Some(existing) = prefs.locations.iter().find(|l| l.is_same_place(&loc)) {
        return Err(Refused::new(
            "already_saved",
            format!("\"{}\" is already saved, as \"{}\"", loc.name, existing.name),
        ));
    }
    prefs.locations.push(loc);
    prefs.active = prefs.locations.len() - 1;
    Ok(prefs.active)
}

/// Forgets the place at `idx`. The last one stays: a weather app with no place shows nothing.
pub fn remove(prefs: &mut Prefs, idx: usize) -> Result<SavedLocation, Refused> {
    let n = prefs.locations.len();
    if idx >= n {
        return Err(Refused::new(
            "no_such_place",
            format!(
                "there is no saved location {idx}; there {}",
                if n == 1 { "is 1".to_string() } else { format!("are {n}") }
            ),
        ));
    }
    if n == 1 {
        return Err(Refused::new(
            "last_place",
            "the last saved location cannot be removed; add another first",
        ));
    }
    let removed = prefs.locations.remove(idx);
    prefs.active = if prefs.active > idx {
        prefs.active - 1
    } else {
        prefs.active.min(prefs.locations.len() - 1)
    };
    Ok(removed)
}

/// Shows the place at `idx`.
pub fn select(prefs: &mut Prefs, idx: usize) -> Result<SavedLocation, Refused> {
    if idx >= prefs.locations.len() {
        return Err(Refused::new(
            "no_such_place",
            format!("there is no saved location {idx}"),
        ));
    }
    prefs.active = idx;
    Ok(prefs.active_location())
}

/// The saved place whose name contains `want` (case-insensitively), as the `show_location`
/// intent finds it, or the sentence that names what there is.
pub fn find(prefs: &Prefs, want: &str) -> Result<usize, Refused> {
    let want = want.trim().to_lowercase();
    prefs
        .locations
        .iter()
        .position(|l| l.name.to_lowercase().contains(&want))
        .ok_or_else(|| {
            let names: Vec<&str> = prefs.locations.iter().map(|l| l.name.as_str()).collect();
            Refused::new(
                "no_such_place",
                if names.is_empty() {
                    "no locations are saved yet; add one first".to_string()
                } else {
                    format!(
                        "no saved location matches \"{want}\"; there is: {}",
                        names.join(", ")
                    )
                },
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> Prefs {
        Prefs {
            fahrenheit: false,
            active: 1,
            locations: vec![
                SavedLocation::new_test("London", 51.5, -0.1),
                SavedLocation::new_test("Paris, France", 48.85, 2.35),
                SavedLocation::new_test("Oslo", 59.9, 10.7),
            ],
        }
    }

    #[test]
    fn adding_shows_the_new_place_and_refuses_the_same_place_twice() {
        let mut p = three();
        assert_eq!(add(&mut p, SavedLocation::new_test("Rome", 41.9, 12.5)), Ok(3));
        assert_eq!(p.active, 3);
        let e = add(&mut p, SavedLocation::new_test("Paris", 48.851, 2.351)).unwrap_err();
        assert_eq!(e.sentence, "\"Paris\" is already saved, as \"Paris, France\"");
        assert_eq!(p.locations.len(), 4, "a refusal changes nothing");
    }

    #[test]
    fn removing_keeps_the_shown_place_shown_and_never_the_last() {
        let mut p = three();
        remove(&mut p, 0).unwrap();
        assert_eq!(p.active, 0, "Paris moved down with the removal");
        assert_eq!(p.active_location().name, "Paris, France");
        remove(&mut p, 1).unwrap();
        assert!(remove(&mut p, 0).is_err(), "the last stays");
        assert!(remove(&mut p, 5).unwrap_err().sentence.contains("there is 1"));
    }

    #[test]
    fn selecting_and_finding() {
        let mut p = three();
        assert_eq!(select(&mut p, 2).unwrap().name, "Oslo");
        assert!(select(&mut p, 3).is_err());
        assert_eq!(find(&p, "par"), Ok(1));
        let e = find(&p, "berlin").unwrap_err();
        assert!(e.sentence.contains("there is: London, Paris, France, Oslo"), "{}", e.sentence);
    }
}
