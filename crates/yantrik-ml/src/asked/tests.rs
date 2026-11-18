use super::*;

#[test]
fn a_thread_starts_out_asked() {
    assert!(!started_by_itself());
    assert_eq!(header_for(true), None);
}

#[test]
fn the_guard_sets_the_mark_and_puts_the_earlier_value_back_nested() {
    assert!(!started_by_itself());
    {
        let _outer = self_started();
        assert!(started_by_itself());
        {
            // Inside a self-started path something is carried on as asked: it is restored after.
            let _asked = adopt(false);
            assert!(!started_by_itself());
        }
        assert!(started_by_itself(), "the inner guard restored the outer value");
        {
            let _inner = self_started();
            assert!(started_by_itself());
        }
        assert!(started_by_itself(), "a second self-started inside leaves it set");
    }
    assert!(!started_by_itself(), "back to asked when the outer guard drops");
}

#[test]
fn another_thread_does_not_see_the_mark_unless_it_is_carried() {
    let _mine = self_started();
    let plain = std::thread::spawn(started_by_itself).join().unwrap();
    assert!(!plain, "a new thread is asked");
    let carried = carry();
    let kept = std::thread::spawn(move || {
        let _g = adopt(carried);
        started_by_itself()
    })
    .join()
    .unwrap();
    assert!(kept, "a thread that adopts what was carried has the mark");
    let after = std::thread::spawn(move || {
        drop(adopt(carried));
        started_by_itself()
    })
    .join()
    .unwrap();
    assert!(!after, "and it is gone when the guard drops");
}

#[test]
fn only_the_gateways_client_carries_the_mark_as_a_header() {
    let _mine = self_started();
    assert_eq!(header_for(true), Some(("x-yantrik-asked", "self")));
    assert_eq!(header_for(false), None, "a provider never sees it");
}

#[test]
fn skipping_follows_the_account_and_is_this_threads_own() {
    assert!(!skip_self_started("a test"), "no shell, no check: not tight");
    let tight = tight_for_this_thread(true);
    assert!(skip_self_started("a test"));
    assert!(!std::thread::spawn(|| skip_self_started("a test")).join().unwrap(), "another thread is not told");
    {
        let _roomy = tight_for_this_thread(false);
        assert!(!skip_self_started("a test"));
    }
    assert!(skip_self_started("a test"), "the earlier answer came back");
    drop(tight);
    assert!(!skip_self_started("a test"));
}

#[test]
fn the_gateways_refusal_is_known_by_status_and_code_and_by_nothing_else() {
    let body = serde_json::json!({"error": {"message": "This account allows 2 requests a minute.", "type": "yantrik_gateway", "code": "kept_for_asked"}}).to_string();
    let k = kept(429, &body).expect("the refusal");
    assert_eq!(k.said, "This account allows 2 requests a minute.");
    assert_eq!(kept_said(&k).as_deref(), Some("This account allows 2 requests a minute."));
    assert_eq!(kept(400, &body), None, "the status is part of it");
    let other = serde_json::json!({"error": {"message": "slow down", "code": "rate_limit"}}).to_string();
    assert_eq!(kept(429, &other), None, "a provider's own 429 is not this");
    assert_eq!(kept(429, "not json"), None);
    assert_eq!(kept_said(&std::io::Error::other("x")), None);
}
