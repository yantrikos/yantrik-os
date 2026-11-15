//! Asking every application for its windows, together (#1257 review, H3 and N2).
//!
//! Any connection can be an application: `Embed` on the registry is how real ones register, and
//! nothing stops a program registering a hundred that never answer. Asked one after another, each
//! under its own limit, thirteen of them used up the whole listing. So the applications are asked
//! together, sixteen at a time, each under [`APP_LIMIT`]; the ones a caller already knows to be
//! slow are not asked at all; at most [`MAX_APPS`] are asked in one listing; and whatever was not
//! heard from is listed as unanswered, never as gone.
//!
//! Generic over what one application is asked, so the loop can be tested without a bus.

use std::collections::HashSet;
use std::future::Future;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use tokio::time::Instant;

use crate::handles::Reference;
use crate::screen::WindowInfo;

/// The longest one application may take to list its own windows. One that is frozen costs a
/// listing this much, and is reported as not having answered: it has not closed its windows.
pub const APP_LIMIT: Duration = Duration::from_millis(400);

/// Applications asked at the same time.
pub const AT_ONCE: usize = 16;

/// Applications asked in one listing at most. The rest are listed as unanswered.
pub const MAX_APPS: usize = 256;

/// What a walk of every application found.
#[derive(Default)]
pub struct Listing {
    pub windows: Vec<(Reference, WindowInfo)>,
    /// Applications the registry named, up to [`MAX_APPS`]: the connections the model may ask
    /// about, whether or not they have a window.
    pub apps: Vec<String>,
    /// Connections that did not list their windows within [`APP_LIMIT`], were not asked because
    /// they are slow or because there were too many, or were not heard from before the listing's
    /// own limit. Their windows are not in `windows`, and are not thereby closed.
    pub unanswered: Vec<String>,
    /// Each application that was asked and heard from within the listing's limit, for the
    /// watcher's ration of time.
    pub asked: Vec<Asked>,
}

/// One application a listing asked, and what it cost.
pub struct Asked {
    pub connection: String,
    pub took: Duration,
    /// False: it did not answer within [`APP_LIMIT`].
    pub answered: bool,
}

/// Ask each of `apps` for its windows with `per_app`, at most [`AT_ONCE`] at a time, none that
/// `skip` names, and all of it within `overall`.
pub async fn list_apps<F, Fut>(apps: Vec<Reference>, skip: impl Fn(&str) -> bool, per_app: F, overall: Duration) -> Listing
where
    F: Fn(Reference) -> Fut,
    Fut: Future<Output = Vec<(Reference, WindowInfo)>>,
{
    let mut listing = Listing::default();
    let mut asked = Vec::new();
    for (index, app) in apps.into_iter().enumerate() {
        if index < MAX_APPS {
            listing.apps.push(app.0.clone());
        }
        if index >= MAX_APPS || skip(&app.0) {
            listing.unanswered.push(app.0);
        } else {
            asked.push((index, app));
        }
    }
    let waiting: Vec<String> = asked.iter().map(|(_, app)| app.0.clone()).collect();
    let mut answered: Vec<(usize, Vec<(Reference, WindowInfo)>)> = Vec::new();
    let mut heard = HashSet::new();
    let work = async {
        let mut results = stream::iter(asked)
            .map(|(index, app)| {
                let name = app.0.clone();
                let question = per_app(app);
                async move {
                    let started = Instant::now();
                    let found = tokio::time::timeout(APP_LIMIT, question).await.ok();
                    (index, name, started.elapsed(), found)
                }
            })
            .buffer_unordered(AT_ONCE);
        while let Some((index, name, took, found)) = results.next().await {
            heard.insert(name.clone());
            listing.asked.push(Asked { connection: name.clone(), took, answered: found.is_some() });
            match found {
                Some(windows) => answered.push((index, windows)),
                None => listing.unanswered.push(name),
            }
        }
    };
    // Whatever was heard before the limit counts; the rest is unanswered.
    let _ = tokio::time::timeout(overall, work).await;
    listing.unanswered.extend(waiting.into_iter().filter(|name| !heard.contains(name)));
    // In the order the registry named them, whichever answered first.
    answered.sort_by_key(|(index, _)| *index);
    listing.windows = answered.into_iter().flat_map(|(_, windows)| windows).collect();
    listing
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
