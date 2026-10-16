#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod pptx;
mod summary;

use pptx::ImportSummary;
use serde::Serialize;
use std::path::Path;

#[tauri::command]
fn import_notes(
    pptx_path: String,
    output_path: String,
    txt_path: String,
    keep_original: bool,
) -> Result<ImportSummary, String> {
    pptx::import_notes_from_script(&pptx_path, &output_path, &txt_path, keep_original)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryPptResult {
    output_path: String,
    slide_count: usize,
}

/// TXT 옆에 "요약완성_{TXT이름}.pptx"로 저장. 같은 이름이 있으면 " (2)", " (3)"...을 붙여 기존 파일을 덮어쓰지 않는다.
#[tauri::command]
fn build_summary_ppt(template_path: String, txt_path: String) -> Result<SummaryPptResult, String> {
    let txt = std::fs::read_to_string(&txt_path).map_err(|e| format!("TXT 파일을 읽을 수 없습니다(UTF-8 필요): {e}"))?;
    let mut template = std::fs::read(&template_path).map_err(|e| format!("템플릿 PPTX를 읽을 수 없습니다: {e}"))?;
    pptx::sanitize_extended_timestamp_fields(&mut template);
    let (bytes, slide_count) = summary::build_summary_pptx(template, &txt)?;

    let txt = Path::new(&txt_path);
    let dir = txt.parent().unwrap_or(Path::new("."));
    let stem = txt.file_stem().and_then(|s| s.to_str()).unwrap_or("요약");
    let mut output = dir.join(format!("요약완성_{stem}.pptx"));
    let mut n = 2;
    while output.exists() {
        output = dir.join(format!("요약완성_{stem} ({n}).pptx"));
        n += 1;
    }
    std::fs::write(&output, bytes).map_err(|e| format!("파일이 사용 중이거나 저장 권한이 없습니다: {e}"))?;
    Ok(SummaryPptResult { output_path: output.to_string_lossy().into_owned(), slide_count })
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
        .invoke_handler(tauri::generate_handler![import_notes, build_summary_ppt])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
