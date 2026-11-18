//! The transport and the intents that play, as mpv reports them: play, pause, seek, the volume,
//! next and previous, auto-advance, a player that is missing, and one that dies.

use serde_json::json;

use super::{open, open_with, Setup};

#[test]
fn with_no_mpv_it_is_said_in_plain_words_on_screen_and_to_the_caller() {
    let Some(f) = open_with("no-mpv", Setup { player: false, ..Setup::default() }) else { return };
    let err = f.act("play", json!({})).unwrap_err();
    assert!(err.contains("mpv, is not installed"), "{err}");
    assert_eq!(f.notice(), err, "said twice: to the caller and in the window");
    assert_eq!(f.state()["player"], "unavailable");
    assert_eq!(f.state()["player_problem"], json!(err));
    // The button says the same.
    f.app.run(crate::Command::DismissNotice);
    f.app.ui(|ui| ui.invoke_play_pause());
    f.quiet();
    assert!(f.notice().contains("mpv, is not installed"), "{}", f.notice());
}

#[test]
fn play_pause_seek_and_the_volume_answer_with_what_mpv_reports() {
    let Some(f) = open("transport") else { return };
    let played = f.act("play", json!({"index": 2})).unwrap();
    assert_eq!(played["reported_by"], "mpv");
    assert_eq!(played["playing"], true, "{played}");
    assert_eq!(played["track"], "Alpha 001");
    assert_eq!(played["volume"], 40.0, "mpv started at the kept volume");
    assert!(played.get("player_moved_since").is_none(), "{played}");
    assert!(f.notice().is_empty(), "{}", f.notice());
    f.until("the window plays", |f| f.playing());
    assert_eq!(f.app.ui(|ui| ui.get_now_title().to_string()), "Alpha 001");
    assert_eq!(f.app.ui(|ui| ui.get_time_total().to_string()), "3:20");
    assert_eq!(f.app.ui(|ui| ui.get_queue_current_index()), 1);
    assert_eq!(f.status(), "Playing \u{201c}Alpha 001\u{201d}");

    let paused = f.act("pause", json!({})).unwrap();
    assert_eq!(paused["paused"], true);
    f.until("the window shows it paused", |f| !f.playing());
    assert!(f.summary().starts_with("Music \u{2014} paused on \u{201c}Alpha 001\u{201d} at 0:00"), "{}", f.summary());
    let resumed = f.act("play", json!({})).unwrap();
    assert_eq!(resumed["playing"], true);
    assert!(resumed["path"].as_str().is_some_and(|p| p.ends_with("Alpha 001.mp3")), "it resumed, not restarted: {resumed}");

    let sought = f.act("seek", json!({"seconds": 50.0})).unwrap();
    assert_eq!(sought["position"], 50.0);
    f.until("the clock moves", |f| f.app.ui(|ui| ui.get_time_current().to_string()) == "0:50");
    assert!((f.app.ui(|ui| ui.get_progress()) - 0.25).abs() < 0.01, "the bar is time-pos over duration");
    let past = f.act("seek", json!({"seconds": 999.0})).unwrap_err();
    assert_eq!(past, "The track is 3:20 long; 999 seconds is past its end.");
    assert_eq!(f.notice(), past);
    let negative = f.act("seek", json!({"seconds": -1.0})).unwrap_err();
    assert_eq!(negative, "`seconds` is a point in the track, 0 or more.");

    let loud = f.act("set_volume", json!({"percent": 65})).unwrap();
    assert_eq!(loud["volume"], 65.0);
    f.until("the slider follows", |f| (f.app.ui(|ui| ui.get_volume()) - 0.65).abs() < 0.001);
    assert_eq!(f.act("set_volume", json!({"percent": 140})).unwrap_err(), "`percent` is from 0 to 100, not 140.");
    f.app.run_writes();
    assert_eq!(f.app.committed_settings().volume, 65.0, "the volume is kept for next time");

    let next = f.act("next", json!({})).unwrap();
    assert_eq!(next["track"], "Alpha 002");
    // Under three seconds in, previous goes to the track before.
    let back = f.act("previous", json!({})).unwrap();
    assert_eq!(back["track"], "Alpha 001");

    let d = f.state();
    assert_eq!(d["track"], "Alpha 001");
    assert_eq!(d["playing"], true);
    assert_eq!(d["duration"], 200.0);
    assert_eq!(d["volume"], 65.0);
    assert_eq!(d["queue_length"], 300);
    assert_eq!(d["queue_position"], 2);
    assert_eq!(d["library_size"], 300);
    assert_eq!(d["player"], "running");
    assert!(f.summary().starts_with("Music \u{2014} playing \u{201c}Alpha 001\u{201d} (0:00 of 3:20), library 300 tracks"), "{}", f.summary());
}

#[test]
fn previous_more_than_three_seconds_in_goes_back_to_the_start() {
    let Some(f) = open("previous") else { return };
    f.act("play", json!({"index": 5})).unwrap();
    f.act("seek", json!({"seconds": 42.0})).unwrap();
    let back = f.act("previous", json!({})).unwrap();
    assert_eq!(back["position"], 0.0, "{back}");
    assert!(back.get("track").is_none(), "the same track, back to its start: {back}");
    assert_eq!(f.app.model(|m| m.queue.current_index()), Some(4), "the queue did not move");
    // At the top of the queue, previous goes to the start of what is playing.
    f.act("play", json!({"index": 1})).unwrap();
    let top = f.act("previous", json!({})).unwrap();
    assert_eq!(top["position"], 0.0, "{top}");
}

#[test]
fn the_queue_refuses_past_its_ends_in_the_windows_words() {
    let Some(f) = open("queue-ends") else { return };
    assert_eq!(f.act("next", json!({})).unwrap_err(), "The queue is empty: pick a track in the library first.");
    assert_eq!(f.act("previous", json!({})).unwrap_err(), "The queue is empty: pick a track in the library first.");
    f.act("play", json!({"index": 300})).unwrap();
    assert_eq!(f.act("next", json!({})).unwrap_err(), "This is the last track in the queue.");
    assert_eq!(f.act("play", json!({"index": 301})).unwrap_err(), "There is no track 301 in the library list.");
    assert_eq!(f.act("play", json!({"index": 0})).unwrap_err(), "`index` counts from 1.");
    assert_eq!(f.notice(), "`index` counts from 1.", "and the window says so too");
    assert_eq!(f.app.model(|m| m.queue.current_index()), Some(299), "a refused play leaves the queue as it was");
}

#[test]
fn the_buttons_play_from_the_list_the_queue_and_the_transport() {
    let Some(f) = open("buttons") else { return };
    f.app.ui(|ui| ui.invoke_play_track_index(9));
    f.until("row 10 plays", |f| f.playing_path().is_some_and(|p| p.ends_with("Alpha 009.mp3")));
    f.app.ui(|ui| ui.invoke_next_track());
    f.until("next plays", |f| f.playing_path().is_some_and(|p| p.ends_with("Alpha 010.mp3")));
    f.app.ui(|ui| ui.invoke_queue_play_index(2));
    f.until("the queue's row 3 plays", |f| f.playing_path().is_some_and(|p| p.ends_with("Alpha 002.mp3")));
    f.app.ui(|ui| ui.invoke_queue_play_index(400));
    f.quiet();
    assert_eq!(f.notice(), "That track is no longer in the queue.");
    f.app.ui(|ui| ui.invoke_seek(0.5));
    f.until("the bar seeks", |f| f.app.ui(|ui| ui.get_time_current().to_string()) == "1:40");
    f.app.ui(|ui| ui.invoke_play_pause());
    f.until("paused", |f| !f.playing());
    f.app.ui(|ui| ui.invoke_play_pause());
    f.until("playing again", |f| f.playing());
    f.app.ui(|ui| ui.invoke_toggle_mute());
    f.until("muted, by mpv's word", |f| f.app.ui(|ui| ui.get_volume_muted()));
    f.app.ui(|ui| ui.invoke_volume_changed(0.3));
    f.until("the volume, by mpv's word", |f| f.state()["volume"].as_f64().is_some_and(|v| (v - 30.0).abs() < 0.001));
    f.app.ui(|ui| ui.invoke_stop());
    f.until("stopped", |f| f.status() == "Stopped.");
    assert!(!f.playing());
    assert_eq!(f.state()["track"], json!(null));
    f.app.ui(|ui| ui.invoke_stop());
    f.quiet();
    assert_eq!(f.notice(), "Nothing is playing, so there is nothing to stop.");
}

#[test]
fn muting_with_no_player_is_kept_until_one_starts() {
    let Some(f) = open("mute-early") else { return };
    f.app.ui(|ui| ui.invoke_toggle_mute());
    f.quiet();
    assert!(f.app.ui(|ui| ui.get_volume_muted()));
    assert_eq!(f.state()["muted"], true);
    let played = f.act("play", json!({"index": 1})).unwrap();
    assert_eq!(played["muted"], true, "the player started muted: {played}");
}

#[test]
fn a_track_that_ends_plays_the_next_one() {
    let Some(f) = open("advance") else { return };
    // The stand-in ends a file named `short` as soon as it starts.
    std::fs::write(f.music.join("Beta/zz short one.mp3"), b"").unwrap();
    let after = f.music.join("Beta/zz then this.mp3");
    std::fs::write(&after, b"").unwrap();
    f.act("rescan", json!({})).unwrap();
    f.app.ui(|ui| ui.invoke_browse_filter_changed("zz".into()));
    assert_eq!(f.rows(), 2);
    f.app.ui(|ui| ui.invoke_play_track_index(0));
    let want = after.to_string_lossy().into_owned();
    f.until("the end of `zz short one` played the next one", |f| f.playing_path().as_deref() == Some(want.as_str()));
    assert_eq!(f.app.model(|m| m.queue.current_index()), Some(1));
    // At the end of the queue it stays there.
    assert_eq!(f.act("next", json!({})).unwrap_err(), "This is the last track in the queue.");
}

#[test]
fn the_search_narrows_the_list_and_index_follows_it() {
    let Some(f) = open("filter") else { return };
    f.app.ui(|ui| ui.invoke_browse_filter_changed("gamma 01".into()));
    assert_eq!(f.rows(), 10);
    assert_eq!(f.state()["library_listed"], 10);
    let played = f.act("play", json!({"index": 1})).unwrap();
    assert_eq!(played["track"], "Gamma 010");
    f.app.ui(|ui| ui.invoke_browse_filter_changed("".into()));
    assert_eq!(f.rows(), 300);
}

#[test]
fn a_player_that_dies_is_said_in_plain_words_and_the_next_play_starts_another() {
    let Some(f) = open("dies") else { return };
    f.act("play", json!({"index": 1})).unwrap();
    let first = f.pid().unwrap();
    let crash = f.music.join("crash.mp3");
    std::fs::write(&crash, b"").unwrap();
    let _ = f.act("play_file", json!({"path": crash}));
    f.until("the death is noticed", |f| f.state()["player"] == "stopped");
    assert_eq!(f.notice(), "The player (mpv) stopped. Press play to start it again.");
    assert!(!f.playing());
    f.quiet();
    assert!(super::fake::reaped(first), "the player that died was reaped, not left a zombie");
    let again = f.act("play_file", json!({"path": f.track("Beta", 3)})).unwrap();
    assert_eq!(again["playing"], true, "a new mpv: {again}");
    assert_eq!(f.state()["player"], "running");
    assert!(f.notice().is_empty(), "the complaint about mpv is gone: {}", f.notice());
    assert_ne!(f.pid(), Some(first));
}

#[test]
fn nothing_redraws_at_rest_and_the_position_redraws_only_the_transport() {
    let Some(f) = open("at-rest") else { return };
    // No timer of the app's own: what moves on screen is what mpv pushes.
    assert_eq!(f.app.timers_armed(), 0);
    f.act("play", json!({"index": 1})).unwrap();
    f.act("pause", json!({})).unwrap();
    assert_eq!(f.app.timers_armed(), 0, "nothing ticks while paused");
    let before = f.app.revision();
    std::thread::sleep(std::time::Duration::from_millis(200));
    f.quiet();
    assert_eq!(f.app.revision(), before, "paused, nothing reached the window");
}

/// Paused part of the way into a second, `describe` says where it stopped, not the second before.
#[test]
fn a_pause_shows_where_the_track_stopped() {
    let Some(f) = open("pause-position") else { return };
    f.act("play", json!({"index": 1})).unwrap();
    f.act("seek", json!({"seconds": 12.0})).unwrap();
    f.act("seek", json!({"seconds": 12.9})).unwrap();
    f.quiet();
    assert_eq!(f.state()["position"], 12.0, "within the second shown, the window is not redrawn");
    f.act("pause", json!({})).unwrap();
    f.until("the position it stopped at is in", |f| f.state()["position"] == 12.9);
}

/// A fast drag is one change at a time, and mpv ends at the slider's last value.
#[test]
fn a_volume_drag_ends_where_the_slider_does() {
    let Some(f) = open("drag") else { return };
    f.act("play", json!({"index": 1})).unwrap();
    for step in 0..=20 {
        f.app.ui(move |ui| ui.invoke_volume_changed(step as f32 * 0.04));
    }
    assert_eq!(f.app.model(|m| m.player_jobs), 1, "one change on its way, the newest waiting");
    let last = (20.0f32 * 0.04) as f64 * 100.0;
    f.until("mpv is at the slider's last value", |f| f.state()["volume"].as_f64().is_some_and(|v| (v - last).abs() < 0.001));
    assert!(!f.app.model(|m| m.volume_sending));
    f.app.run_writes();
    assert!((f.app.committed_settings().volume - last).abs() < 0.001);
}

/// A save that did not reach the disk is said as what it was.
#[test]
fn a_failed_save_is_said_as_a_settings_save() {
    let Some(f) = open("not-saved") else { return };
    f.app.event(yantrik_app::app::Event::SettingsNotSaved("the disk is full".into()));
    assert!(f.notice().starts_with("could not save Music's settings to "), "{}", f.notice());
    assert!(f.notice().ends_with(": the disk is full"), "{}", f.notice());
}

/// A call's `set_volume` in the middle of the person's drag is the newest, so it is the one mpv
/// ends at, the one kept, and the one the call is told (#937 delta review, finding B).
#[test]
fn a_calls_volume_in_the_middle_of_a_drag_is_the_one_that_wins() {
    let Some(f) = open("drag-and-call") else { return };
    f.act("play", json!({"index": 1})).unwrap();
    f.app.ui(|ui| ui.invoke_volume_changed(0.2));
    f.app.ui(|ui| ui.invoke_volume_changed(0.7));
    assert!(f.app.model(|m| m.volume_next.is_some()), "the 0.7 waits for the 0.2 to land");
    let answer = f.agent("set_volume", json!({"percent": 30})).unwrap();
    assert_eq!(answer["volume"], 30.0, "the call is told what mpv applied: {answer}");
    f.quiet();
    assert_eq!(f.state()["volume"], 30.0, "mpv ends at the newest value, the call's");
    f.app.run_writes();
    assert_eq!(f.app.committed_settings().volume, 30.0);
    // The person's next move after the call is the newest again.
    f.app.ui(|ui| ui.invoke_volume_changed(0.5));
    f.until("mpv follows the person again", |f| f.state()["volume"] == 50.0);
}
