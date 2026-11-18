//! Which events get a brief: the clock around daylight saving, who counts as the person, and what
//! a change to the event does to its fingerprint.

use chrono::{Duration, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone};
use yantrik_ipc_contracts::calendar::parse_stamp;

use super::tests::{attendee, event, maya, now, ME};
use super::window::{fingerprint, meeting_of, to_unix_in, Own};

/// A clock with daylight saving, as the UK keeps it in 2026: GMT, then BST (+1) from 01:00 UTC on
/// 29 March to 01:00 UTC on 25 October. Built here because the tests must not depend on the
/// machine's own zone.
#[derive(Clone, Copy, Debug)]
struct London;

impl London {
    fn gmt() -> FixedOffset {
        FixedOffset::east_opt(0).expect("offset")
    }
    fn bst() -> FixedOffset {
        FixedOffset::east_opt(3600).expect("offset")
    }
}

impl TimeZone for London {
    type Offset = FixedOffset;
    fn from_offset(_: &FixedOffset) -> London {
        London
    }
    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).expect("noon"))
    }
    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        let fits: Vec<FixedOffset> = [London::bst(), London::gmt()]
            .into_iter()
            .filter(|off| self.offset_from_utc_datetime(&(*local - Duration::seconds(i64::from(off.local_minus_utc())))) == *off)
            .collect();
        match fits[..] {
            [] => LocalResult::None,
            [one] => LocalResult::Single(one),
            [first, second] => LocalResult::Ambiguous(first, second),
            _ => unreachable!("two offsets at most"),
        }
    }
    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).expect("noon"))
    }
    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
        let on = parse_stamp("2026-03-29T01:00:00").expect("stamp");
        let off = parse_stamp("2026-10-25T01:00:00").expect("stamp");
        if *utc >= on && *utc < off { London::bst() } else { London::gmt() }
    }
}

fn wall(text: &str) -> NaiveDateTime {
    parse_stamp(text).expect("stamp")
}

fn at_utc(text: &str) -> i64 {
    chrono::Utc.from_utc_datetime(&wall(text)).timestamp()
}

#[test]
fn a_time_in_the_hour_the_clocks_skip_is_read_as_the_hour_after_it() {
    // 01:30 never happens on 29 March: at 01:00 GMT the clocks go to 02:00 BST. It is shown at
    // 02:30 BST, which is 01:30 UTC.
    assert_eq!(to_unix_in(&London, wall("2026-03-29T01:30:00")), Some(at_utc("2026-03-29T01:30:00")));
    // 00:30 is ordinary GMT; 02:30 is ordinary BST (01:30 UTC), the same instant as the gap's.
    assert_eq!(to_unix_in(&London, wall("2026-03-29T00:30:00")), Some(at_utc("2026-03-29T00:30:00")));
    assert_eq!(to_unix_in(&London, wall("2026-03-29T02:30:00")), Some(at_utc("2026-03-29T01:30:00")));
}

#[test]
fn a_time_in_the_hour_that_happens_twice_is_read_as_its_first() {
    // 01:30 happens twice on 25 October: first in BST (00:30 UTC), then in GMT (01:30 UTC).
    assert_eq!(to_unix_in(&London, wall("2026-10-25T01:30:00")), Some(at_utc("2026-10-25T00:30:00")));
    // Either side of it is unambiguous.
    assert_eq!(to_unix_in(&London, wall("2026-10-25T00:30:00")), Some(at_utc("2026-10-24T23:30:00")));
    assert_eq!(to_unix_in(&London, wall("2026-10-25T02:30:00")), Some(at_utc("2026-10-25T02:30:00")));
}

#[test]
fn the_person_is_known_by_their_name_when_their_address_is_not_one_this_machine_has() {
    // No email account is set up, so no address is the person's, but the invitation lists them
    // with an address of their own.
    let alone = Own::new(Vec::<String>::new(), "Pat");
    let n = now();
    let e = event("e1", "Focus", n + Duration::minutes(5), vec![attendee("Pat", "pat@somewhere.test")]);
    assert!(meeting_of(&e, &alone).is_none(), "only the person is on it: no brief");
    let with_maya = event("e2", "Q4", n + Duration::minutes(5), vec![attendee("pat", "pat@somewhere.test"), maya()]);
    let meeting = meeting_of(&with_maya, &alone).expect("a meeting");
    assert_eq!(meeting.people.len(), 1, "the person is not someone to prepare for");
    assert_eq!(meeting.people[0].name, "Maya Chen");
    assert!(meeting_of(&event("e3", "Q4", n + Duration::minutes(5), vec![attendee("Pat", ME)]), &Own::new([ME.to_string()], "Someone")).is_none());
}

#[test]
fn what_a_card_says_is_what_its_fingerprint_depends_on() {
    let (own, n) = (Own::new([ME.to_string()], "Pat"), now());
    let base = event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()]);
    let print = |e: &yantrik_ipc_contracts::calendar::CalendarEvent| fingerprint(&meeting_of(e, &own).expect("meeting"));
    let same = print(&base);
    assert_eq!(print(&base.clone()), same);
    let mut longer = base.clone();
    longer.end = (n + Duration::minutes(70)).format("%Y-%m-%dT%H:%M:%S").to_string();
    assert_ne!(print(&longer), same, "a moved end time");
    let mut elsewhere = base.clone();
    elsewhere.location = Some("Room 9".into());
    assert_ne!(print(&elsewhere), same, "a moved room");
    let mut described = base.clone();
    described.description = "Agenda".into();
    assert_ne!(print(&described), same, "a new description");
}
