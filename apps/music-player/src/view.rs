//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that touches Slint.
//!
//! The lists are rebuilt only when they change (a scan, the filter, the track that plays); the
//! position redraws the transport alone, once a second while something plays and never at rest.

use slint::{ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;

use crate::command::Command;
use crate::library::{self, Track};
use crate::model::{Inbox, Model, PART_LISTS, PART_STATUS, PART_TRANSPORT};
use crate::{AgentContextItem, Music, MusicPlayerApp, MusicScanFolderData, MusicTrackData};

pub fn wire(cx: &Cx<Music>) {
    let ui = cx.ui();
    // Every handler below does what its control says, or puts the reason in `notice`.
    ui.on_play_pause(cx.cmd(|| Command::PlayPause));
    ui.on_stop(cx.cmd(|| Command::Stop));
    ui.on_next_track(cx.cmd(|| Command::Next));
    ui.on_prev_track(cx.cmd(|| Command::Previous));
    ui.on_seek(cx.cmd1(Command::SeekTo));
    ui.on_volume_changed(cx.cmd1(Command::VolumeTo));
    ui.on_toggle_mute(cx.cmd(|| Command::ToggleMute));
    ui.on_play_track_index(cx.cmd1(Command::PlayRow));
    ui.on_queue_play_index(cx.cmd1(Command::QueueRow));
    ui.on_browse_filter_changed(cx.cmd1(|text: SharedString| Command::Filter(text.to_string())));
    ui.on_scan_add_folder(cx.cmd1(|path: SharedString| Command::AddFolder(path.to_string())));
    ui.on_scan_remove_folder(cx.cmd1(Command::RemoveFolder));
    ui.on_scan_rescan(cx.cmd(|| Command::Rescan));
    ui.on_dismiss_notice(cx.cmd(|| Command::DismissNotice));
    drop(ui);
    // The player's reader thread and the scan's progress post through this.
    cx.run(Command::Attach(Inbox(cx.post())));
}

pub fn render(ui: &MusicPlayerApp, m: &Model, changed: &Changed) {
    if changed.has(PART_LISTS) {
        lists(ui, m);
    }
    if changed.has(PART_LISTS) || changed.has(PART_STATUS) {
        ui.set_is_scanning(m.scans.running > 0);
        ui.set_status_text(m.status.as_str().into());
        ui.set_notice(m.notice.as_str().into());
    }
    if changed.has(PART_TRANSPORT) {
        transport(ui, m);
    }
}

fn row(t: &Track, current: bool) -> MusicTrackData {
    MusicTrackData {
        title: t.title.as_str().into(),
        album: t.album.as_str().into(),
        path: t.path.as_str().into(),
        is_current: current,
    }
}

fn rows<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

/// The library list, the queue and the folders.
fn lists(ui: &MusicPlayerApp, m: &Model) {
    let playing = m.queue.current().map(|t| t.path.as_str());
    let library: Vec<MusicTrackData> = m
        .shown
        .iter()
        .filter_map(|&i| m.library.get(i))
        .map(|t| row(t, playing == Some(t.path.as_str())))
        .collect();
    ui.set_library_tracks(rows(library));
    ui.set_library_track_count(m.library.len() as i32);
    let current = m.queue.current_index();
    let queue = m.queue.tracks().iter().enumerate().map(|(i, t)| row(t, Some(i) == current)).collect();
    ui.set_queue_tracks(rows(queue));
    ui.set_queue_current_index(current.map(|i| i as i32).unwrap_or(-1));
    let folders = m
        .folders
        .iter()
        .enumerate()
        .map(|(i, f)| MusicScanFolderData {
            path: f.as_str().into(),
            track_count: m.per_folder.get(i).copied().unwrap_or(0) as i32,
        })
        .collect();
    ui.set_scan_folders(rows(folders));
}

/// The transport, from what mpv last reported.
fn transport(ui: &MusicPlayerApp, m: &Model) {
    let o = m.observed();
    let loaded = o.path.is_some() && !o.idle && !o.gone;
    let (title, album) = match (loaded, m.queue.current()) {
        (true, Some(t)) if o.path.as_deref() == Some(t.path.as_str()) => (t.title.clone(), t.album.clone()),
        (true, _) => (library::title_of(o.path.as_deref().unwrap_or_default()), String::new()),
        (false, _) => (String::new(), String::new()),
    };
    ui.set_now_title(title.as_str().into());
    ui.set_now_album(album.into());
    ui.set_is_playing(o.playing());
    let (pos, dur) = if loaded { (o.time_pos, o.duration) } else { (None, None) };
    ui.set_time_current(library::clock(pos).into());
    ui.set_time_total(library::clock(dur).into());
    ui.set_progress(match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => (p / d) as f32,
        _ => 0.0,
    });
    ui.set_volume((m.volume_now() / 100.0) as f32);
    ui.set_volume_muted(m.muted_now());
    // What this app holds, for the agent rail: the track mpv is playing.
    let context = if loaded {
        vec![AgentContextItem {
            id: "track".into(),
            label: title.into(),
            detail: match (pos, dur) {
                (Some(p), Some(d)) => format!("{} of {}", library::clock(Some(p)), library::clock(Some(d))),
                _ => String::new(),
            }
            .into(),
            source: "file".into(),
        }]
    } else {
        Vec::new()
    };
    ui.set_agent_context(rows(context));
}
