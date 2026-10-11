//! What AppTile needs from Rust: the letter on the tile of an app with no art and no icon.
//!
//! Slint cannot take a string apart, so `AppInitial.of` is a callback each window that draws
//! tiles answers with this one rule (the shell in wire/mod.rs, the preview harness for its
//! stills).

/// The letter an unknown app's tile wears: the first letter or digit of the name in its id,
/// upper-cased. A reverse-DNS id (`org.gnome.Nautilus`) is named by its last part, and our own
/// `yantrik-` prefix is not part of anything's name.
pub fn initial(app_id: &str) -> String {
    let name = app_id.rsplit('.').next().unwrap_or(app_id);
    crate::lock_shared::initial_of(name.strip_prefix("yantrik-").unwrap_or(name))
}

#[cfg(test)]
mod tests {
    use super::initial;

    #[test]
    fn an_app_is_known_by_the_name_in_its_id() {
        assert_eq!(initial("gimp"), "G");
        assert_eq!(initial("org.gnome.Nautilus"), "N");
        assert_eq!(initial("com.github.tchx84.Flatseal"), "F");
        assert_eq!(initial("yantrik-thing"), "T");
        assert_eq!(initial("7zip"), "7");
        assert_eq!(initial("_private"), "P", "the first letter, not the first character");
        assert_eq!(initial(""), "", "nothing to go on draws the plate bare");
    }
}
