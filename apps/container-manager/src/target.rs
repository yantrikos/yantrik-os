//! Which ONE container a destructive call means, and what the approval card names it as.
//!
//! `remove` is asked about on a card that names the container from the runtime's own listing
//! (`app.name_target`), and then runs when the person allows it. The namer and the handler both
//! resolve the caller's word here, against a listing read NOW with full ids — never the window's
//! list, which can be minutes old — and the card's grant carries the full id it named. The handler
//! holds its own resolution to that id (`held_to_grant`) before `rm -f`, so a container renamed
//! into the name after the Allow is not the one removed (security review of #652, H1).
//!
//! A word that could mean more than one container — two id prefixes, two names containing it —
//! names nothing: the card is then Decline only, and the handler refuses rather than guess.

use yantrik_app_runtime::control::Target;

use crate::runtime;

/// The containers on this machine as the runtime lists them now, with untruncated ids.
pub fn listing_now() -> Result<Vec<runtime::Container>, String> {
    let rt = runtime::detect();
    match runtime::run(rt, &["ps", "-a", "--no-trunc", "--format", runtime::PS_FORMAT]) {
        runtime::Exit::Ran { code: Some(0), stdout, .. } => Ok(runtime::parse_containers(&stdout)),
        _ => Err(format!("{rt} would not list the containers on this machine, so none was named")),
    }
}

/// The one container `needle` means in `list`: an exact name, else the one id it begins, else
/// the one name it is part of. `None` when there is none, or more than one at the step that
/// decides.
pub fn one<'a>(list: &'a [runtime::Container], needle: &str) -> Option<&'a runtime::Container> {
    let want = needle.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    let steps: [&dyn Fn(&runtime::Container) -> bool; 3] = [
        &|c| c.name.to_lowercase() == want,
        &|c| c.id.to_lowercase().starts_with(&want),
        &|c| c.name.to_lowercase().contains(&want),
    ];
    for matches in steps {
        let mut found = list.iter().filter(|c| matches(c));
        if let Some(first) = found.next() {
            return found.next().is_none().then_some(first);
        }
    }
    None
}

/// The approval card's rows for the container `needle` names in `list`: its name, its image and
/// its state as the runtime reports them, identified by its full id.
pub fn container_target(list: &[runtime::Container], needle: &str) -> Option<Target> {
    let c = one(list, needle)?;
    let state = if c.status_text.trim().is_empty() { c.state.clone() } else { c.status_text.clone() };
    Some(Target {
        rows: vec![
            ("Container".into(), c.name.clone()),
            ("Image".into(), c.image.clone()),
            ("State".into(), state),
        ],
        series: false,
        identity: c.id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(id: &str, name: &str) -> runtime::Container {
        runtime::Container {
            id: id.into(),
            name: name.into(),
            image: "postgres:16".into(),
            state: "running".into(),
            status_text: "Up 2 hours".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_container_is_named_by_name_image_and_state_and_identified_by_its_full_id() {
        let list = vec![container("3f2a9c1b7d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4", "postgres-dev")];
        let t = container_target(&list, "3f2a9c1b7d4e").expect("named by id");
        assert_eq!(t.rows[0], ("Container".to_string(), "postgres-dev".to_string()));
        assert_eq!(t.rows[1].1, "postgres:16");
        assert_eq!(t.rows[2].1, "Up 2 hours");
        assert_eq!(t.identity.len(), 64, "the full id, not the prefix the caller gave");
        assert!(container_target(&list, "nope").is_none());
    }

    /// The caller's word means one container or none: a prefix two ids begin with, or a part of
    /// two names, names nothing. An exact name still wins over a longer one containing it.
    #[test]
    fn a_word_that_could_mean_two_containers_means_none() {
        let list = vec![container("aaa111", "web"), container("aaa222", "webhook-runner"), container("bbb333", "db-web")];
        assert_eq!(one(&list, "web").map(|c| c.id.as_str()), Some("aaa111"), "the exact name");
        assert!(one(&list, "aaa").is_none(), "two ids begin with it");
        assert_eq!(one(&list, "aaa2").map(|c| c.id.as_str()), Some("aaa222"));
        assert!(one(&list, "we").is_none(), "part of three names");
        assert_eq!(one(&list, "hook").map(|c| c.id.as_str()), Some("aaa222"));
    }

    /// H1 for containers: the card named one container; another is renamed into the word before
    /// the grant is spent. The handler's resolution now has another identity, and the shared
    /// check refuses before anything is removed.
    #[test]
    fn a_container_renamed_into_the_word_after_the_allow_is_not_removed() {
        let before = vec![container("aaa111", "scratch"), container("bbb222", "prod-db")];
        let named = container_target(&before, "scratch").expect("named").identity;
        let after = vec![container("aaa111", "scratch-old"), container("bbb222", "scratch")];
        let _grant = yantrik_app_runtime::control::GrantedTargetScope::enter(Some(named.clone()));
        let now = one(&after, "scratch").map(|c| c.id.as_str());
        let refused = yantrik_app_runtime::control::held_to_grant(now).unwrap_err();
        assert!(refused.starts_with("the target changed after you allowed it"), "{refused}");
        assert!(yantrik_app_runtime::control::held_to_grant(Some(&named)).is_ok());
    }
}
