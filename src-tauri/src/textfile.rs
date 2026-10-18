//! 녹취/대본/요약 파일 읽기: TXT(UTF-8), HWPX(zip+XML), HWP(5.0 바이너리) 모두 지원.
//! 한글 프로그램(COM) 없이 파일 구조를 직접 읽어 본문 글자만 뽑는다.

use std::io::{Cursor, Read};
use std::path::Path;

pub fn read_text_file(path: &str) -> Result<String, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let text = match ext.as_str() {
        "hwpx" => read_hwpx(path)?,
        "hwp" => read_hwp(path)?,
        _ => std::fs::read_to_string(path).map_err(|e| format!("TXT 파일을 읽을 수 없습니다(UTF-8 필요): {e}"))?,
    };
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

// ---- HWPX: Contents/section0.xml, section1.xml ... 의 <hp:t> 글자, <hp:p>마다 줄바꿈 ----

fn read_hwpx(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("HWPX 파일을 읽을 수 없습니다: {e}"))?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("HWPX(zip) 열기 실패: {e}"))?;
    let mut out = String::new();
    for i in 0.. {
        let name = format!("Contents/section{i}.xml");
        let Ok(mut f) = zip.by_name(&name) else { break };
        let mut xml = String::new();
        f.read_to_string(&mut xml).map_err(|e| format!("{name} 읽기 실패: {e}"))?;
        out.push_str(&hwpx_section_text(&xml));
    }
    if out.trim().is_empty() {
        return Err("HWPX에서 본문 글자를 찾지 못했습니다.".into());
    }
    Ok(out)
}

fn hwpx_section_text(xml: &str) -> String {
    let mut out = String::new();
    // 문단 끝(</hp:p>)마다 줄바꿈, 그 사이의 <hp:t>...</hp:t> 글자를 이어 붙임 (표 안 문단도 같은 방식)
    for para in xml.split("</hp:p>") {
        let mut line = String::new();
        let mut rest = para;
        while let Some(i) = rest.find("<hp:t") {
            rest = &rest[i..];
            let Some(gt) = rest.find('>') else { break };
            let tag = &rest[..gt];
            if tag.ends_with('/') || !matches!(rest.as_bytes().get(5), Some(b'>') | Some(b' ')) {
                rest = &rest[gt + 1..];
                continue;
            }
            let body = &rest[gt + 1..];
            let Some(end) = body.find("</hp:t>") else { break };
            // <hp:t> 안의 하위 태그: 줄바꿈(<hp:lineBreak/>)·탭(<hp:tab/>)만 글자로 바꾸고 나머지는 제거
            let mut inner = String::new();
            let mut tag = None::<String>;
            for ch in body[..end].chars() {
                match (&mut tag, ch) {
                    (None, '<') => tag = Some(String::new()),
                    (Some(t), '>') => {
                        if t.starts_with("hp:lineBreak") {
                            inner.push('\n');
                        } else if t.starts_with("hp:tab") {
                            inner.push('\t');
                        }
                        tag = None;
                    }
                    (Some(t), c) => t.push(c),
                    (None, c) => inner.push(c),
                }
            }
            line.push_str(&unescape(&inner));
            rest = &body[end..];
        }
        if para.contains("<hp:p") {
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// ---- HWP 5.0: OLE 복합 파일 → BodyText/SectionN(압축 시 raw deflate) → 레코드 중 PARA_TEXT(태그 67) ----

const HWPTAG_PARA_TEXT: u32 = 0x10 + 51;

fn read_hwp(path: &str) -> Result<String, String> {
    let mut comp = cfb::open(path).map_err(|e| format!("HWP 파일을 열 수 없습니다(HWP 5.0 이상만 지원): {e}"))?;

    let mut header = Vec::new();
    comp.open_stream("/FileHeader")
        .and_then(|mut s| s.read_to_end(&mut header))
        .map_err(|_| "HWP 파일 헤더가 없습니다.".to_string())?;
    if header.len() < 40 || !header.starts_with(b"HWP Document File") {
        return Err("HWP 5.0 형식이 아닙니다.".into());
    }
    let flags = u32::from_le_bytes([header[36], header[37], header[38], header[39]]);
    if flags & 0x2 != 0 {
        return Err("암호가 걸린 HWP는 읽을 수 없습니다.".into());
    }
    if flags & 0x4 != 0 {
        return Err("배포용 HWP 문서는 읽을 수 없습니다. 한글에서 TXT로 저장해 주세요.".into());
    }
    let compressed = flags & 0x1 != 0;

    let mut out = String::new();
    for i in 0.. {
        let name = format!("/BodyText/Section{i}");
        let Ok(mut stream) = comp.open_stream(&name) else { break };
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(|e| format!("{name} 읽기 실패: {e}"))?;
        let data = if compressed {
            let mut buf = Vec::new();
            flate2::read::DeflateDecoder::new(&raw[..])
                .read_to_end(&mut buf)
                .map_err(|e| format!("{name} 압축 해제 실패: {e}"))?;
            buf
        } else {
            raw
        };
        out.push_str(&hwp_section_text(&data));
    }
    if out.trim().is_empty() {
        return Err("HWP에서 본문 글자를 찾지 못했습니다.".into());
    }
    Ok(out)
}

fn hwp_section_text(data: &[u8]) -> String {
    let mut out = String::new();
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let h = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;
        let tag = h & 0x3FF;
        let mut size = (h >> 20) as usize;
        if size == 0xFFF {
            if pos + 4 > data.len() {
                break;
            }
            size = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
            pos += 4;
        }
        if pos + size > data.len() {
            break;
        }
        if tag == HWPTAG_PARA_TEXT {
            out.push_str(&para_text(&data[pos..pos + size]));
            out.push('\n');
        }
        pos += size;
    }
    out
}

/// PARA_TEXT 레코드(UTF-16LE). 0~31은 제어 문자: 일부는 1글자, 인라인/확장 컨트롤은 8글자(16바이트) 분량.
fn para_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let mut text = Vec::new();
    let mut i = 0;
    while i < units.len() {
        let c = units[i];
        if c >= 32 {
            text.push(c);
            i += 1;
            continue;
        }
        match c {
            9 => {
                text.push(b'\t' as u16);
                i += 8;
            }
            10 => {
                text.push(b'\n' as u16);
                i += 1;
            }
            24 => {
                text.push(b'-' as u16);
                i += 1;
            }
            30 | 31 => {
                text.push(b' ' as u16);
                i += 1;
            }
            0 | 13 | 25..=29 => i += 1,
            _ => i += 8, // 1~8, 11, 12, 14~23: 표·그림·필드 등 컨트롤 (8글자 분량 건너뜀)
        }
    }
    String::from_utf16_lossy(&text).trim_end().to_string()
}

#[cfg(test)]
mod tests {
    /// TEXT_FILE 환경변수 지정 후 `cargo test read_real_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn read_real_file() {
        let text = super::read_text_file(&std::env::var("TEXT_FILE").unwrap()).unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        println!("chars={} lines={}", text.chars().count(), lines.len());
        for l in lines.iter().take(15) {
            println!("| {l}");
        }
    }

    #[test]
    fn hwpx_section_paragraphs() {
        let xml = r#"<hs:sec><hp:p id="1"><hp:run><hp:t>안녕 &amp; </hp:t><hp:t>하세요</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t/></hp:run></hp:p><hp:p><hp:run><hp:t>=IF(A1&gt;0,"y","n")</hp:t></hp:run></hp:p></hs:sec>"#;
        assert_eq!(super::hwpx_section_text(xml), "안녕 & 하세요\n\n=IF(A1>0,\"y\",\"n\")\n");
    }
}
