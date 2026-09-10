use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[derive(Serialize)]
pub struct PageResult {
    pub page: u32,
    pub message: String,
}

#[derive(Serialize)]
pub struct ImportSummary {
    pub applied: Vec<PageResult>,
    pub failed: Vec<PageResult>,
}

struct NoteBlock {
    page: u32,
    text: String,
}

/// 일부 PPTX는 중앙 디렉터리의 "확장 타임스탬프"(UT, 0x5455) extra field가
/// flags 바이트와 실제 데이터 길이가 서로 안 맞는 상태로 저장되어 있는 경우가 있음
/// (예: PowerPoint가 슬라이드 마스터만 다시 저장하면서 로컬 헤더는 3개 타임스탬프로 쓰고
/// 중앙 디렉터리 사본은 flags를 갱신하지 않는 등). 이 필드는 zip 내부 메타데이터일 뿐
/// PPTX 내용과 무관한데도, Rust `zip` crate(2.x/8.x 전부 확인함)는 이 불일치를 엄격하게
/// 거부해 EOCD를 못 찾는 것처럼 보이는 "invalid Zip archive: Could not find EOCD" 에러를
/// 낸다(zip crate 자체 이슈, 실제로는 EOCD도 CDFH도 다 정상 위치에 있음). 파싱 전에
/// flags 바이트를 데이터 길이와 일치하도록 미리 고쳐서 우회한다. 원본 파일(.bak)은
/// 이 함수 호출 전에 이미 저장되므로 디스크상의 원본은 그대로 보존됨.
fn sanitize_extended_timestamp_fields(bytes: &mut [u8]) {
    let len = bytes.len();
    if len < 22 {
        return;
    }

    let search_floor = len.saturating_sub(22 + 65536);
    let mut eocd = None;
    let mut i = len - 22;
    loop {
        if bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
            let comment_len = u16::from_le_bytes([bytes[i + 20], bytes[i + 21]]) as usize;
            if i + 22 + comment_len == len {
                eocd = Some(i);
                break;
            }
        }
        if i == search_floor {
            break;
        }
        i -= 1;
    }
    let Some(eocd) = eocd else { return };

    let entry_count = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]) as usize;
    let cd_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    let cd_offset = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    if cd_offset >= len || cd_offset + cd_size > len {
        return;
    }

    let mut pos = cd_offset;
    for _ in 0..entry_count {
        if pos + 46 > len || bytes[pos..pos + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return;
        }
        let name_len = u16::from_le_bytes([bytes[pos + 28], bytes[pos + 29]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[pos + 30], bytes[pos + 31]]) as usize;
        let comment_len = u16::from_le_bytes([bytes[pos + 32], bytes[pos + 33]]) as usize;
        let extra_start = pos + 46 + name_len;
        let extra_end = extra_start + extra_len;
        if extra_end > len {
            return;
        }

        let mut off = extra_start;
        while off + 4 <= extra_end {
            let tag = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
            let size = u16::from_le_bytes([bytes[off + 2], bytes[off + 3]]) as usize;
            let data_start = off + 4;
            let data_end = data_start + size;
            if data_end > extra_end {
                break;
            }
            if tag == 0x5455 && size >= 1 {
                let flags = bytes[data_start];
                let consistent = size == 5 || size as u32 == 1 + 4 * flags.count_ones();
                if !consistent && (size - 1) % 4 == 0 {
                    let chunks = (size - 1) / 4;
                    if chunks <= 3 {
                        bytes[data_start] = ((1u16 << chunks) - 1) as u8;
                    }
                }
            }
            off = data_end;
        }

        pos = extra_end + comment_len;
    }
}

pub fn import_notes_from_script(pptx_path: &str, txt_path: &str) -> Result<ImportSummary, String> {
    let script_text =
        fs::read_to_string(txt_path).map_err(|e| format!("대본 파일을 읽을 수 없습니다: {e}"))?;
    let blocks = parse_script(&script_text);
    if blocks.is_empty() {
        return Err("대본에서 'Np' 형식의 페이지 표시를 찾지 못했습니다.".to_string());
    }

    let mut original_bytes =
        fs::read(pptx_path).map_err(|e| format!("PPTX 파일을 읽을 수 없습니다: {e}"))?;

    let bak_path = format!("{pptx_path}.bak");
    if !Path::new(&bak_path).exists() {
        fs::write(&bak_path, &original_bytes).map_err(|e| format!("백업 생성 실패: {e}"))?;
    }

    sanitize_extended_timestamp_fields(&mut original_bytes);

    let mut archive = ZipArchive::new(Cursor::new(original_bytes))
        .map_err(|e| format!("PPTX(zip) 열기 실패: {e}"))?;

    let all_names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
    let slide_path_by_page = build_slide_path_by_page(&mut archive)?;
    let slide_count = slide_path_by_page.len() as u32;

    let mut overrides: HashMap<String, Vec<u8>> = HashMap::new();
    let mut applied = Vec::new();
    let mut failed = Vec::new();

    if !all_names.iter().any(|n| n == "ppt/notesMasters/notesMaster1.xml") {
        inject_notes_master(&mut archive, &all_names, &mut overrides)?;
    }

    for block in &blocks {
        if !slide_path_by_page.contains_key(&block.page) {
            failed.push(PageResult {
                page: block.page,
                message: format!("슬라이드 {}번이 존재하지 않음 (총 {}장)", block.page, slide_count),
            });
            continue;
        }
        match apply_note(
            &mut archive,
            &all_names,
            &slide_path_by_page,
            &mut overrides,
            block.page,
            &block.text,
        ) {
            Ok(()) => applied.push(PageResult {
                page: block.page,
                message: format!("슬라이드 {}번에 입력 완료", block.page),
            }),
            Err(e) => failed.push(PageResult { page: block.page, message: e }),
        }
    }

    if applied.is_empty() {
        return Err("적용된 슬라이드가 없습니다.".to_string());
    }

    write_pptx(&mut archive, &all_names, &overrides, pptx_path)?;

    Ok(ImportSummary { applied, failed })
}

/// 이 PPT에 메모(노트) 인프라(notesMaster 등)가 아예 없는 경우, PowerPoint를 통해
/// 슬라이드 1의 노트에 공백 한 칸을 입력하고 저장시켜 인프라를 정식으로 생성시킨다.
/// (직접 손으로 notesMaster XML을 만드는 것보다 PowerPoint가 생성한 걸 쓰는 게 안전함)
/// PowerPoint로 실제 생성한 notesMaster1.xml과 그 전용 테마를 그대로 캡처해 내장해둔 것.
/// 손으로 XML을 새로 작성하는 것보다 PowerPoint 자신이 만든 바이트를 그대로 쓰는 게 안전함.
const NOTES_MASTER_XML: &[u8] = include_bytes!("../assets/notes_template/notesMaster1.xml");
const NOTES_MASTER_THEME_XML: &[u8] =
    include_bytes!("../assets/notes_template/notesMasterTheme.xml");

/// 이 PPT에 메모(노트) 인프라(notesMaster 등)가 아예 없는 경우, 내장해둔 템플릿 파트들을
/// zip에 직접 추가해 인프라를 만든다. PowerPoint 실행 없이 즉시 끝남.
fn inject_notes_master(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    all_names: &[String],
    overrides: &mut HashMap<String, Vec<u8>>,
) -> Result<(), String> {
    let mut theme_idx = 1u32;
    loop {
        let candidate = format!("ppt/theme/theme{theme_idx}.xml");
        let exists =
            all_names.iter().any(|n| n == &candidate) || overrides.contains_key(&candidate);
        if !exists {
            break;
        }
        theme_idx += 1;
    }
    let theme_path = format!("ppt/theme/theme{theme_idx}.xml");
    overrides.insert(theme_path, NOTES_MASTER_THEME_XML.to_vec());

    let notes_master_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme{theme_idx}.xml"/></Relationships>"#
    );
    overrides.insert(
        "ppt/notesMasters/_rels/notesMaster1.xml.rels".to_string(),
        notes_master_rels.into_bytes(),
    );
    overrides.insert(
        "ppt/notesMasters/notesMaster1.xml".to_string(),
        NOTES_MASTER_XML.to_vec(),
    );

    let pres_rels_bytes = overrides
        .get("ppt/_rels/presentation.xml.rels")
        .cloned()
        .or_else(|| read_entry(archive, "ppt/_rels/presentation.xml.rels"))
        .ok_or("presentation.xml.rels 읽기 실패")?;
    let mut pres_rel_map = get_rels_map(&pres_rels_bytes);
    let new_rid = next_unused_rid(&pres_rel_map);
    pres_rel_map.insert(
        new_rid.clone(),
        (
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster"
                .to_string(),
            "notesMasters/notesMaster1.xml".to_string(),
        ),
    );
    overrides.insert(
        "ppt/_rels/presentation.xml.rels".to_string(),
        build_rels_xml(&pres_rel_map).into_bytes(),
    );

    let pres_xml_bytes = overrides
        .get("ppt/presentation.xml")
        .cloned()
        .or_else(|| read_entry(archive, "ppt/presentation.xml"))
        .ok_or("presentation.xml 읽기 실패")?;
    let pres_xml = String::from_utf8_lossy(&pres_xml_bytes).into_owned();
    if !pres_xml.contains("</p:sldMasterIdLst>") {
        return Err("presentation.xml 구조를 인식할 수 없습니다".to_string());
    }
    let insertion = format!(
        "</p:sldMasterIdLst><p:notesMasterIdLst><p:notesMasterId r:id=\"{new_rid}\"/></p:notesMasterIdLst>"
    );
    let updated_pres_xml = pres_xml.replacen("</p:sldMasterIdLst>", &insertion, 1);
    overrides.insert("ppt/presentation.xml".to_string(), updated_pres_xml.into_bytes());

    let ct_bytes = overrides
        .get("[Content_Types].xml")
        .cloned()
        .or_else(|| read_entry(archive, "[Content_Types].xml"))
        .ok_or("[Content_Types].xml 읽기 실패")?;
    let ct_str = String::from_utf8_lossy(&ct_bytes);
    let ct_insertion = format!(
        r#"<Override PartName="/ppt/notesMasters/notesMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml"/><Override PartName="/ppt/theme/theme{theme_idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#
    );
    let updated_ct = ct_str.replacen("</Types>", &ct_insertion, 1);
    overrides.insert("[Content_Types].xml".to_string(), updated_ct.into_bytes());

    Ok(())
}

fn parse_script(text: &str) -> Vec<NoteBlock> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = Vec::new();
    let mut current_page: Option<u32> = None;
    let mut current_text = String::new();

    for line in normalized.split('\n') {
        if let Some(p) = page_marker(line) {
            if let Some(page) = current_page {
                blocks.push(NoteBlock { page, text: current_text.trim().to_string() });
            }
            current_page = Some(p);
            current_text.clear();
        } else if current_page.is_some() {
            current_text.push_str(line);
            current_text.push('\n');
        }
    }
    if let Some(page) = current_page {
        blocks.push(NoteBlock { page, text: current_text.trim().to_string() });
    }
    blocks
}

fn page_marker(line: &str) -> Option<u32> {
    let t = line.trim();
    if t.len() < 2 {
        return None;
    }
    let last = t.chars().last().unwrap();
    if last != 'p' && last != 'P' {
        return None;
    }
    let num_part = &t[..t.len() - last.len_utf8()];
    if num_part.is_empty() || !num_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    num_part.parse::<u32>().ok()
}

fn read_entry(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Option<Vec<u8>> {
    let mut file = archive.by_name(name).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn get_attr(e: &BytesStart, key: &[u8]) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == key {
            return Some(String::from_utf8_lossy(&a.value).to_string());
        }
    }
    None
}

fn get_slide_rids_in_order(xml: &[u8]) -> Vec<String> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut in_list = false;
    let mut rids = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if e.name().as_ref() == b"p:sldIdLst" {
                    in_list = true;
                }
                if in_list && e.name().as_ref() == b"p:sldId" {
                    if let Some(rid) = get_attr(&e, b"r:id") {
                        rids.push(rid);
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                if in_list && e.name().as_ref() == b"p:sldId" {
                    if let Some(rid) = get_attr(&e, b"r:id") {
                        rids.push(rid);
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"p:sldIdLst" {
                    in_list = false;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    rids
}

/// id -> (type, target)
fn get_rels_map(xml: &[u8]) -> HashMap<String, (String, String)> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut map = HashMap::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"Relationship" {
                    let id = get_attr(&e, b"Id");
                    let ty = get_attr(&e, b"Type");
                    let target = get_attr(&e, b"Target");
                    if let (Some(id), Some(ty), Some(target)) = (id, ty, target) {
                        map.insert(id, (ty, target));
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    map
}

fn resolve_rel_target(base_dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        if seg == ".." {
            parts.pop();
        } else if seg == "." || seg.is_empty() {
        } else {
            parts.push(seg);
        }
    }
    parts.join("/")
}

fn build_slide_path_by_page(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
) -> Result<HashMap<u32, String>, String> {
    let pres_xml =
        read_entry(archive, "ppt/presentation.xml").ok_or("presentation.xml 읽기 실패")?;
    let rids = get_slide_rids_in_order(&pres_xml);
    let rels_xml = read_entry(archive, "ppt/_rels/presentation.xml.rels")
        .ok_or("presentation.xml.rels 읽기 실패")?;
    let rels_map = get_rels_map(&rels_xml);
    let mut map = HashMap::new();
    for (i, rid) in rids.iter().enumerate() {
        if let Some((_, target)) = rels_map.get(rid) {
            map.insert((i + 1) as u32, format!("ppt/{target}"));
        }
    }
    Ok(map)
}

fn next_unused_rid(map: &HashMap<String, (String, String)>) -> String {
    let mut n = 1u32;
    loop {
        let candidate = format!("rId{n}");
        if !map.contains_key(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn xml_escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn xml_escape_attr(s: &str) -> String {
    xml_escape_text(s).replace('"', "&quot;")
}

fn build_rels_xml(map: &HashMap<String, (String, String)>) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut entries: Vec<(&String, &(String, String))> = map.iter().collect();
    entries.sort_by_key(|(id, _)| id.trim_start_matches("rId").parse::<u32>().unwrap_or(u32::MAX));
    for (id, (ty, target)) in entries {
        s.push_str(&format!(
            r#"<Relationship Id="{}" Type="{}" Target="{}"/>"#,
            xml_escape_attr(id),
            xml_escape_attr(ty),
            xml_escape_attr(target)
        ));
    }
    s.push_str("</Relationships>");
    s
}

fn empty_rels_xml_bytes() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#
        .to_vec()
}

fn build_notes_rels_xml(slide_file_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster" Target="../notesMasters/notesMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="../slides/{slide_file_name}"/></Relationships>"#
    )
}

fn paragraphs_xml(text: &str) -> String {
    if text.trim().is_empty() {
        return r#"<a:p><a:endParaRPr lang="ko-KR" dirty="0"/></a:p>"#.to_string();
    }
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                r#"<a:p><a:endParaRPr lang="ko-KR" dirty="0"/></a:p>"#.to_string()
            } else {
                format!(
                    r#"<a:p><a:r><a:rPr lang="ko-KR" dirty="0"/><a:t>{}</a:t></a:r></a:p>"#,
                    xml_escape_text(line)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

fn build_new_notes_slide_xml(text: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{}</p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#,
        paragraphs_xml(text)
    )
}

fn add_content_type_override(xml: &[u8], part_name: &str) -> Vec<u8> {
    let s = String::from_utf8_lossy(xml);
    let insertion = format!(
        r#"<Override PartName="{}" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/></Types>"#,
        xml_escape_attr(part_name)
    );
    s.replacen("</Types>", &insertion, 1).into_bytes()
}

/// Non-body placeholder types that should never be treated as the notes text box.
const NON_BODY_PH_TYPES: &[&str] = &["sldImg", "sldNum", "dt", "ftr", "hdr", "title", "ctrTitle", "subTitle"];

fn set_notes_body_text(xml: &[u8], text: &str) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut out: Vec<u8> = Vec::new();

    let mut depth: i32 = 0;
    let mut sp_depth: i32 = -1;
    let mut is_body_shape = false;
    let mut skipping_txbody = false;
    let mut found_target = false;

    loop {
        let ev = reader.read_event_into(&mut buf).map_err(|e| e.to_string())?;
        match &ev {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                let name = e.name();
                if name.as_ref() == b"p:sp" {
                    sp_depth = depth;
                    is_body_shape = false;
                }
                if name.as_ref() == b"p:txBody" && sp_depth != -1 && is_body_shape && !found_target
                {
                    let new_body = format!(
                        r#"<p:txBody><a:bodyPr/><a:lstStyle/>{}</p:txBody>"#,
                        paragraphs_xml(text)
                    );
                    out.extend_from_slice(new_body.as_bytes());
                    skipping_txbody = true;
                    found_target = true;
                } else if !skipping_txbody {
                    out.extend_from_slice(&reader_slice(&ev)?);
                }
            }
            Event::Empty(e) => {
                let name = e.name();
                if name.as_ref() == b"p:ph" && sp_depth != -1 {
                    let ty = get_attr(e, b"type");
                    let is_non_body = matches!(ty.as_deref(), Some(t) if NON_BODY_PH_TYPES.contains(&t));
                    if !is_non_body {
                        is_body_shape = true;
                    }
                }
                if !skipping_txbody {
                    out.extend_from_slice(&reader_slice(&ev)?);
                }
            }
            Event::End(e) => {
                let name = e.name();
                if name.as_ref() == b"p:txBody" && skipping_txbody {
                    skipping_txbody = false;
                    depth -= 1;
                    continue;
                }
                if name.as_ref() == b"p:sp" && sp_depth == depth {
                    sp_depth = -1;
                    is_body_shape = false;
                }
                depth -= 1;
                if !skipping_txbody {
                    out.extend_from_slice(&reader_slice(&ev)?);
                }
            }
            _ => {
                if !skipping_txbody {
                    out.extend_from_slice(&reader_slice(&ev)?);
                }
            }
        }
        buf.clear();
    }

    if !found_target {
        return Err("메모 텍스트 상자를 찾지 못했습니다".to_string());
    }

    Ok(out)
}

fn reader_slice(ev: &Event) -> Result<Vec<u8>, String> {
    let mut w = quick_xml::writer::Writer::new(Cursor::new(Vec::new()));
    w.write_event(ev.clone()).map_err(|e| e.to_string())?;
    Ok(w.into_inner().into_inner())
}

fn apply_note(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    all_names: &[String],
    slide_path_by_page: &HashMap<u32, String>,
    overrides: &mut HashMap<String, Vec<u8>>,
    page: u32,
    text: &str,
) -> Result<(), String> {
    let slide_path = slide_path_by_page
        .get(&page)
        .ok_or_else(|| format!("슬라이드 {page}번이 존재하지 않음"))?
        .clone();
    let slide_file_name = slide_path.rsplit('/').next().unwrap().to_string();
    let slide_rels_path = format!("ppt/slides/_rels/{slide_file_name}.rels");

    let slide_rels_bytes = overrides
        .get(&slide_rels_path)
        .cloned()
        .or_else(|| read_entry(archive, &slide_rels_path))
        .unwrap_or_else(empty_rels_xml_bytes);
    let mut slide_rel_map = get_rels_map(&slide_rels_bytes);

    let existing_target = slide_rel_map
        .iter()
        .find(|(_, (ty, _))| ty.ends_with("/notesSlide"))
        .map(|(_, (_, target))| target.clone());

    if let Some(target) = existing_target {
        let notes_path = resolve_rel_target("ppt/slides", &target);
        let existing_xml = overrides
            .get(&notes_path)
            .cloned()
            .or_else(|| read_entry(archive, &notes_path))
            .ok_or_else(|| format!("{notes_path} 를 찾을 수 없음"))?;
        let updated_xml = set_notes_body_text(&existing_xml, text)?;
        overrides.insert(notes_path, updated_xml);
        return Ok(());
    }

    let has_notes_master = all_names.iter().any(|n| n == "ppt/notesMasters/notesMaster1.xml")
        || overrides.contains_key("ppt/notesMasters/notesMaster1.xml");
    if !has_notes_master {
        return Err(format!(
            "슬라이드 {page}번: 이 PPT에 메모 인프라가 없습니다. PowerPoint에서 아무 슬라이드 메모칸에나 한 글자 입력 후 저장하고 다시 시도해주세요."
        ));
    }

    let mut max_idx = 0u32;
    let scan = |names: &mut dyn Iterator<Item = &String>, max_idx: &mut u32| {
        for n in names {
            if let Some(rest) = n.strip_prefix("ppt/notesSlides/notesSlide") {
                if let Some(num) = rest.strip_suffix(".xml") {
                    if let Ok(v) = num.parse::<u32>() {
                        if v > *max_idx {
                            *max_idx = v;
                        }
                    }
                }
            }
        }
    };
    scan(&mut all_names.iter(), &mut max_idx);
    scan(&mut overrides.keys(), &mut max_idx);
    let new_idx = max_idx + 1;
    let notes_path = format!("ppt/notesSlides/notesSlide{new_idx}.xml");
    let notes_rels_path = format!("ppt/notesSlides/_rels/notesSlide{new_idx}.xml.rels");

    overrides.insert(notes_path, build_new_notes_slide_xml(text).into_bytes());
    overrides.insert(notes_rels_path, build_notes_rels_xml(&slide_file_name).into_bytes());

    let new_rid = next_unused_rid(&slide_rel_map);
    slide_rel_map.insert(
        new_rid,
        (
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide"
                .to_string(),
            format!("../notesSlides/notesSlide{new_idx}.xml"),
        ),
    );
    overrides.insert(slide_rels_path, build_rels_xml(&slide_rel_map).into_bytes());

    let ct_bytes = overrides
        .get("[Content_Types].xml")
        .cloned()
        .or_else(|| read_entry(archive, "[Content_Types].xml"))
        .ok_or("[Content_Types].xml 읽기 실패")?;
    let ct_updated =
        add_content_type_override(&ct_bytes, &format!("/ppt/notesSlides/notesSlide{new_idx}.xml"));
    overrides.insert("[Content_Types].xml".to_string(), ct_updated);

    Ok(())
}

fn write_pptx(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    all_names: &[String],
    overrides: &HashMap<String, Vec<u8>>,
    pptx_path: &str,
) -> Result<(), String> {
    let out_buf = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(out_buf);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut written: HashSet<String> = HashSet::new();

    for name in all_names {
        if name.ends_with('/') {
            writer.add_directory(name, options).map_err(|e| e.to_string())?;
            written.insert(name.clone());
            continue;
        }
        let bytes = if let Some(b) = overrides.get(name) {
            b.clone()
        } else {
            let mut f = archive.by_name(name).map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            buf
        };
        writer.start_file(name, options).map_err(|e| e.to_string())?;
        writer.write_all(&bytes).map_err(|e| e.to_string())?;
        written.insert(name.clone());
    }

    for (name, bytes) in overrides {
        if !written.contains(name) {
            writer.start_file(name, options).map_err(|e| e.to_string())?;
            writer.write_all(bytes).map_err(|e| e.to_string())?;
        }
    }

    let finished = writer.finish().map_err(|e| e.to_string())?;
    fs::write(pptx_path, finished.into_inner()).map_err(|e| format!("PPTX 저장 실패: {e}"))?;
    Ok(())
}

