//! Yantrik Music Player — standalone app binary.
//!
//! Manages local music library, playlists, queue, and playback state.
//! Playback engine integration (mpv) is stubbed for now.

use slint::ComponentHandle;
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-music-player");

    let app = MusicPlayerApp::new().unwrap();
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
}
