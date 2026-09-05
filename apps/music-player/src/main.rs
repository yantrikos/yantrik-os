//! Yantrik Music Player — standalone app binary.
//!
//! Manages local music library, playlists, queue, and playback state.
//! Playback engine integration (mpv) is stubbed for now.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-music-player");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("music-player") else { return };

    let app = MusicPlayerApp::new().unwrap();

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    app.run().unwrap();
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &MusicPlayerApp) {
    // Set initial state
    app.set_volume(0.8);
    app.set_playback_speed(1.0);
    app.set_repeat_mode(0);
    app.set_shuffle_on(false);
    app.set_is_playing(false);
    app.set_library_track_count(0);
    app.set_queue_current_index(-1);
    app.set_active_playlist_index(-1);

    // Playback controls
    {
        let weak = app.as_weak();
        app.on_play_pause(move || {
            let Some(ui) = weak.upgrade() else { return };
            let playing = ui.get_is_playing();
            ui.set_is_playing(!playing);
            tracing::info!("Play/pause toggled: {}", !playing);
        });
    }

    {
        let weak = app.as_weak();
        app.on_stop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_is_playing(false);
            ui.set_progress(0.0);
            ui.set_time_current("0:00".into());
            tracing::info!("Playback stopped");
        });
    }

    app.on_next_track(|| { tracing::info!("Next track (stub)"); });
    app.on_prev_track(|| { tracing::info!("Previous track (stub)"); });

    {
        let weak = app.as_weak();
        app.on_seek(move |pos| {
            if let Some(ui) = weak.upgrade() {
                ui.set_progress(pos);
            }
        });
    }

    {
        let weak = app.as_weak();
        app.on_volume_changed(move |vol| {
            if let Some(ui) = weak.upgrade() {
                ui.set_volume(vol);
            }
        });
    }

    {
        let weak = app.as_weak();
        app.on_toggle_mute(move || {
            let Some(ui) = weak.upgrade() else { return };
            let muted = ui.get_volume_muted();
            ui.set_volume_muted(!muted);
        });
    }

    {
        let weak = app.as_weak();
        app.on_toggle_shuffle(move || {
            let Some(ui) = weak.upgrade() else { return };
            let on = ui.get_shuffle_on();
            ui.set_shuffle_on(!on);
        });
    }

    {
        let weak = app.as_weak();
        app.on_cycle_repeat(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mode = ui.get_repeat_mode();
            ui.set_repeat_mode((mode + 1) % 3);
        });
    }

    {
        let weak = app.as_weak();
        app.on_set_playback_speed(move |speed| {
            if let Some(ui) = weak.upgrade() {
                ui.set_playback_speed(speed);
            }
        });
    }

    app.on_play_track_index(|idx| { tracing::info!("Play track index {} (stub)", idx); });
    app.on_queue_play_index(|idx| { tracing::info!("Queue play index {} (stub)", idx); });

    // Library browse
    {
        let weak = app.as_weak();
        app.on_browse_category_changed(move |cat| {
            if let Some(ui) = weak.upgrade() {
                ui.set_browse_category(cat);
            }
        });
    }

    app.on_browse_filter_changed(|_filter| {});
    app.on_browse_item_selected(|_item| {});

    // Queue management
    app.on_add_to_queue(|_idx| { tracing::info!("Add to queue (stub)"); });
    app.on_play_next(|_idx| { tracing::info!("Play next (stub)"); });
    app.on_queue_remove(|_idx| { tracing::info!("Queue remove (stub)"); });
    app.on_queue_move_up(|_idx| {});
    app.on_queue_move_down(|_idx| {});
    app.on_queue_clear(|| { tracing::info!("Queue clear (stub)"); });

    // Playlist management
    app.on_playlist_create(|name| { tracing::info!("Create playlist: {} (stub)", name); });
    app.on_playlist_rename(|idx, name| { tracing::info!("Rename playlist {}: {} (stub)", idx, name); });
    app.on_playlist_delete(|idx| { tracing::info!("Delete playlist {} (stub)", idx); });

    {
        let weak = app.as_weak();
        app.on_playlist_select(move |idx| {
            if let Some(ui) = weak.upgrade() {
                ui.set_active_playlist_index(idx);
            }
        });
    }

    app.on_playlist_add_track(|_pl, _tr| {});
    app.on_playlist_remove_track(|_pl, _tr| {});

    // Scan folders
    app.on_scan_add_folder(|path| { tracing::info!("Add scan folder: {} (stub)", path); });
    app.on_scan_remove_folder(|_idx| {});
    app.on_scan_rescan(|| { tracing::info!("Rescan library (stub)"); });

    // Folder watch + equalizer
    app.on_music_toggle_folder_watch(|| { tracing::info!("Toggle folder watch (stub)"); });

    {
        let weak = app.as_weak();
        app.on_music_set_equalizer(move |preset| {
            if let Some(ui) = weak.upgrade() {
                ui.set_music_equalizer_preset(preset);
            }
        });
    }

    // AI stubs
    app.on_ai_explain_pressed(|| { tracing::info!("AI explain requested (standalone mode)"); });
    app.on_ai_dismiss(|| {});

    // A design fixture so the library, queue and transport can be reviewed without a music
    // collection or a playback backend. Runs last: it overrides the initial state above.
    if std::env::var_os("YANTRIK_MUSIC_DEMO").is_some() {
        demo::populate(app);
    }
}

/// Design fixture: `YANTRIK_MUSIC_DEMO=1` fills the library, queue and now-playing state with
/// plausible data so the screen can be judged (and screenshotted) before mpv is wired up.
mod demo {
    use super::*;

    pub fn populate(app: &MusicPlayerApp) {
        let rows: &[(&str, &str, &str, &str, &str, bool)] = &[
            ("Nightfall Over Kolkata", "Arun Sen Quartet", "Monsoon Sessions", "Jazz", "6:12", false),
            ("Tuning Fork", "Hale & Wren", "Small Machines", "Electronic", "4:03", true),
            ("Barrel of the Morning", "Ilse Mordaunt", "Ferrous", "Folk", "3:28", false),
            ("Signal Path", "Kaveri Rao", "Low Orbit", "Electronic", "5:47", false),
            ("Eleven Bridges", "The Undertow", "Eleven Bridges", "Rock", "4:55", false),
            ("Slow Aperture", "Hale & Wren", "Small Machines", "Electronic", "7:19", false),
            ("Dust and Copper", "Ilse Mordaunt", "Ferrous", "Folk", "2:51", false),
            ("Paper Lanterns", "Arun Sen Quartet", "Monsoon Sessions", "Jazz", "5:33", false),
            ("Cold Start", "Kaveri Rao", "Low Orbit", "Electronic", "3:12", false),
            ("The Long Way Round", "The Undertow", "Eleven Bridges", "Rock", "6:41", false),
            ("Harbour Lights", "Marta Vinke", "Northern Line", "Ambient", "8:04", false),
            ("Second Shift", "Marta Vinke", "Northern Line", "Ambient", "5:16", false),
        ];
        let tracks: Vec<MusicTrackData> = rows
            .iter()
            .map(|(title, artist, album, genre, dur, current)| MusicTrackData {
                title: (*title).into(),
                artist: (*artist).into(),
                album: (*album).into(),
                genre: (*genre).into(),
                duration_text: (*dur).into(),
                duration_secs: 0.0,
                path: format!("/home/yantrik/Music/{artist}/{album}/{title}.flac").into(),
                is_current: *current,
                format_info: "FLAC 44.1kHz".into(),
                bitrate: "1008 kbps".into(),
            })
            .collect();

        let artists = [
            ("Arun Sen Quartet", 3, 24),
            ("Hale & Wren", 2, 19),
            ("Ilse Mordaunt", 4, 31),
            ("Kaveri Rao", 2, 16),
            ("Marta Vinke", 5, 42),
            ("The Undertow", 1, 11),
        ];
        let albums = [
            ("Monsoon Sessions", "Arun Sen Quartet", 9, "2024"),
            ("Small Machines", "Hale & Wren", 11, "2025"),
            ("Ferrous", "Ilse Mordaunt", 8, "2023"),
            ("Low Orbit", "Kaveri Rao", 10, "2026"),
            ("Northern Line", "Marta Vinke", 12, "2022"),
            ("Eleven Bridges", "The Undertow", 11, "2021"),
        ];
        let genres = [("Electronic", 37), ("Folk", 28), ("Jazz", 24), ("Ambient", 42), ("Rock", 11)];
        let playlists = [
            ("Focus", 24, "1h 48m", false),
            ("Late shift", 61, "4h 12m", false),
            ("Bass check", 9, "38m", false),
        ];

        app.set_library_tracks(ModelRc::new(VecModel::from(tracks.clone())));
        app.set_library_track_count(143);
        app.set_library_artists(ModelRc::new(VecModel::from(
            artists
                .iter()
                .map(|(name, albums, tracks)| MusicArtistData {
                    name: (*name).into(),
                    album_count: *albums,
                    track_count: *tracks,
                })
                .collect::<Vec<_>>(),
        )));
        app.set_library_albums(ModelRc::new(VecModel::from(
            albums
                .iter()
                .map(|(name, artist, count, year)| MusicAlbumData {
                    name: (*name).into(),
                    artist: (*artist).into(),
                    track_count: *count,
                    year: (*year).into(),
                })
                .collect::<Vec<_>>(),
        )));
        app.set_library_genres(ModelRc::new(VecModel::from(
            genres
                .iter()
                .map(|(name, count)| MusicGenreData { name: (*name).into(), track_count: *count })
                .collect::<Vec<_>>(),
        )));
        app.set_playlists(ModelRc::new(VecModel::from(
            playlists
                .iter()
                .map(|(name, count, dur, active)| MusicPlaylistData {
                    name: (*name).into(),
                    track_count: *count,
                    total_duration: (*dur).into(),
                    is_active: *active,
                })
                .collect::<Vec<_>>(),
        )));
        app.set_scan_folders(ModelRc::new(VecModel::from(vec![
            MusicScanFolderData { path: "/home/yantrik/Music".into(), track_count: 128, is_scanning: false },
            MusicScanFolderData { path: "/mnt/archive/flac".into(), track_count: 15, is_scanning: false },
        ])));

        // Queue: everything after the current track.
        app.set_queue_tracks(ModelRc::new(VecModel::from(tracks[2..8].to_vec())));
        app.set_queue_current_index(0);

        let now = &tracks[1];
        app.set_now_title(now.title.clone());
        app.set_now_artist(now.artist.clone());
        app.set_now_album(now.album.clone());
        app.set_now_genre(now.genre.clone());
        app.set_now_format_info(now.format_info.clone());
        app.set_now_bitrate(now.bitrate.clone());
        app.set_is_playing(true);
        app.set_progress(0.38);
        app.set_time_current("1:32".into());
        app.set_time_total("4:03".into());
        app.set_browse_sub_header(SharedString::default());
        app.set_music_folder_watch_active(true);
        app.set_music_folder_watch_status("Watching 2 folders".into());
    }
}
