mod bins;
mod dirs;
mod download;
mod editor;
mod jobs;
mod library;
mod settings;
mod state;

pub use state::AppState;
use tauri::{Builder, Manager, Runtime};

/// Register every command. Generic over the runtime so the integration tests
/// can drive the real command layer with Tauri's mock runtime.
pub fn register_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        editor::app_info,
        editor::probe_media,
        editor::make_preview,
        editor::default_save_path,
        editor::export_video,
        editor::cancel_job,
        editor::reveal_in_finder,
        editor::open_url,
        download::download_link,
        download::download_dir,
        editor::export_dir,
        editor::estimate_export,
        library::library_add_text,
        library::library_replace_text,
        library::library_list,
        library::library_add,
        library::library_update,
        library::library_remove,
        settings::get_settings,
        settings::save_settings,
        settings::ytdlp_version,
        settings::update_ytdlp,
    ])
}

pub fn run() {
    register_commands(tauri::Builder::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&cache_dir)?;

            // The watermark library is shown straight from disk by the webview.
            let library_dir = data_dir.join("watermarks");
            std::fs::create_dir_all(library_dir.join("files"))?;
            let _ = app.asset_protocol_scope().allow_directory(&library_dir, true);

            let state = AppState::new(data_dir.clone(), cache_dir.clone());
            let auto_update_ytdlp = state.settings().auto_update_ytdlp;
            app.manage(state);

            editor::prune_preview_cache(&cache_dir);

            // Fetch the downloader on first launch, and (setting-controlled) refresh
            // it in the background every launch: Instagram / X / TikTok break it often.
            tauri::async_runtime::spawn(async move {
                if settings::ensure_ytdlp(&data_dir).await.is_ok() && auto_update_ytdlp {
                    let _ = settings::run_ytdlp_update(&data_dir).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running FillernCut");
}
