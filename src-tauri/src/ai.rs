//! "GPT 자동" 모드: ChatGPT 사이트에 갈 필요 없이 OpenAI API로 대본/요약 TXT를 바로 만든다.
//! 토큰 절약을 위해 PPTX는 파일째 보내지 않고 슬라이드 텍스트만 뽑아서 보낸다(이미지만 있는 슬라이드 내용은 GPT가 못 봄).
//! 설정(API 키)은 exe 옆 board1_config.json에 평문 저장 — 배포 계획 없는 개인 도구.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zip::ZipArchive;

/// 모델은 고정 (사용자 지정)
const MODEL: &str = "gpt-5.6-luna";
/// Responses API — 최신 모델(gpt-5.6-luna 등)은 이쪽 전용일 수 있고, 예전 모델도 다 지원함
const OPENAI_URL: &str = "https://api.openai.com/v1/responses";

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AiConfig {
    pub enabled: bool,
    pub api_key: String,
}

fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("board1_config.json")))
        .unwrap_or_else(|| PathBuf::from("board1_config.json"))
}

pub fn load_config() -> AiConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &AiConfig) -> Result<(), String> {
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), text).map_err(|e| format!("설정 저장 실패: {e}"))
}

// ---- 입력 준비 ----

/// 슬라이드 순서대로 각 슬라이드의 텍스트(문단마다 줄바꿈).
fn slide_texts(pptx_path: &str) -> Result<Vec<String>, String> {
    let mut bytes = std::fs::read(pptx_path).map_err(|e| format!("PPTX 파일을 읽을 수 없습니다: {e}"))?;
    crate::pptx::sanitize_extended_timestamp_fields(&mut bytes);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("PPTX(zip) 열기 실패: {e}"))?;
    let map = crate::pptx::build_slide_path_by_page(&mut archive)?;
    let mut out = Vec::new();
    for page in 1..=map.len() as u32 {
        let mut xml = String::new();
        if let Some(path) = map.get(&page) {
            if let Ok(mut f) = archive.by_name(path) {
                let _ = f.read_to_string(&mut xml);
            }
        }
        let lines: Vec<String> = xml
            .split("</a:p>")
            .map(|para| {
                let mut s = String::new();
                let mut rest = para;
                while let Some(i) = rest.find("<a:t") {
                    rest = &rest[i..];
                    let Some(gt) = rest.find('>') else { break };
                    if rest[..gt].ends_with('/') || !matches!(rest.as_bytes()[4], b'>' | b' ') {
                        rest = &rest[gt + 1..];
                        continue;
                    }
                    let body = &rest[gt + 1..];
                    let Some(end) = body.find("</a:t>") else { break };
                    s.push_str(&body[..end]);
                    rest = &body[end..];
                }
                unescape(&s).trim().to_string()
            })
            .filter(|l| !l.is_empty())
            .collect();
        out.push(lines.join("\n"));
    }
    Ok(out)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn slides_block(slides: &[String]) -> String {
    slides
        .iter()
        .enumerate()
        .map(|(i, t)| format!("[{}p]\n{}", i + 1, if t.is_empty() { "(텍스트 없음)" } else { t }))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 녹취는 TXT/HWP/HWPX 모두 가능
fn read_txt(path: &str) -> Result<String, String> {
    crate::textfile::read_text_file(path)
}

/// 입력 TXT 옆에 "{prefix}{TXT이름}.txt", 같은 이름이 있으면 " (2)"... 붙임.
fn write_beside(input: &str, prefix: &str, text: &str) -> Result<String, String> {
    let p = Path::new(input);
    let dir = p.parent().unwrap_or(Path::new("."));
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("결과");
    let mut out = dir.join(format!("{prefix}{stem}.txt"));
    let mut n = 2;
    while out.exists() {
        out = dir.join(format!("{prefix}{stem} ({n}).txt"));
        n += 1;
    }
    std::fs::write(&out, text.replace("\r\n", "\n").replace('\n', "\r\n"))
        .map_err(|e| format!("TXT 저장 실패: {e}"))?;
    Ok(out.to_string_lossy().into_owned())
}

// ---- OpenAI 호출 ----

fn chat(cfg: &AiConfig, messages: Value) -> Result<String, String> {
    if cfg.api_key.trim().is_empty() {
        return Err("OpenAI API 키가 없습니다. 우측 위 GPT 자동 설정에서 입력하세요.".into());
    }
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(900)).build();
    let body = json!({ "model": MODEL, "input": messages });
    match agent
        .post(OPENAI_URL)
        .set("Authorization", &format!("Bearer {}", cfg.api_key.trim()))
        .send_json(body)
    {
        Ok(resp) => {
            let v: Value = resp.into_json().map_err(|e| format!("OpenAI 응답 해석 실패: {e}"))?;
            if v["status"] == "incomplete" {
                return Err("GPT 출력이 길이 제한에 걸려 잘렸습니다. 녹취를 나눠서 다시 시도하세요.".into());
            }
            // output[]의 message 항목들 안의 output_text를 이어 붙임 (reasoning 항목은 건너뜀)
            let text: String = v["output"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["type"] == "message")
                .flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
                .filter(|c| c["type"] == "output_text")
                .filter_map(|c| c["text"].as_str().map(str::to_string))
                .collect();
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err("GPT가 빈 응답을 돌려줬습니다.".into());
            }
            Ok(text)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let msg = resp
                .into_json::<Value>()
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or_default();
            Err(format!("OpenAI 오류({code}): {msg}"))
        }
        Err(e) => Err(format!("OpenAI 연결 실패: {e}")),
    }
}

/// GPT가 습관적으로 감싸는 ``` 코드 블록 제거.
fn strip_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.split_once('\n').map_or("", |(_, r)| r);
        return rest.trim_end().trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}

const SCRIPT_SYSTEM: &str = "너는 강의 녹취록을 슬라이드별로 나누는 도우미다. 사용자가 슬라이드별 텍스트와 녹취 원문을 준다. 녹취 원문을 슬라이드 흐름에 맞게 페이지별로 분배해라.
규칙:
1. 출력 형식: 각 페이지마다 'Np' 한 줄(예: 1p)을 쓰고, 다음 줄부터 그 페이지에 해당하는 녹취 원문을 쓴다. 페이지 사이에는 빈 줄 하나.
2. 녹취 원문은 요약하거나 새로 쓰지 말고 원문 문장을 그대로 사용한다. 녹취의 문장을 순서대로 빠짐없이 어느 한 페이지에 배정한다.
3. 대응하는 설명이 없는 페이지는 억지로 만들지 말고 'Np' 줄만 쓰고 비워둔다.
4. 페이지 번호는 1p부터 마지막 슬라이드까지 전부 쓴다. 0p는 쓰지 않는다.
5. 텍스트가 없는 슬라이드(이미지 중심)는 앞뒤 슬라이드와 녹취 흐름으로 판단한다.
6. 머리말, 설명, 코드 블록 없이 결과 텍스트만 출력한다.";

/// 강의안 PPTX + 원본 녹취 TXT → "Np" 대본 TXT를 녹취 옆에 저장하고 그 경로를 돌려준다.
pub fn generate_script(pptx_path: &str, transcript_path: &str) -> Result<String, String> {
    let cfg = load_config();
    let slides = slide_texts(pptx_path)?;
    let transcript = read_txt(transcript_path)?;
    let user = format!(
        "[슬라이드 텍스트] (총 {}장)\n\n{}\n\n[녹취 원문]\n{}",
        slides.len(),
        slides_block(&slides),
        transcript
    );
    let out = chat(&cfg, json!([{ "role": "system", "content": SCRIPT_SYSTEM }, { "role": "user", "content": user }]))?;
    write_beside(transcript_path, "AI대본_", &strip_fence(&out))
}

const SUMMARY_SYSTEM: &str = "너는 강의 요약 TXT를 작성하는 도우미다. 사용자가 녹취록(그리고 있으면 스토리보드 슬라이드 텍스트)을 준다. 스토리보드와 녹취록을 대조해 강의 요약 TXT를 작성해라.
구성: 표지용 차시명, 학습목표 2개, 학습내용 5개, 본문 5페이지. 각 본문 페이지는 대주제 1개, 소주제 2개, 소주제별 본문 3~4개 항목. 대주제는 슬라이드 제목 한 줄에 들어가야 하므로 공백 포함 18자 이내로 짧게 쓴다. 항목은 가능하면 짧은 한 문장으로 정리하고, 설명이 긴 문장은 줄인다. 함수명과 수식은 정확하게 보존한다. 학습내용 5개는 본문 1~5의 대주제와 순서대로 대응한다.
출력 형식(이 형식 외의 문장은 절대 쓰지 말 것):
[차시명]
차시명 한 줄

[학습목표]
- 목표1
- 목표2

[학습내용]
- 내용1
(5개)

[본문 1]
대주제: ...
소주제1: ...
본문1:
- 항목
소주제2: ...
본문2:
- 항목

[본문 2]부터 [본문 5]까지 같은 형식.
목록 항목은 '- '로 시작하고, 소주제에 번호(①, 1. 등)를 붙이지 않는다. 머리말, 설명, 코드 블록, Markdown(#, **) 없이 결과만 출력한다.";

/// 원본 녹취 TXT (+ 스토리보드 PPTX) → 요약 TXT를 녹취 옆에 저장하고 그 경로를 돌려준다.
/// 형식 검사(summary::parse_summary)에 걸리면 오류 내용을 GPT에 돌려줘 한 번 고치게 한다.
pub fn generate_summary(transcript_path: &str, storyboard_path: Option<&str>) -> Result<String, String> {
    let cfg = load_config();
    let transcript = read_txt(transcript_path)?;
    let mut user = String::new();
    if let Some(sb) = storyboard_path {
        let slides = slide_texts(sb)?;
        user.push_str(&format!("[스토리보드 슬라이드 텍스트] (총 {}장)\n\n{}\n\n", slides.len(), slides_block(&slides)));
    }
    user.push_str(&format!("[녹취록]\n{transcript}"));

    let mut messages = vec![json!({ "role": "system", "content": SUMMARY_SYSTEM }), json!({ "role": "user", "content": user })];
    let mut out = strip_fence(&chat(&cfg, Value::Array(messages.clone()))?);
    if let Err(errors) = crate::summary::parse_summary(&out) {
        messages.push(json!({ "role": "assistant", "content": out }));
        messages.push(json!({
            "role": "user",
            "content": format!("형식 검사에서 아래 오류가 났다. 같은 내용으로 형식만 고쳐서 전체를 다시 출력해라.\n{errors}")
        }));
        out = strip_fence(&chat(&cfg, Value::Array(messages))?);
    }
    write_beside(transcript_path, "AI요약_", &out)
}

#[cfg(test)]
mod tests {
    /// SLIDES_PPTX 환경변수 지정 후 `cargo test slide_texts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn slide_texts_from_real_pptx() {
        let slides = super::slide_texts(&std::env::var("SLIDES_PPTX").unwrap()).unwrap();
        println!("{}", super::slides_block(&slides));
    }

    #[test]
    fn strips_code_fence() {
        assert_eq!(super::strip_fence("```text\n1p\n안녕\n```"), "1p\n안녕");
        assert_eq!(super::strip_fence("1p\n안녕"), "1p\n안녕");
    }
}

#[cfg(test)]
mod network_tests {
    /// 가짜 키로 실제 OpenAI에 요청해 오류 메시지가 제대로 올라오는지 확인 (`cargo test fake_key -- --ignored --nocapture`)
    #[test]
    #[ignore]
    fn fake_key_error_is_readable() {
        let cfg = super::AiConfig { enabled: true, api_key: "sk-invalid-test".into() };
        let err = super::chat(&cfg, serde_json::json!([{ "role": "user", "content": "hi" }])).unwrap_err();
        println!("{err}");
        assert!(err.starts_with("OpenAI 오류(401)"), "{err}");
    }
}
