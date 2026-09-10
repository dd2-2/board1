#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod pptx;

use pptx::ImportSummary;

#[tauri::command]
fn import_notes(pptx_path: String, txt_path: String) -> Result<ImportSummary, String> {
    pptx::import_notes_from_script(&pptx_path, &txt_path)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![import_notes])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
