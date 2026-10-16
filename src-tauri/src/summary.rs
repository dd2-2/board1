//! 요약 TXT + 3페이지 디자인 템플릿 PPTX → 완성본 PPTX.
//! 데스크톱(src-tauri)과 웹(web/src/lib.rs가 #[path]로 이 파일을 그대로 포함)이 공유하는 순수 로직.
//! 파일 I/O 없음 — 바이트를 받아 바이트를 돌려준다.
//!
//! 템플릿 규칙: 1페이지 표지 {{차시타이틀}}, 2페이지 {{학습목표}}/{{학습내용}},
//! 3페이지 본문 디자인 {{대주제}}/{{소주제1}}/{{본문1}}/{{소주제2}}/{{본문2}}.
//! 자리표시가 들어 있는 텍스트 도형의 첫 문단 서식(글꼴·글머리표 등)을 모든 항목 문단에 똑같이 적용한다.
//! 3페이지는 본문 수만큼 복제한다. 페이지 번호는 템플릿의 슬라이드 번호 필드가 알아서 갱신한다.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

pub struct Summary {
    pub title: String,
    pub objectives: Vec<String>,
    pub contents: Vec<String>,
    pub bodies: Vec<Body>,
}

pub struct Body {
    pub topic: String,
    pub sub1: String,
    pub items1: Vec<String>,
    pub sub2: String,
    pub items2: Vec<String>,
}

// ---- TXT 파싱 ----

#[derive(Default)]
struct BodyDraft {
    header_line: usize,
    number: u32,
    topic: Option<String>,
    sub1: Option<String>,
    items1: Option<Vec<String>>,
    sub2: Option<String>,
    items2: Option<Vec<String>>,
}

#[derive(Clone, Copy)]
enum Section {
    None,
    Title,
    Objectives,
    Contents,
    Body(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum ListTarget {
    Items1,
    Items2,
}

/// 정해진 형식만 허용한다. 형식 밖 문장·누락·중복은 전부 줄 번호와 함께 오류로 모아서 돌려준다.
pub fn parse_summary(text: &str) -> Result<Summary, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut errors: Vec<String> = Vec::new();

    let mut title_header: Option<usize> = None;
    let mut title: Option<String> = None;
    let mut objectives_header: Option<usize> = None;
    let mut objectives: Vec<String> = Vec::new();
    let mut contents_header: Option<usize> = None;
    let mut contents: Vec<String> = Vec::new();
    let mut bodies: Vec<BodyDraft> = Vec::new();

    let mut section = Section::None;
    let mut list_target: Option<ListTarget> = None;

    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let err = |msg: String| format!("TXT {n}번째 줄: {msg}");

        if line.starts_with('[') && line.ends_with(']') {
            let name = line[1..line.len() - 1].trim();
            list_target = None;
            match name {
                "차시명" | "학습목표" | "학습내용" => {
                    let slot = match name {
                        "차시명" => &mut title_header,
                        "학습목표" => &mut objectives_header,
                        _ => &mut contents_header,
                    };
                    if slot.is_some() {
                        errors.push(err(format!("[{name}]이(가) 중복되었습니다.")));
                    }
                    *slot = Some(n);
                    section = match name {
                        "차시명" => Section::Title,
                        "학습목표" => Section::Objectives,
                        _ => Section::Contents,
                    };
                }
                _ => match name.strip_prefix("본문").map(|s| s.trim().parse::<u32>()) {
                    Some(Ok(num)) => {
                        bodies.push(BodyDraft { header_line: n, number: num, ..Default::default() });
                        section = Section::Body(bodies.len() - 1);
                    }
                    _ => {
                        errors.push(err(format!("알 수 없는 섹션입니다: {line}")));
                        section = Section::None;
                    }
                },
            }
            continue;
        }

        let list_item = line.strip_prefix("- ").map(str::trim);
        match section {
            Section::None => errors.push(err(format!("섹션([차시명] 등) 밖의 문장입니다: {line}"))),
            Section::Title => {
                if list_item.is_some() {
                    errors.push(err("차시명은 '- ' 없이 한 줄로 작성해야 합니다.".into()));
                } else if title.is_some() {
                    errors.push(err("차시명은 한 줄이어야 합니다.".into()));
                } else {
                    title = Some(line.to_string());
                }
            }
            Section::Objectives | Section::Contents => {
                let label = if matches!(section, Section::Objectives) { "학습목표" } else { "학습내용" };
                match list_item {
                    Some(item) if !item.is_empty() => {
                        if matches!(section, Section::Objectives) {
                            objectives.push(item.to_string());
                        } else {
                            contents.push(item.to_string());
                        }
                    }
                    Some(_) => errors.push(err(format!("{label} 항목이 비어 있습니다."))),
                    None => errors.push(err(format!("{label} 항목은 '- '로 시작해야 합니다: {line}"))),
                }
            }
            Section::Body(idx) => {
                let body = &mut bodies[idx];
                let num = body.number;
                if let Some(item) = list_item {
                    let target = match list_target {
                        Some(ListTarget::Items1) => body.items1.as_mut(),
                        Some(ListTarget::Items2) => body.items2.as_mut(),
                        None => None,
                    };
                    match target {
                        Some(_) if item.is_empty() => errors.push(err("목록 항목이 비어 있습니다.".into())),
                        Some(list) => list.push(item.to_string()),
                        None => errors.push(err("목록은 '본문1:' 또는 '본문2:' 다음 줄부터 쓸 수 있습니다.".into())),
                    }
                    continue;
                }
                let Some((key, value)) = line.split_once(':') else {
                    errors.push(err(format!("[본문 {num}]에서 알 수 없는 문장입니다: {line}")));
                    continue;
                };
                let key = key.trim();
                let value = value.trim();
                match key {
                    "대주제" | "소주제1" | "소주제2" => {
                        list_target = None;
                        let slot = match key {
                            "대주제" => &mut body.topic,
                            "소주제1" => &mut body.sub1,
                            _ => &mut body.sub2,
                        };
                        if slot.is_some() {
                            errors.push(err(format!("[본문 {num}]에 {key}이(가) 중복되었습니다.")));
                        } else if value.is_empty() {
                            errors.push(err(format!("[본문 {num}]의 {key} 내용이 비어 있습니다.")));
                        } else {
                            *slot = Some(value.to_string());
                        }
                    }
                    "본문1" | "본문2" => {
                        let (slot, target) = if key == "본문1" {
                            (&mut body.items1, ListTarget::Items1)
                        } else {
                            (&mut body.items2, ListTarget::Items2)
                        };
                        if slot.is_some() {
                            errors.push(err(format!("[본문 {num}]에 {key}이(가) 중복되었습니다.")));
                            list_target = None;
                        } else if !value.is_empty() {
                            errors.push(err(format!("{key}: 뒤에는 줄을 바꿔 '- ' 목록으로 작성해야 합니다.")));
                            list_target = None;
                        } else {
                            *slot = Some(Vec::new());
                            list_target = Some(target);
                        }
                    }
                    _ => errors.push(err(format!("[본문 {num}]에서 알 수 없는 항목입니다: {key}"))),
                }
            }
        }
    }

    match (title_header, &title) {
        (None, _) => errors.push("[차시명]이 없습니다.".into()),
        (Some(h), None) => errors.push(format!("TXT {h}번째 줄: 차시명 내용이 비어 있습니다.")),
        _ => {}
    }
    match objectives_header {
        None => errors.push("[학습목표]가 없습니다.".into()),
        Some(h) if objectives.is_empty() || objectives.len() > 3 => errors.push(format!(
            "TXT {h}번째 줄: 학습목표는 1~3개여야 합니다 (현재 {}개).",
            objectives.len()
        )),
        _ => {}
    }
    if contents_header.is_none() {
        errors.push("[학습내용]이 없습니다.".into());
    }

    if bodies.is_empty() {
        errors.push("[본문 1]이 없습니다.".into());
    } else if bodies.len() > 10 {
        errors.push(format!("본문은 1~10개까지 허용됩니다 (현재 {}개).", bodies.len()));
    }

    let mut out_bodies = Vec::new();
    for (i, b) in bodies.iter().enumerate() {
        let h = b.header_line;
        let num = b.number;
        if num as usize != i + 1 {
            errors.push(format!(
                "TXT {h}번째 줄: 본문 번호가 연속되지 않습니다 ([본문 {}] 자리에 [본문 {num}]).",
                i + 1
            ));
        }
        let mut missing = |name: &str| errors.push(format!("TXT {h}번째 줄: [본문 {num}]에 {name}이(가) 누락되었습니다."));
        if b.topic.is_none() {
            missing("대주제");
        }
        if b.sub1.is_none() {
            missing("소주제1");
        }
        if b.items1.is_none() {
            missing("본문1");
        }
        if b.sub2.is_none() {
            missing("소주제2");
        }
        if b.items2.is_none() {
            missing("본문2");
        }
        for (name, items) in [("본문1", &b.items1), ("본문2", &b.items2)] {
            if let Some(items) = items {
                if items.is_empty() || items.len() > 5 {
                    errors.push(format!(
                        "TXT {h}번째 줄: [본문 {num}]의 {name} 항목은 1~5개여야 합니다 (현재 {}개).",
                        items.len()
                    ));
                }
            }
        }
        if let (Some(topic), Some(sub1), Some(items1), Some(sub2), Some(items2)) =
            (&b.topic, &b.sub1, &b.items1, &b.sub2, &b.items2)
        {
            out_bodies.push(Body {
                topic: topic.clone(),
                sub1: sub1.clone(),
                items1: items1.clone(),
                sub2: sub2.clone(),
                items2: items2.clone(),
            });
        }
    }

    if contents_header.is_some() && !bodies.is_empty() && contents.len() != bodies.len() {
        errors.push(format!(
            "학습내용 수({}개)와 본문 페이지 수({}개)가 다릅니다.",
            contents.len(),
            bodies.len()
        ));
    }

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(Summary { title: title.unwrap(), objectives, contents, bodies: out_bodies })
}

// ---- PPTX 생성 ----

const SLIDE_REL_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
const SLIDE_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";

/// 파싱 + 생성을 한 번에. 성공하면 (완성본 바이트, 총 슬라이드 수).
/// 호출 전에 zip 확장 타임스탬프 정리(sanitize_extended_timestamp_fields)는 호출부가 한다.
pub fn build_summary_pptx(template: Vec<u8>, txt: &str) -> Result<(Vec<u8>, usize), String> {
    let summary = parse_summary(txt)?;
    let mut archive =
        ZipArchive::new(Cursor::new(template)).map_err(|e| format!("PPTX(zip) 열기 실패: {e}"))?;
    let all_names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
    let name_set: HashSet<&str> = all_names.iter().map(|s| s.as_str()).collect();

    let mut pres = read_text(&mut archive, "ppt/presentation.xml")?;
    let mut pres_rels = read_text(&mut archive, "ppt/_rels/presentation.xml.rels")?;

    let list_start = pres.find("<p:sldIdLst>").ok_or("presentation.xml에 슬라이드 목록이 없습니다.")?;
    let list_end = pres[list_start..].find("</p:sldIdLst>").ok_or("presentation.xml 슬라이드 목록이 손상되었습니다.")?
        + list_start;
    let sld_ids: Vec<(u32, String)> = tags(&pres[list_start..list_end], "p:sldId")
        .into_iter()
        .filter_map(|t| Some((attr(t, "id")?.parse().ok()?, attr(t, "r:id")?)))
        .collect();
    if sld_ids.len() != 3 {
        return Err(format!(
            "포맷은 표지·학습 안내·본문 디자인으로 구성된 3페이지여야 합니다 (현재 {}페이지).",
            sld_ids.len()
        ));
    }

    let rel_targets: HashMap<String, String> = tags(&pres_rels, "Relationship")
        .into_iter()
        .filter_map(|t| Some((attr(t, "Id")?, attr(t, "Target")?)))
        .collect();
    let slide_paths: Vec<String> = sld_ids
        .iter()
        .map(|(_, rid)| {
            let target = rel_targets.get(rid).ok_or(format!("슬라이드 관계 {rid}를 찾을 수 없습니다."))?;
            Ok(match target.strip_prefix('/') {
                Some(abs) => abs.to_string(),
                None => format!("ppt/{target}"),
            })
        })
        .collect::<Result<_, String>>()?;

    let mut overrides: HashMap<String, Vec<u8>> = HashMap::new();

    let cover = read_text(&mut archive, &slide_paths[0])?;
    let cover = fill_slide(&cover, 1, &[(&["차시타이틀", "차시명"], vec![summary.title.clone()])])?;
    overrides.insert(slide_paths[0].clone(), cover.into_bytes());

    let guide = read_text(&mut archive, &slide_paths[1])?;
    let guide = fill_slide(
        &guide,
        2,
        &[(&["학습목표"], summary.objectives.clone()), (&["학습내용"], summary.contents.clone())],
    )?;
    overrides.insert(slide_paths[1].clone(), guide.into_bytes());

    let body_template = read_text(&mut archive, &slide_paths[2])?;
    let body_rels_path = rels_path_of(&slide_paths[2]);
    let body_rels = read_text(&mut archive, &body_rels_path)?;
    // 복제본은 원본 3페이지의 메모(notesSlide)를 공유하면 안 되므로 그 관계만 뺀다.
    let dup_rels = tags(&body_rels, "Relationship")
        .into_iter()
        .filter(|t| attr(t, "Type").is_some_and(|ty| ty.ends_with("/notesSlide")))
        .fold(body_rels.clone(), |acc, t| acc.replacen(t, "", 1));

    let mut used_rids: HashSet<String> = rel_targets.keys().cloned().collect();
    let mut next_sld_id = sld_ids.iter().map(|(id, _)| *id).max().unwrap_or(255) + 1;
    let mut next_slide_no = 1u32;
    let mut new_sld_entries = String::new();
    let mut new_rels = String::new();
    let mut new_ct = String::new();

    for (i, body) in summary.bodies.iter().enumerate() {
        let page = i + 3;
        let fills: [(&[&str], Vec<String>); 5] = [
            (&["대주제"], vec![body.topic.clone()]),
            (&["소주제1"], vec![format!("①\t{}", body.sub1)]),
            (&["본문1"], body.items1.clone()),
            (&["소주제2"], vec![format!("②\t{}", body.sub2)]),
            (&["본문2"], body.items2.clone()),
        ];
        let xml = fill_slide(&body_template, page, &fills)?;
        if i == 0 {
            overrides.insert(slide_paths[2].clone(), xml.into_bytes());
            continue;
        }

        while name_set.contains(format!("ppt/slides/slide{next_slide_no}.xml").as_str())
            || overrides.contains_key(&format!("ppt/slides/slide{next_slide_no}.xml"))
        {
            next_slide_no += 1;
        }
        let slide_name = format!("slide{next_slide_no}.xml");
        overrides.insert(format!("ppt/slides/{slide_name}"), bump_creation_id(&xml, i as u32).into_bytes());
        overrides.insert(format!("ppt/slides/_rels/{slide_name}.rels"), dup_rels.clone().into_bytes());

        let mut r = 1u32;
        while used_rids.contains(&format!("rId{r}")) {
            r += 1;
        }
        let rid = format!("rId{r}");
        used_rids.insert(rid.clone());

        new_rels.push_str(&format!(
            r#"<Relationship Id="{rid}" Type="{SLIDE_REL_TYPE}" Target="slides/{slide_name}"/>"#
        ));
        new_sld_entries.push_str(&format!(r#"<p:sldId id="{next_sld_id}" r:id="{rid}"/>"#));
        new_ct.push_str(&format!(
            r#"<Override PartName="/ppt/slides/{slide_name}" ContentType="{SLIDE_CONTENT_TYPE}"/>"#
        ));
        next_sld_id += 1;
    }

    if !new_sld_entries.is_empty() {
        pres.insert_str(list_end, &new_sld_entries);
        let pos = pres_rels.rfind("</Relationships>").ok_or("presentation.xml.rels가 손상되었습니다.")?;
        pres_rels.insert_str(pos, &new_rels);
        let mut ct = read_text(&mut archive, "[Content_Types].xml")?;
        let pos = ct.rfind("</Types>").ok_or("[Content_Types].xml이 손상되었습니다.")?;
        ct.insert_str(pos, &new_ct);
        overrides.insert("ppt/presentation.xml".into(), pres.into_bytes());
        overrides.insert("ppt/_rels/presentation.xml.rels".into(), pres_rels.into_bytes());
        overrides.insert("[Content_Types].xml".into(), ct.into_bytes());
    }

    let total = summary.bodies.len() + 2;
    if name_set.contains("docProps/app.xml") {
        let app = read_text(&mut archive, "docProps/app.xml")?;
        if let Some(s) = app.find("<Slides>") {
            if let Some(e) = app[s..].find("</Slides>") {
                let updated = format!("{}<Slides>{total}{}", &app[..s], &app[s + e..]);
                overrides.insert("docProps/app.xml".into(), updated.into_bytes());
            }
        }
    }

    let bytes = write_zip(&mut archive, &all_names, &overrides)?;
    Ok((bytes, total))
}

/// 슬라이드 XML에서 텍스트 전체가 `{{이름}}`인 텍스트 도형을 찾아 내용을 바꾼다.
/// 각 자리표시는 해당 슬라이드에 정확히 한 번 있어야 한다.
fn fill_slide(xml: &str, page: usize, fills: &[(&[&str], Vec<String>)]) -> Result<String, String> {
    let mut found = vec![0usize; fills.len()];
    let mut out = String::with_capacity(xml.len());
    let mut pos = 0;
    while let Some(rel) = xml[pos..].find("<p:txBody>") {
        let body_start = pos + rel + "<p:txBody>".len();
        let body_end = xml[body_start..].find("</p:txBody>").ok_or("슬라이드 XML이 손상되었습니다.")? + body_start;
        out.push_str(&xml[pos..body_start]);
        let inner = &xml[body_start..body_end];

        let plain = paragraph_texts(inner).join("\n");
        let key = plain.trim().strip_prefix("{{").and_then(|s| s.strip_suffix("}}")).map(str::trim);
        let hit = key.and_then(|k| fills.iter().position(|(names, _)| names.contains(&k)));
        match hit {
            Some(idx) => {
                found[idx] += 1;
                out.push_str(&rebuild_text_body(inner, &fills[idx].1));
            }
            None => out.push_str(inner),
        }
        pos = body_end;
    }
    out.push_str(&xml[pos..]);

    for (idx, (names, _)) in fills.iter().enumerate() {
        match found[idx] {
            1 => {}
            0 => return Err(format!("{page}페이지에 {{{{{}}}}} 자리표시가 없습니다.", names[0])),
            _ => return Err(format!("{page}페이지에 {{{{{}}}}} 자리표시가 중복되어 있습니다.", names[0])),
        }
    }
    Ok(out)
}

/// txBody 내부를 bodyPr/lstStyle은 그대로 두고, 문단만 새로 만든다.
/// 서식은 run이 있는 첫 문단의 pPr과, 자리표시 이름이 들어 있던 run의 rPr을 모든 문단에 복사한다.
fn rebuild_text_body(inner: &str, lines: &[String]) -> String {
    let paras = paragraph_spans(inner);
    let prefix = &inner[..paras.first().map_or(inner.len(), |p| p.0)];

    let mut ppr = String::new();
    let mut rpr = String::new();
    for &(s, e) in &paras {
        let para = &inner[s..e];
        let runs = element_spans(para, "a:r");
        if runs.is_empty() {
            continue;
        }
        ppr = element_spans(para, "a:pPr").first().map_or(String::new(), |&(a, b)| para[a..b].to_string());
        let pick = runs
            .iter()
            .find(|&&(a, b)| {
                let t: String = texts(&para[a..b]).concat();
                !t.replace(['{', '}'], "").trim().is_empty()
            })
            .unwrap_or(&runs[0]);
        let run = &para[pick.0..pick.1];
        rpr = element_spans(run, "a:rPr").first().map_or(String::new(), |&(a, b)| run[a..b].replace(r#" err="1""#, ""));
        break;
    }

    let mut out = String::from(prefix);
    for line in lines {
        out.push_str(&format!("<a:p>{ppr}<a:r>{rpr}<a:t>{}</a:t></a:r></a:p>", xml_escape(line)));
    }
    out
}

/// 복제 슬라이드마다 p14:creationId 값을 달리한다(같은 값이 여러 슬라이드에 있으면 PowerPoint가 혼동할 수 있음).
fn bump_creation_id(xml: &str, offset: u32) -> String {
    let Some(s) = xml.find("<p14:creationId") else { return xml.to_string() };
    let Some(v) = xml[s..].find(r#"val=""#) else { return xml.to_string() };
    let vs = s + v + r#"val=""#.len();
    let Some(ve) = xml[vs..].find('"') else { return xml.to_string() };
    let orig: u32 = xml[vs..vs + ve].parse().unwrap_or(0);
    format!("{}{}{}", &xml[..vs], orig.wrapping_add(offset.wrapping_mul(7919)), &xml[vs + ve..])
}

// ---- 작은 문자열 기반 XML 도우미 (PowerPoint가 저장한 XML 전제) ----

/// `<name` 바로 뒤가 공백·`>`·`/`인 위치만 태그 시작으로 인정 (`<a:p`가 `<a:pPr`에 걸리지 않도록).
fn find_tag(s: &str, from: usize, name: &str) -> Option<usize> {
    let open = format!("<{name}");
    let mut pos = from;
    while let Some(rel) = s[pos..].find(&open) {
        let at = pos + rel;
        match s.as_bytes().get(at + open.len()) {
            Some(b' ') | Some(b'>') | Some(b'/') => return Some(at),
            _ => pos = at + open.len(),
        }
    }
    None
}

/// 같은 이름이 중첩되지 않는 요소들의 (시작, 끝) 바이트 범위. 자기 닫힘 태그도 처리.
fn element_spans(s: &str, name: &str) -> Vec<(usize, usize)> {
    let close = format!("</{name}>");
    let mut spans = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_tag(s, pos, name) {
        let Some(gt) = s[start..].find('>') else { break };
        let tag_end = start + gt + 1;
        let end = if s[..tag_end].ends_with("/>") {
            tag_end
        } else {
            match s[tag_end..].find(&close) {
                Some(c) => tag_end + c + close.len(),
                None => break,
            }
        };
        spans.push((start, end));
        pos = end;
    }
    spans
}

fn paragraph_spans(inner: &str) -> Vec<(usize, usize)> {
    element_spans(inner, "a:p")
}

fn texts(s: &str) -> Vec<&str> {
    element_spans(s, "a:t")
        .into_iter()
        .filter_map(|(a, b)| {
            let el = &s[a..b];
            let gt = el.find('>')?;
            el[gt + 1..].strip_suffix("</a:t>")
        })
        .collect()
}

fn paragraph_texts(inner: &str) -> Vec<String> {
    paragraph_spans(inner).into_iter().map(|(a, b)| texts(&inner[a..b]).concat()).collect()
}

/// 여는 태그 문자열들 (`<name ...>` 또는 `<name .../>`).
fn tags<'a>(s: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_tag(s, pos, name) {
        let Some(gt) = s[start..].find('>') else { break };
        out.push(&s[start..start + gt + 1]);
        pos = start + gt + 1;
    }
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let s = tag.find(&key)? + key.len();
    let e = tag[s..].find('"')?;
    Some(tag[s..s + e].to_string())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn rels_path_of(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &part[..i], &part[i + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

fn read_text(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Result<String, String> {
    let mut f = archive.by_name(name).map_err(|_| format!("PPTX 안에 {name}이(가) 없습니다."))?;
    let mut s = String::new();
    f.read_to_string(&mut s).map_err(|e| format!("{name} 읽기 실패: {e}"))?;
    Ok(s)
}

fn write_zip(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    all_names: &[String],
    overrides: &HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut written: HashSet<&str> = HashSet::new();

    for name in all_names {
        if name.ends_with('/') {
            writer.add_directory(name, options).map_err(|e| e.to_string())?;
            continue;
        }
        let bytes = match overrides.get(name) {
            Some(b) => b.clone(),
            None => {
                let mut f = archive.by_name(name).map_err(|e| e.to_string())?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                buf
            }
        };
        writer.start_file(name, options).map_err(|e| e.to_string())?;
        writer.write_all(&bytes).map_err(|e| e.to_string())?;
        written.insert(name);
    }

    let mut extra: Vec<&String> = overrides.keys().filter(|n| !written.contains(n.as_str())).collect();
    extra.sort();
    for name in extra {
        writer.start_file(name.as_str(), options).map_err(|e| e.to_string())?;
        writer.write_all(&overrides[name]).map_err(|e| e.to_string())?;
    }

    Ok(writer.finish().map_err(|e| e.to_string())?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}[차시명]\n엑셀 기초\n\n[학습목표]\n- 목표 하나\n\n[학습내용]\n- 내용1\n- 내용2\n\n[본문 1]\n대주제: 주제1\n소주제1: 소1\n본문1:\n- =IF(C2>=500,\"high\",\"low\")\n소주제2: 소2\n본문2:\n- $A$1 참조\n\n[본문 2]\n대주제: 주제2\n소주제1: 소1\n본문1:\n- a\n소주제2: 소2\n본문2:\n- b\n";

    #[test]
    fn parses_sample() {
        let s = parse_summary(SAMPLE).unwrap();
        assert_eq!(s.title, "엑셀 기초");
        assert_eq!(s.bodies.len(), 2);
        assert_eq!(s.bodies[0].items1[0], "=IF(C2>=500,\"high\",\"low\")");
    }

    #[test]
    fn reports_missing_and_count_mismatch() {
        let txt = SAMPLE.replace("소주제2: 소2\n본문2:\n- b\n", "").replace("- 내용2\n", "");
        let err = parse_summary(&txt).err().unwrap();
        assert!(err.contains("[본문 2]에 소주제2이(가) 누락"), "{err}");
        assert!(err.contains("학습내용 수(1개)와 본문 페이지 수(2개)"), "{err}");
    }

    /// 실제 템플릿으로 생성 확인용: SUMMARY_TEMPLATE, SUMMARY_TXT, SUMMARY_OUT 환경변수 지정 후 `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn builds_from_real_template() {
        let template = std::fs::read(std::env::var("SUMMARY_TEMPLATE").unwrap()).unwrap();
        let txt = std::fs::read_to_string(std::env::var("SUMMARY_TXT").unwrap()).unwrap();
        let (bytes, count) = build_summary_pptx(template, &txt).unwrap();
        std::fs::write(std::env::var("SUMMARY_OUT").unwrap(), bytes).unwrap();
        println!("slides: {count}");
    }

    #[test]
    fn rejects_stray_text() {
        let body = SAMPLE.trim_start_matches('\u{feff}');
        let err = parse_summary(&format!("다음은 요약입니다.\n{body}")).err().unwrap();
        assert!(err.starts_with("TXT 1번째 줄"), "{err}");
    }
}
