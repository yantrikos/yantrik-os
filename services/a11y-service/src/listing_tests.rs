//! A listing asks applications together, and none of them can hold it up for long.

use std::cell::Cell;

use super::*;
use zbus::zvariant::ObjectPath;

fn app(n: usize) -> Reference {
    (format!(":1.{n}"), ObjectPath::try_from("/org/a11y/atspi/accessible/root").expect("a valid path").into())
}

fn window_of(app: &Reference) -> (Reference, WindowInfo) {
    let info = WindowInfo { id: format!("w{}", app.0), app: "app".into(), title: "t".into(), role: "frame".into(), pid: 1 };
    (app.clone(), info)
}

/// #1257 review, N2: thirteen applications that never answer cost 5.2 s one after another, which
/// is more than a whole listing is allowed.
#[tokio::test(start_paused = true)]
async fn twenty_applications_that_never_answer_cost_one_limit_not_twenty() {
    let apps: Vec<Reference> = (0..20).map(app).collect();
    let started = Instant::now();
    let listing = list_apps(apps, |_| false, |_| std::future::pending(), Duration::from_secs(5)).await;
    let took = started.elapsed();
    assert_eq!(listing.unanswered.len(), 20, "none answered, and none is said to be gone");
    assert!(listing.windows.is_empty());
    // Sixteen together and then four: two limits, not twenty.
    assert!(took <= APP_LIMIT * 2 + Duration::from_millis(50), "{took:?}");
    assert_eq!(listing.asked.len(), 20, "each is charged for what it took");
    assert!(listing.asked.iter().all(|asked| !asked.answered && asked.took >= APP_LIMIT));
}

#[tokio::test(start_paused = true)]
async fn applications_that_answer_are_listed_in_the_order_the_registry_named_them() {
    let apps: Vec<Reference> = (0..5).map(app).collect();
    // The first answers last.
    let per_app = |app: Reference| async move {
        let n: u64 = app.0.trim_start_matches(":1.").parse().unwrap_or(0);
        tokio::time::sleep(Duration::from_millis(50 - n * 10)).await;
        vec![window_of(&app)]
    };
    let listing = list_apps(apps, |_| false, per_app, Duration::from_secs(5)).await;
    let names: Vec<&str> = listing.windows.iter().map(|(app, _)| app.0.as_str()).collect();
    assert_eq!(names, [":1.0", ":1.1", ":1.2", ":1.3", ":1.4"]);
    assert!(listing.unanswered.is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_application_known_to_be_slow_is_not_asked_and_is_listed_as_unanswered() {
    let asked = Cell::new(0);
    let per_app = |app: Reference| {
        asked.set(asked.get() + 1);
        async move { vec![window_of(&app)] }
    };
    let listing = list_apps((0..4).map(app).collect(), |name| name == ":1.2", per_app, Duration::from_secs(5)).await;
    assert_eq!(asked.get(), 3, "the slow one cost nothing");
    assert_eq!(listing.unanswered, [":1.2"]);
    assert_eq!(listing.windows.len(), 3);
    assert_eq!(listing.apps.len(), 4, "it is still an application the registry named");
}

#[tokio::test(start_paused = true)]
async fn no_more_than_the_cap_of_applications_are_asked_in_one_listing() {
    let asked = Cell::new(0);
    let per_app = |app: Reference| {
        asked.set(asked.get() + 1);
        async move { vec![window_of(&app)] }
    };
    let listing = list_apps((0..MAX_APPS + 44).map(app).collect(), |_| false, per_app, Duration::from_secs(5)).await;
    assert_eq!(asked.get(), MAX_APPS);
    assert_eq!(listing.unanswered.len(), 44, "the rest are not said to be gone");
    assert_eq!(listing.apps.len(), MAX_APPS);
}

#[tokio::test(start_paused = true)]
async fn no_more_than_sixteen_are_asked_at_once() {
    let running = Cell::new(0usize);
    let most = Cell::new(0usize);
    let per_app = |app: Reference| {
        let (running, most) = (&running, &most);
        async move {
            running.set(running.get() + 1);
            most.set(most.get().max(running.get()));
            tokio::time::sleep(Duration::from_millis(10)).await;
            running.set(running.get() - 1);
            vec![window_of(&app)]
        }
    };
    let listing = list_apps((0..100).map(app).collect(), |_| false, per_app, Duration::from_secs(5)).await;
    assert_eq!(listing.windows.len(), 100);
    assert_eq!(most.get(), AT_ONCE);
}

/// The listing's own limit is not an error: what was heard counts, the rest is unanswered.
#[tokio::test(start_paused = true)]
async fn what_was_heard_before_the_overall_limit_is_kept() {
    let per_app = |app: Reference| async move {
        if app.0 == ":1.0" {
            return vec![window_of(&app)];
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        vec![window_of(&app)]
    };
    let listing = list_apps((0..40).map(app).collect(), |_| false, per_app, Duration::from_millis(500)).await;
    assert!(listing.windows.len() >= 17, "the first sixteen and the next that began in time: {}", listing.windows.len());
    assert_eq!(listing.windows.len() + listing.unanswered.len(), 40, "everybody is either listed or unanswered");
    assert!(!listing.unanswered.is_empty());
}
