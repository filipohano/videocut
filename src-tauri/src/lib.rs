mod bins;
mod dirs;
mod download;
mod editor;
mod history;
mod jobs;
mod library;
mod settings;
mod state;

pub use state::AppState;
#[cfg(not(windows))]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Builder, Emitter, Manager, Runtime};

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
        download::discard_download,
        editor::export_dir,
        editor::estimate_export,
        history::history_list,
        history::history_remove,
        history::history_clear,
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

/// On Windows there is no menu bar: the shortcuts (Ctrl+Z, Ctrl+N, …) are handled by the page itself.
#[cfg(not(windows))]
/// A menu with our own Undo / Redo. The default Edit menu would swallow ⌘Z for the
/// webview's text-field undo; ours is forwarded to the app (`menu-undo` / `menu-redo`).
fn build_menu<R: Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let new = MenuItem::with_id(handle, "new", "New Video or Photo", true, Some("CmdOrCtrl+N"))?;
    // A menu item holds one shortcut, so ⌘R is a second item that does the same.
    let new_alt = MenuItem::with_id(handle, "new-alt", "Start Over", true, Some("CmdOrCtrl+R"))?;
    let open = MenuItem::with_id(handle, "open", "Open…", true, Some("CmdOrCtrl+O"))?;
    let undo = MenuItem::with_id(handle, "undo", "Undo", true, Some("CmdOrCtrl+Z"))?;
    let redo = MenuItem::with_id(handle, "redo", "Redo", true, Some("CmdOrCtrl+Shift+Z"))?;
    let sep = || PredefinedMenuItem::separator(handle);
    let app_menu = Submenu::with_items(
        handle,
        "FillernCut",
        true,
        &[
            &PredefinedMenuItem::about(handle, None, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(handle, None)?,
            &PredefinedMenuItem::hide_others(handle, None)?,
            &sep()?,
            &PredefinedMenuItem::quit(handle, None)?,
        ],
    )?;
    let file = Submenu::with_items(
        handle,
        "File",
        true,
        &[
            &new,
            &new_alt,
            &open,
            &sep()?,
            &PredefinedMenuItem::close_window(handle, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        handle,
        "Edit",
        true,
        &[
            &undo,
            &redo,
            &sep()?,
            &PredefinedMenuItem::cut(handle, None)?,
            &PredefinedMenuItem::copy(handle, None)?,
            &PredefinedMenuItem::paste(handle, None)?,
            &PredefinedMenuItem::select_all(handle, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        handle,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(handle, None)?,
            &PredefinedMenuItem::maximize(handle, None)?,
            &PredefinedMenuItem::fullscreen(handle, None)?,
        ],
    )?;
    Menu::with_items(handle, &[&app_menu, &file, &edit, &window])
}

pub fn run() {
    let builder = register_commands(tauri::Builder::default());
    #[cfg(not(windows))]
    let builder = builder.menu(build_menu);
    builder
        .on_menu_event(|app, event| match event.id().as_ref() {
            "new" | "new-alt" => {
                let _ = app.emit("menu-new", ());
            }
            "open" => {
                let _ = app.emit("menu-open", ());
            }
            "undo" => {
                let _ = app.emit("menu-undo", ());
            }
            "redo" => {
                let _ = app.emit("menu-redo", ());
            }
            _ => {}
        })
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
            // ...and so are the history preview images.
            let thumbs = data_dir.join("history").join("thumbs");
            std::fs::create_dir_all(&thumbs)?;
            let _ = app.asset_protocol_scope().allow_directory(&thumbs, true);

            let state = AppState::new(data_dir.clone(), cache_dir.clone());
            let auto_update_ytdlp = state.settings().auto_update_ytdlp;
            app.manage(state);

            editor::prune_preview_cache(&cache_dir);
            dirs::clear_downloads(&cache_dir);

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
