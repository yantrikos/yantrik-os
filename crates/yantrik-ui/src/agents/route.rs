//! The Overview's route (Pranab, 27 September): one run, drawn as a line of stations, not every
//! agent at once. It starts at what the run was asked. Each call, question and approval on the way
//! is a station, and an agent it started branches off where it started. It ends where the run
//! ended, or at the station it is at now, with the reply still ahead.
//!
//! Built here from the session, so it is tested without a screen; agents_route.slint draws it.

use super::model::{Agent, ApprovalOutcome, CallState, Item, State, Turn};

/// Stations kept on one route. A longer run folds its earliest passed stations into one.
const MOST: usize = 40;
/// A station's title, cut to one line.
const TITLE_CHARS: usize = 90;

/// One station.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stop {
    /// What pressing it opens: the session item's key (`t3.2`), or a branch's agent id.
    pub key: String,
    /// start | call | question | approval | branch | now | end | more
    pub kind: &'static str,
    /// passed | here | waiting | failed | lost | ahead
    pub state: &'static str,
    pub title: String,
    pub sub: String,
    /// The line into this station's row and out of it: "" (none), done, live, ahead. A branch's
    /// row carries the line past it; its own spur is drawn in its own state.
    pub track_in: &'static str,
    pub track_out: &'static str,
    /// Where the one train on the live stretch is in this row's half into the station and out of
    /// it: the share of its run where that half begins, or -1 when the half is not live. The
    /// stretch can cross several rows (a branch between), and the train crosses them in turn.
    pub train_in: f32,
    pub train_out: f32,
    /// The share of the train's run one half-row takes.
    pub train_span: f32,
}

/// One run as a route.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub title: String,
    /// "Running · 1m 20s · 4 stations passed", "Done in 2m 3s · 6 calls".
    pub summary: String,
    /// Still moving: the only route drawn animated.
    pub live: bool,
    pub stops: Vec<Stop>,
}

/// The route of run `run` of `a` (its latest when `None`), with `children`, the agents it started,
/// as branches. None when it has run nothing yet.
pub fn route(a: &Agent, run: Option<u64>, children: &[&Agent], now: u64) -> Option<Route> {
    let turn = match run {
        Some(n) => a.turns.iter().find(|t| t.n == n)?,
        None => a.turns.last()?,
    };
    let live = turn.open();
    let mut stops = vec![Stop {
        key: format!("t{}", turn.n),
        kind: "start",
        state: "passed",
        title: one_line(if turn.prompt.trim().is_empty() { "Started" } else { &turn.prompt }),
        sub: format!("Asked · {}", ago(now, turn.started)),
        ..Default::default()
    }];

    // The stations on the way, each with when it happened, so branches go in where they started.
    let mut on_the_way: Vec<(u64, Stop)> = turn
        .items
        .iter()
        .enumerate()
        .filter_map(|(j, item)| station(turn, j, item, now))
        .collect();
    let ends = turn.ended.unwrap_or(u64::MAX);
    for child in children.iter().filter(|c| c.meta.started >= turn.started && c.meta.started <= ends) {
        on_the_way.push((child.meta.started, branch(child)));
    }
    on_the_way.sort_by_key(|(at, _)| *at);
    let calls = on_the_way.iter().filter(|(_, s)| s.kind == "call").count();
    let passed = on_the_way.iter().filter(|(_, s)| s.state == "passed").count();
    stops.extend(fold(on_the_way.into_iter().map(|(_, s)| s).collect()));

    if live {
        // Where the run is now, when no station on the way holds it: the mind is thinking.
        // A branch at work is its own agent; the run itself is still somewhere on its line.
        if !stops.iter().any(|s| s.kind != "branch" && (s.state == "here" || s.state == "waiting")) {
            let doing = if a.status.trim().is_empty() { "Thinking" } else { a.status.trim() };
            stops.push(Stop {
                key: String::new(),
                kind: "now",
                state: "here",
                title: one_line(doing),
                sub: ago(now, a.since),
                ..Default::default()
            });
        }
        stops.push(Stop { key: String::new(), kind: "end", state: "ahead", title: "Reply".into(), sub: String::new(), ..Default::default() });
    } else {
        let took = span(turn.ended.unwrap_or(now).saturating_sub(turn.started));
        let (state, title) = match (turn.lost, turn.ok) {
            (true, _) => ("lost", "Stopped with the desktop"),
            (_, Some(false)) => ("failed", "Failed"),
            _ => ("passed", "Done"),
        };
        stops.push(Stop { key: String::new(), kind: "end", state, title: title.into(), sub: format!("after {took}"), ..Default::default() });
    }

    lay_track(&mut stops, live);
    let took = span(turn.ended.unwrap_or(now).saturating_sub(turn.started));
    let summary = if live {
        let doing = if a.state == State::WaitingForYou { "Waiting for you" } else { "Running" };
        format!("{doing} · {took} · {passed} passed")
    } else {
        let end = &stops.last().expect("a route has an end").title;
        format!("{end} · {took} · {}", count(calls, "call"))
    };
    Some(Route { title: one_line(&turn.prompt), summary, live, stops })
}

/// The line between stations. The stretch into a station on the line is ahead when the station
/// is, live when the run is at it now, and done otherwise; a branch's row carries the stretch
/// that passes it.
fn lay_track(stops: &mut [Stop], live: bool) {
    let into = |s: &Stop| match s.state {
        "ahead" => "ahead",
        "here" | "waiting" if live => "live",
        _ => "done",
    };
    let next_on_line = |stops: &[Stop], from: usize| stops[from..].iter().find(|s| s.kind != "branch").map(into).unwrap_or("");
    for i in 0..stops.len() {
        let (track_in, track_out) = if stops[i].kind == "branch" {
            let passing = next_on_line(stops, i + 1);
            (passing, passing)
        } else {
            let track_in = if i == 0 { "" } else { into(&stops[i]) };
            (track_in, next_on_line(stops, i + 1))
        };
        stops[i].track_in = track_in;
        stops[i].track_out = track_out;
    }
    // One train, over every live half-row in order.
    let halves = stops.iter().map(|s| (s.track_in == "live") as usize + (s.track_out == "live") as usize).sum::<usize>();
    let span = if halves == 0 { 0.0 } else { 1.0 / halves as f32 };
    let mut at = 0.0;
    for s in stops.iter_mut() {
        s.train_span = span;
        s.train_in = -1.0;
        s.train_out = -1.0;
        if s.track_in == "live" {
            s.train_in = at;
            at += span;
        }
        if s.track_out == "live" {
            s.train_out = at;
            at += span;
        }
    }
}

/// A session item as a station, with when it happened; None for what is not one (text, thinking,
/// notes: the conversation, not the route).
fn station(turn: &Turn, j: usize, item: &Item, now: u64) -> Option<(u64, Stop)> {
    let key = format!("t{}.{j}", turn.n);
    match item {
        Item::Card(c) => {
            // The call's own summary already says how many times it ran (×n); adding it again
            // read "os_act ×2 ×2" on VM 520.
            let title = one_line(&station_name(&c.as_call().summary()));
            let (state, sub) = match c.state {
                CallState::Running => ("here", format!("running · {}", span(now.saturating_sub(c.started)))),
                CallState::Ok => ("passed", took(c.started, c.ended, &c.summary)),
                CallState::Failed => ("failed", took(c.started, c.ended, &c.summary)),
                CallState::Interrupted => ("lost", "never said how it went".into()),
                CallState::Untold => ("passed", "told in its text".into()),
            };
            Some((c.started, Stop { key, kind: "call", state, title, sub, ..Default::default() }))
        }
        Item::Question(q) => {
            let (state, sub) = if q.waiting() {
                ("waiting", "waiting for your answer".to_string())
            } else if !q.answer.is_empty() {
                ("passed", format!("You answered: {}", one_line(&q.answer)))
            } else {
                ("lost", q.closed.clone())
            };
            Some((q.asked, Stop { key, kind: "question", state, title: one_line(&q.prompt), sub, ..Default::default() }))
        }
        Item::Approval(ap) => {
            let (state, sub) = match ap.outcome {
                ApprovalOutcome::Pending => ("waiting", "waiting for your yes"),
                ApprovalOutcome::Allowed => ("passed", "allowed"),
                ApprovalOutcome::Denied => ("lost", "denied"),
                ApprovalOutcome::Expired => ("lost", "nobody answered in time"),
                ApprovalOutcome::Withdrawn => ("lost", "withdrawn"),
            };
            Some((ap.asked, Stop { key, kind: "approval", state, title: format!("Approval: {}", ap.what), sub: sub.into(), ..Default::default() }))
        }
        Item::Text(_) | Item::Thinking(_) | Item::Note(_) => None,
    }
}

/// A call as a station names it: this desktop's own tools without the prefix an MCP client puts
/// on them ("mcp_yantrik_os_os_act" is `os_act`); anything else as the harness named it.
fn station_name(summary: &str) -> String {
    for prefix in ["mcp_yantrik_os_", "mcp_yantrik-os_", "mcp__yantrik-os__", "mcp__yantrik_os__"] {
        if let Some(rest) = summary.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    summary.to_string()
}

/// An agent this run started, as a branch off the line.
fn branch(child: &Agent) -> Stop {
    let state = match child.state {
        State::Thinking | State::RunningTool => "here",
        State::WaitingForYou => "waiting",
        State::Done => "passed",
        State::Failed => "failed",
        State::HarnessGone => "lost",
        State::Idle => "ahead",
    };
    let who = child.meta.role.as_ref().map(|r| r.name.clone()).unwrap_or_else(|| child.meta.mind.clone());
    Stop {
        key: child.meta.id.0.clone(),
        kind: "branch",
        state,
        title: format!("{who}: {}", one_line(&child.meta.title)),
        sub: child.state.label().into(),
        ..Default::default()
    }
}

/// At most MOST stations on the way: the earliest passed ones fold into one, so where the run is
/// now and what it did last stay on the line.
fn fold(stops: Vec<Stop>) -> Vec<Stop> {
    if stops.len() <= MOST {
        return stops;
    }
    let over = stops.len() - MOST + 1;
    let folded = stops.iter().take(over).filter(|s| s.state == "passed").count();
    if folded < over {
        // Something early still needs the eye (waiting, failed): keep it all.
        return stops;
    }
    let mut out = vec![Stop {
        key: String::new(),
        kind: "more",
        state: "passed",
        title: format!("{over} earlier stations"),
        sub: "open the list for every step".into(),
        ..Default::default()
    }];
    out.extend(stops.into_iter().skip(over));
    out
}

fn took(started: u64, ended: Option<u64>, summary: &str) -> String {
    let t = ended.map(|e| span(e.saturating_sub(started))).unwrap_or_default();
    match (t.is_empty(), summary.trim().is_empty()) {
        (_, true) => t,
        (true, false) => one_line(summary),
        (false, false) => format!("{t} · {}", one_line(summary)),
    }
}

/// "3s", "1m 20s", "2h 5m".
pub fn span(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 if secs % 60 == 0 => format!("{}m", secs / 60),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// How long ago, in whole minutes past the first: a finished route's words then change once a
/// minute, not every second, so leaving it on screen costs next to nothing (#68).
fn ago(now: u64, at: u64) -> String {
    match now.saturating_sub(at) {
        0..=4 => "just now".into(),
        secs @ 5..=59 => format!("{secs}s ago"),
        secs @ 60..=3599 => format!("{}m ago", secs / 60),
        secs => format!("{}h ago", secs / 3600),
    }
}

fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TITLE_CHARS {
        return flat;
    }
    format!("{}…", flat.chars().take(TITLE_CHARS - 1).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::super::model::{AgentMeta, Event, Provenance};
    use super::super::store::Store;
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn store() -> (Store, Arc<AtomicU64>) {
        let clock = Arc::new(AtomicU64::new(1_790_000_000));
        let reading = clock.clone();
        (Store::with_clock(Box::new(move || reading.load(Ordering::SeqCst))), clock)
    }

    fn id(s: &str) -> super::super::model::AgentId {
        super::super::model::AgentId(s.to_string())
    }

    fn start(call: &str, name: &str, args: serde_json::Value) -> Event {
        Event::ToolStart { call: call.into(), name: name.into(), target: String::new(), args }
    }

    fn end(call: &str, ok: bool) -> Event {
        Event::ToolEnd { call: call.into(), ok, summary: String::new(), exit_code: Some(if ok { 0 } else { 1 }) }
    }

    fn shape(r: &Route) -> Vec<(&'static str, &'static str)> {
        r.stops.iter().map(|s| (s.kind, s.state)).collect()
    }

    #[test]
    fn a_live_run_is_a_line_of_passed_stations_to_the_one_it_is_at_with_the_reply_ahead() {
        let (mut s, clock) = store();
        let pi = id("pi:c-tidy");
        s.open_turn(&pi, "tidy the photos folder");
        s.event(&pi, &start("t1", "bash", json!({"command": "ls ~/Pictures"})), Provenance::Reported);
        clock.fetch_add(3, Ordering::SeqCst);
        s.event(&pi, &end("t1", true), Provenance::Reported);
        s.event(&pi, &start("t2", "bash", json!({"command": "fdupes -r ~/Pictures"})), Provenance::Reported);
        clock.fetch_add(80, Ordering::SeqCst);

        let r = route(s.agent(&pi).unwrap(), None, &[], clock.load(Ordering::SeqCst)).unwrap();
        assert!(r.live);
        assert_eq!(shape(&r), vec![("start", "passed"), ("call", "passed"), ("call", "here"), ("end", "ahead")]);
        assert_eq!(r.stops[1].sub, "3s");
        assert_eq!(r.stops[2].sub, "running · 1m 20s");
        assert_eq!(r.stops[1].key, "t1.0", "a station opens its own card");
        assert_eq!(r.summary, "Running · 1m 23s · 1 passed");
    }

    #[test]
    fn a_run_between_calls_is_at_a_thinking_station_and_one_that_asks_is_waiting_on_the_person() {
        let (mut s, clock) = store();
        let pi = id("pi:c-dl");
        s.open_turn(&pi, "clean up Downloads");
        s.event(&pi, &start("t1", "bash", json!({"command": "ls"})), Provenance::Reported);
        s.event(&pi, &end("t1", true), Provenance::Reported);
        let now = clock.load(Ordering::SeqCst);
        let r = route(s.agent(&pi).unwrap(), None, &[], now).unwrap();
        assert_eq!(shape(&r), vec![("start", "passed"), ("call", "passed"), ("now", "here"), ("end", "ahead")]);

        let ask = Event::Request { request_id: "r1".into(), prompt: "Delete 3 old installers?".into(), options: vec![] };
        s.event(&pi, &ask, Provenance::Reported);
        let r = route(s.agent(&pi).unwrap(), None, &[], now).unwrap();
        assert_eq!(shape(&r), vec![("start", "passed"), ("call", "passed"), ("question", "waiting"), ("end", "ahead")]);
        assert!(r.summary.starts_with("Waiting for you"), "{}", r.summary);
    }

    #[test]
    fn a_finished_run_ends_at_how_it_ended_and_is_not_live() {
        let (mut s, clock) = store();
        let pi = id("pi:c-web");
        s.open_turn(&pi, "read the top stories");
        s.event(&pi, &start("t1", "web_fetch", json!({"url": "https://example.com"})), Provenance::Reported);
        clock.fetch_add(2, Ordering::SeqCst);
        s.event(&pi, &end("t1", false), Provenance::Reported);
        clock.fetch_add(5, Ordering::SeqCst);
        s.close_turn(&pi, false);

        let r = route(s.agent(&pi).unwrap(), None, &[], clock.load(Ordering::SeqCst) + 600).unwrap();
        assert!(!r.live);
        assert_eq!(shape(&r), vec![("start", "passed"), ("call", "failed"), ("end", "failed")]);
        assert_eq!(r.stops[2].sub, "after 7s", "measured to when it ended, not to now");
        assert_eq!(r.summary, "Failed · 7s · 1 call");
    }

    #[test]
    fn one_run_of_a_chat_is_that_turn_alone_and_an_agent_it_started_is_a_branch() {
        let (mut s, clock) = store();
        let main = id("hermes:main");
        s.open_chat_turn(&main, "what's the weather");
        s.close_turn(&main, true);
        clock.fetch_add(60, Ordering::SeqCst);
        s.open_chat_turn(&main, "review the release notes");
        s.event(&main, &start("t1", "read_file", json!({"path": "NOTES.md"})), Provenance::Reported);
        s.event(&main, &end("t1", true), Provenance::Reported);
        clock.fetch_add(1, Ordering::SeqCst);
        let mut kid = AgentMeta::new(id("hermes:c-review"), "hermes");
        kid.parent = Some(main.clone());
        kid.title = "Check the notes against the log".into();
        kid.started = clock.load(Ordering::SeqCst);
        s.upsert_agent(kid);
        s.open_turn(&id("hermes:c-review"), "Check the notes against the log");

        let a = s.agent(&main).unwrap();
        let second = a.turns.last().unwrap().n;
        let child = s.agent(&id("hermes:c-review")).unwrap();
        let r = route(a, Some(second), &[child], clock.load(Ordering::SeqCst)).unwrap();
        assert_eq!(r.title, "review the release notes");
        assert_eq!(
            shape(&r),
            vec![("start", "passed"), ("call", "passed"), ("branch", "here"), ("now", "here"), ("end", "ahead")],
            "the branch works on its own; the run itself is still at a station of its own"
        );
        let track: Vec<_> = r.stops.iter().map(|s| (s.track_in, s.track_out)).collect();
        assert_eq!(
            track,
            vec![("", "done"), ("done", "live"), ("live", "live"), ("live", "ahead"), ("ahead", "")],
            "done up to the last station passed, live into where it is now, ahead to the reply"
        );
        let train: Vec<_> = r.stops.iter().map(|s| (s.train_in, s.train_out)).collect();
        assert_eq!(
            train,
            vec![(-1.0, -1.0), (-1.0, 0.0), (0.25, 0.5), (0.75, -1.0), (-1.0, -1.0)],
            "one train over the four live half-rows, in order"
        );
        assert_eq!(r.stops[2].key, "hermes:c-review", "a branch opens the agent it started");
        assert!(r.stops[2].title.starts_with("hermes: Check the notes"), "{}", r.stops[2].title);

        let first = a.turns.first().unwrap().n;
        let r = route(a, Some(first), &[child], clock.load(Ordering::SeqCst)).unwrap();
        assert_eq!(shape(&r), vec![("start", "passed"), ("end", "passed")], "the earlier turn started no agent");
    }

    #[test]
    fn a_long_run_folds_its_earliest_passed_stations_and_keeps_the_rest() {
        let (mut s, clock) = store();
        let pi = id("pi:c-long");
        s.open_turn(&pi, "index everything");
        for n in 0..60 {
            let call = format!("t{n}");
            s.event(&pi, &start(&call, "read_file", json!({"path": format!("f{n}")})), Provenance::Reported);
            clock.fetch_add(1, Ordering::SeqCst);
            s.event(&pi, &end(&call, true), Provenance::Reported);
        }
        let r = route(s.agent(&pi).unwrap(), None, &[], clock.load(Ordering::SeqCst)).unwrap();
        assert_eq!(r.stops[1].kind, "more");
        assert_eq!(r.stops[1].title, "21 earlier stations");
        assert_eq!(r.stops.len(), 1 + MOST + 2, "start, the kept stations, where it is now, the reply");
        assert!(r.summary.ends_with("60 passed"), "the summary counts what was folded too: {}", r.summary);
    }

    #[test]
    fn nothing_run_yet_is_no_route() {
        let (mut s, _) = store();
        let pi = id("pi:c-new");
        s.upsert_agent(AgentMeta::new(pi.clone(), "pi"));
        assert!(route(s.agent(&pi).unwrap(), None, &[], 0).is_none());
    }

    #[test]
    fn a_station_is_named_by_the_call_once_without_this_desktops_mcp_prefix() {
        assert_eq!(station_name("mcp_yantrik_os_os_act ×2"), "os_act ×2");
        assert_eq!(station_name("mcp__yantrik-os__web_go \"example.com\""), "web_go \"example.com\"");
        assert_eq!(station_name("mcp_github_search"), "mcp_github_search", "another server's tool keeps its name");
        assert_eq!(station_name("bash"), "bash");
    }

    #[test]
    fn spans_read_as_a_person_says_them() {
        assert_eq!(span(3), "3s");
        assert_eq!(span(60), "1m");
        assert_eq!(span(83), "1m 23s");
        assert_eq!(span(7_500), "2h 5m");
    }
}
