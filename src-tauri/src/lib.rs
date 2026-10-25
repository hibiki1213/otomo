use tauri::Manager;

mod asset_cache;
mod chat;
mod control;
mod engine;
mod export;
mod i18n;
mod instruments;
mod library;
mod menu;
mod mcp_setup;
mod paste;
mod pdf;
mod songs;
mod spatial;
mod updates;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    i18n::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(library::LibraryState::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            app.manage(songs::SongLibrary::open(app.handle())?);
            app.manage(chat::ChatState::open(app.handle())?);
            control::start(app.handle());
            menu::install(app.handle())?;
            // Without an audio device the app still runs (web engine fallback).
            match engine::start() {
                Ok(handle) => {
                    app.manage(handle);
                }
                Err(e) => log::error!("native audio engine unavailable: {e}"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            asset_cache::asset_cache_get,
            asset_cache::asset_cache_put,
            engine::engine_load_soundfont,
            engine::engine_load_song,
            engine::engine_play,
            engine::engine_pause,
            engine::engine_stop,
            engine::engine_seek,
            engine::engine_set_master_volume,
            engine::engine_set_track_muted,
            engine::engine_set_track_gain,
            engine::engine_set_track_program,
            engine::engine_status,
            engine::engine_spatial_options,
            engine::engine_set_spatial,
            engine::engine_set_track_instrument,
            engine::plugin_list,
            engine::plugin_open_editor,
            export::score_tracks,
            export::score_musicxml,
            export::score_chords,
            export::export_write_text,
            i18n::system_language,
            i18n::set_language,
            updates::update_blocked,
            library::library_list,
            library::library_install,
            library::library_remove,
            library::library_suggest,
            library::synth_presets,
            library::library_user_instruments,
            library::library_add_user_instrument,
            paste::songs_paste,
            songs::songs_list,
            songs::songs_import,
            songs::songs_data,
            songs::songs_played,
            songs::songs_rename,
            songs::songs_set_favorite,
            songs::songs_remove,
            songs::songs_get,
            songs::songs_pending,
            songs::songs_pending_data,
            songs::songs_apply_pending,
            songs::songs_discard_pending,
            songs::songs_undo,
            songs::songs_redo,
            songs::songs_instruments,
            songs::songs_set_instrument,
            songs::songs_clear_instruments,
            mcp_setup::mcp_status,
            mcp_setup::mcp_connect,
            mcp_setup::mcp_disconnect,
            pdf::pdf_write,
            chat::chat_status,
            chat::chat_list,
            chat::chat_messages,
            chat::chat_running,
            chat::chat_send,
            chat::chat_stop,
            chat::chat_delete,
            chat::chat_set_model,
            chat::chat_set_effort,
            chat::chat_login,
            chat::chat_login_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
