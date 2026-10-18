#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod pptx;
mod summary;
mod textfile;

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
    warnings: Vec<String>,
}

/// TXT 옆(또는 지정 폴더)에 "요약완성_{TXT이름}.pptx"로 저장. 같은 이름이 있으면 " (2)", " (3)"...을 붙여 기존 파일을 덮어쓰지 않는다.
#[tauri::command]
fn build_summary_ppt(
    template_path: String,
    txt_path: String,
    output_dir: Option<String>,
) -> Result<SummaryPptResult, String> {
    let txt = textfile::read_text_file(&txt_path)?;
    let mut template = std::fs::read(&template_path).map_err(|e| format!("템플릿 PPTX를 읽을 수 없습니다: {e}"))?;
    pptx::sanitize_extended_timestamp_fields(&mut template);
    let (bytes, slide_count, warnings) = summary::build_summary_pptx(template, &txt)?;

    let txt = Path::new(&txt_path);
    // "경로 지정"으로 고른 폴더가 있으면 거기, 없으면 TXT 옆
    let dir = match &output_dir {
        Some(d) => Path::new(d),
        None => txt.parent().unwrap_or(Path::new(".")),
    };
    let stem = txt.file_stem().and_then(|s| s.to_str()).unwrap_or("요약");
    let mut output = dir.join(format!("요약완성_{stem}.pptx"));
    let mut n = 2;
    while output.exists() {
        output = dir.join(format!("요약완성_{stem} ({n}).pptx"));
        n += 1;
    }
    std::fs::write(&output, bytes).map_err(|e| format!("파일이 사용 중이거나 저장 권한이 없습니다: {e}"))?;
    Ok(SummaryPptResult { output_path: output.to_string_lossy().into_owned(), slide_count, warnings })
}

#[tauri::command]
fn get_ai_config() -> ai::AiConfig {
    ai::load_config()
}

#[tauri::command]
fn set_ai_config(config: ai::AiConfig) -> Result<(), String> {
    ai::save_config(&config)
}

/// GPT 호출은 수십 초~수 분 걸리므로 UI가 멈추지 않게 블로킹 스레드에서 실행.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn ai_generate_script(pptx_path: String, transcript_path: String) -> Result<String, String> {
    blocking(move || ai::generate_script(&pptx_path, &transcript_path)).await
}

#[tauri::command]
async fn ai_generate_summary(transcript_path: String, storyboard_path: Option<String>) -> Result<String, String> {
    blocking(move || ai::generate_summary(&transcript_path, storyboard_path.as_deref())).await
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
        .invoke_handler(tauri::generate_handler![
            import_notes,
            build_summary_ppt,
            get_ai_config,
            set_ai_config,
            ai_generate_script,
            ai_generate_summary
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
