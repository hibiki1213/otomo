use tauri::Manager;

mod asset_cache;
mod chat;
mod control;
mod engine;
mod instruments;
mod library;
mod mcp_setup;
mod songs;
mod spatial;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
            asset_cache::asset_cache_has,
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
            library::library_list,
            library::library_install,
            library::library_remove,
            library::library_suggest,
            library::synth_presets,
            library::library_user_instruments,
            library::library_add_user_instrument,
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
            songs::songs_undo_apply,
            songs::songs_instruments,
            songs::songs_set_instrument,
            songs::songs_clear_instruments,
            mcp_setup::mcp_status,
            mcp_setup::mcp_connect,
            mcp_setup::mcp_disconnect,
            chat::chat_status,
            chat::chat_list,
            chat::chat_messages,
            chat::chat_running,
            chat::chat_send,
            chat::chat_stop,
            chat::chat_delete,
            chat::chat_set_model,
            chat::chat_set_effort,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
