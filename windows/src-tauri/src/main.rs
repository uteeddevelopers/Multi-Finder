// 릴리스 빌드는 콘솔 창 없이
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use std::sync::Arc;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let shared: state::Shared = Arc::new(state::AppState::new()?);
            app.manage(shared.clone());
            state::startup(app.handle(), &shared);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_launch_args,
            commands::add_folders,
            commands::remove_folder,
            commands::reorder_folders,
            commands::folder_states,
            commands::rescan,
            commands::is_scanning,
            commands::list_children,
            commands::dir_info,
            commands::folder_summaries,
            commands::stats,
            commands::search,
            commands::open_path,
            commands::reveal_paths,
            commands::read_text_preview,
            commands::move_items,
            commands::move_across,
            commands::trash_items,
            commands::trash_duplicates,
            commands::undo,
            commands::undo_state,
            commands::find_duplicates,
            commands::cancel_duplicates,
            commands::ignore_duplicate_group,
            commands::clear_duplicate_ignores,
            commands::compare_folders,
            commands::get_buckets,
            commands::save_buckets,
            commands::model_status,
            commands::download_model,
            commands::cancel_model_download,
            commands::classify,
            commands::cancel_classify,
            commands::list_classified,
            commands::set_bucket,
            commands::scan_leftovers,
            commands::trash_leftovers,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Multi-Finder");
}
