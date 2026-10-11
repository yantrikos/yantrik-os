//! The last answer to a slow question, so a read answers from it instead of asking again.
//!
//! A service's `describe` is a read a caller waits on, and `yos check` holds it to 500 ms. Some of
//! what a describe reports is slow to learn: a process table's CPU shares are a fact about a
//! half-second interval, and the weather is an HTTPS round trip to Open-Meteo. Learning it inside
//! describe made every read of System Monitor's service take ~545 ms (#625) and every read of the
//! weather service ~580 ms. [`Recent`] keeps the last value, fetches a new one behind the answer
//! once the old one ages, and tells the caller how old what it got is.
//!
//! What a read does with the last value, by its age ([`Policy`]):
//!
//! | age                               | the read                                              |
//! |-----------------------------------|-------------------------------------------------------|
//! | up to `current_for`               | answers from it                                       |
//! | up to `usable_for`                | answers from it at once, and fetches a new one behind |
//! | older, none, or for another key   | starts a fetch and waits for it, at most `wait`        |
//!
//! One fetch per key runs at a time: a burst of reads never starts a burst of fetches. A fetch
//! that fails is remembered, and nothing fetches that key again until `retry_after` has passed,
//! so a service that is offline answers "it failed, this long ago" at once rather than waiting on
//! the network every read.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

/// How old a value may be before a read stops answering from it as it is.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// A value this young is as current as a new one would be: it is served and nothing fetches.
    pub current_for: Duration,
    /// Up to this age a value is served at once and a new one fetched behind the answer. Past it,
    /// the value is no longer an answer, and a read waits for a new one as if there were none.
    pub usable_for: Duration,
    /// How long a read with nothing usable waits for a fetch before it answers "fetching".
    pub wait: Duration,
    /// After a fetch fails, how long before that key is fetched again.
    pub retry_after: Duration,
}

/// What a read found.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading<T> {
    /// A value, how long ago it was fetched, and when. `last_error` is set when a fetch since
    /// then failed, so an old value can say why it was not replaced.
    Known { value: T, age: Duration, fetched_at: SystemTime, last_error: Option<String> },
    /// Nothing usable yet; a fetch is under way and did not finish within `wait`.
    Fetching,
    /// Nothing usable; the last fetch failed `age` ago, and the next is not due yet.
    Failed { error: String, age: Duration },
}

type Fetch<K, T> = dyn Fn(&K) -> Result<T, String> + Send + Sync;

struct Value<K, T> {
    key: K,
    at: Instant,
    wall: SystemTime,
    value: T,
}

struct Failure<K> {
    key: K,
    at: Instant,
    error: String,
}

struct State<K, T> {
    last: Option<Value<K, T>>,
    failure: Option<Failure<K>>,
    /// The keys a fetch is running for.
    fetching: Vec<K>,
}

struct Shared<K, T> {
    policy: Policy,
    fetch: Box<Fetch<K, T>>,
    state: Mutex<State<K, T>>,
    /// Woken whenever a fetch ends, either way.
    ended: Condvar,
}

/// The last value of something slow to learn, keyed by what it is about (a place, or `()` when
/// there is only one thing). Cloning shares it.
pub struct Recent<K, T>(Arc<Shared<K, T>>);

impl<K, T> Clone for Recent<K, T> {
    fn clone(&self) -> Self {
        Recent(self.0.clone())
    }
}

impl<K, T> Recent<K, T>
where
    K: Clone + PartialEq + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
{
    /// `fetch` learns the value for a key, slowly; it runs on a thread of its own, never on the
    /// reader's.
    pub fn new(policy: Policy, fetch: impl Fn(&K) -> Result<T, String> + Send + Sync + 'static) -> Self {
        Recent(Arc::new(Shared {
            policy,
            fetch: Box::new(fetch),
            state: Mutex::new(State { last: None, failure: None, fetching: Vec::new() }),
            ended: Condvar::new(),
        }))
    }

    /// Keep a value learned some other way — a data method that just fetched the same thing.
    pub fn put(&self, key: K, value: T) {
        self.put_as_of(key, value, Duration::ZERO);
    }

    /// Keep a value that was learned `age` ago.
    pub fn put_as_of(&self, key: K, value: T, age: Duration) {
        let now = Instant::now();
        let at = now.checked_sub(age).unwrap_or(now);
        let wall = SystemTime::now().checked_sub(age).unwrap_or_else(SystemTime::now);
        let mut st = self.lock();
        if st.failure.as_ref().is_some_and(|f| f.key == key && f.at <= at) {
            st.failure = None;
        }
        st.last = Some(Value { key, at, wall, value });
        drop(st);
        self.0.ended.notify_all();
    }

    /// Start fetching `key` behind the caller, unless a fetch for it is already running or its
    /// last one failed too recently. For warming a value before anyone reads it.
    pub fn refresh(&self, key: &K) {
        let mut st = self.lock();
        if !self.failed_recently(&st, key) {
            self.start(&mut st, key);
        }
    }

    /// The value for `key`, from the last one when there is a usable one. Waits only when there
    /// is none, and then at most `wait`.
    pub fn read(&self, key: &K) -> Reading<T> {
        let policy = self.0.policy;
        let deadline = Instant::now() + policy.wait;
        let mut st = self.lock();
        loop {
            if let Some(v) = st.last.as_ref().filter(|v| v.key == *key) {
                let age = v.at.elapsed();
                if age <= policy.usable_for {
                    let reading = Reading::Known {
                        value: v.value.clone(),
                        age,
                        fetched_at: v.wall,
                        last_error: st
                            .failure
                            .as_ref()
                            .filter(|f| f.key == *key && f.at > v.at)
                            .map(|f| f.error.clone()),
                    };
                    if age > policy.current_for && !self.failed_recently(&st, key) {
                        self.start(&mut st, key);
                    }
                    return reading;
                }
            }
            if self.failed_recently(&st, key) {
                let f = st.failure.as_ref().expect("failed_recently found it");
                return Reading::Failed { error: f.error.clone(), age: f.at.elapsed() };
            }
            self.start(&mut st, key);
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Reading::Fetching;
            }
            st = match self.0.ended.wait_timeout(st, left) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<K, T>> {
        self.0.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn failed_recently(&self, st: &State<K, T>, key: &K) -> bool {
        st.failure
            .as_ref()
            .is_some_and(|f| f.key == *key && f.at.elapsed() < self.0.policy.retry_after)
    }

    /// Start a fetch for `key` on its own thread, unless one is running.
    fn start(&self, st: &mut State<K, T>, key: &K) {
        if st.fetching.contains(key) {
            return;
        }
        st.fetching.push(key.clone());
        let shared = self.0.clone();
        let key = key.clone();
        std::thread::spawn(move || {
            // A fetch that panics must still end, or its key would read as "fetching" for ever.
            let result = catch_unwind(AssertUnwindSafe(|| (shared.fetch)(&key)))
                .unwrap_or_else(|_| Err("the fetch panicked".to_string()));
            let mut st = shared.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            st.fetching.retain(|k| *k != key);
            let at = Instant::now();
            match result {
                Ok(value) => {
                    if st.failure.as_ref().is_some_and(|f| f.key == key) {
                        st.failure = None;
                    }
                    st.last = Some(Value { key, at, wall: SystemTime::now(), value });
                }
                Err(error) => st.failure = Some(Failure { key, at, error }),
            }
            drop(st);
            shared.ended.notify_all();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const POLICY: Policy = Policy {
        current_for: Duration::from_secs(2),
        usable_for: Duration::from_secs(30),
        wait: Duration::from_millis(300),
        retry_after: Duration::from_secs(60),
    };

    /// A fetch that takes `delay`, counts its calls, and answers `n * 10` for key `n`.
    fn counted(delay: Duration) -> (Recent<u32, u32>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let recent = Recent::new(POLICY, move |k: &u32| {
            counter.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(delay);
            Ok(k * 10)
        });
        (recent, calls)
    }

    fn value(r: Reading<u32>) -> u32 {
        match r {
            Reading::Known { value, .. } => value,
            other => panic!("expected a value, got {other:?}"),
        }
    }

    /// Wait (briefly) until `calls` reaches `n`: the fetch runs on its own thread.
    fn until_calls(calls: &AtomicUsize, n: usize) {
        let give_up = Instant::now() + Duration::from_secs(5);
        while calls.load(Ordering::SeqCst) < n && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_value_just_learned_is_served_as_it_is() {
        let (recent, calls) = counted(Duration::ZERO);
        recent.put(1, 7);
        assert_eq!(value(recent.read(&1)), 7);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing should have been fetched");
    }

    #[test]
    fn an_older_value_is_served_at_once_while_a_slow_fetch_replaces_it() {
        let (recent, calls) = counted(Duration::from_secs(2));
        recent.put_as_of(1, 7, Duration::from_secs(10));
        let started = Instant::now();
        let reading = recent.read(&1);
        assert!(started.elapsed() < Duration::from_millis(50), "took {:?}", started.elapsed());
        match reading {
            Reading::Known { value, age, .. } => {
                assert_eq!(value, 7);
                assert!(age >= Duration::from_secs(10), "{age:?}");
            }
            other => panic!("{other:?}"),
        }
        until_calls(&calls, 1);
        // A burst of reads while that fetch runs starts no second one.
        for _ in 0..10 {
            recent.read(&1);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn with_nothing_known_a_read_waits_at_most_its_bound() {
        let (recent, _) = counted(Duration::from_secs(2));
        let started = Instant::now();
        assert_eq!(recent.read(&1), Reading::Fetching);
        let took = started.elapsed();
        assert!(took >= POLICY.wait && took < POLICY.wait + Duration::from_millis(200), "took {took:?}");
    }

    #[test]
    fn with_nothing_known_a_quick_fetch_is_waited_for() {
        let (recent, calls) = counted(Duration::from_millis(20));
        assert_eq!(value(recent.read(&3)), 30);
        assert_eq!(value(recent.read(&3)), 30);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_value_for_another_key_is_not_an_answer() {
        let (recent, _) = counted(Duration::ZERO);
        recent.put(1, 7);
        assert_eq!(value(recent.read(&2)), 20);
    }

    #[test]
    fn a_value_too_old_to_use_is_not_served() {
        let (recent, _) = counted(Duration::ZERO);
        recent.put_as_of(1, 7, Duration::from_secs(31));
        assert_eq!(value(recent.read(&1)), 10);
    }

    #[test]
    fn a_failure_is_reported_at_once_and_not_retried_before_its_time() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let recent: Recent<u32, u32> = Recent::new(POLICY, move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("offline".to_string())
        });
        assert!(matches!(recent.read(&1), Reading::Failed { ref error, .. } if error == "offline"));
        let started = Instant::now();
        assert!(matches!(recent.read(&1), Reading::Failed { .. }));
        assert!(started.elapsed() < Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_old_value_whose_refresh_failed_says_so() {
        let recent: Recent<u32, u32> = Recent::new(POLICY, |_| Err("offline".to_string()));
        recent.put_as_of(1, 7, Duration::from_secs(10));
        recent.read(&1);
        let give_up = Instant::now() + Duration::from_secs(5);
        loop {
            if let Reading::Known { last_error: Some(e), value, .. } = recent.read(&1) {
                assert_eq!((e.as_str(), value), ("offline", 7));
                break;
            }
            assert!(Instant::now() < give_up, "the failure was never reported");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_fetch_that_panics_still_ends() {
        let recent: Recent<u32, u32> = Recent::new(POLICY, |_| panic!("boom"));
        assert!(matches!(recent.read(&1), Reading::Failed { ref error, .. } if error == "the fetch panicked"));
    }
}
