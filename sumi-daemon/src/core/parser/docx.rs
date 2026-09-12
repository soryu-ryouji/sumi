//! docx 解析：zip + OOXML（quick-xml）。core.xml 书目元数据、document.xml 正文文本
//! 与标题样式大纲、首图封面（无内嵌图时由封面管线排版生成兜底）。

use crate::core::parser::{ParsedBook, TocEntry};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Read;

fn read_zip_text(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut file = archive.by_name(name).ok()?;
    let mut xml = String::new();
    file.read_to_string(&mut xml).ok()?;
    Some(xml)
}

/// core.xml：<dc:title> / <dc:creator>（可能多条，取首个创建者）
fn parse_core(xml: &str) -> (String, Vec<String>) {
    let mut title = String::new();
    let mut creators = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut target: Option<&'static str> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if name.ends_with("title") {
                    target = Some("title");
                } else if name.ends_with("creator") {
                    target = Some("creator");
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(kind) = target {
                    let text = t.unescape().unwrap_or_default().into_owned();
                    match kind {
                        "title" => title.push_str(&text),
                        _ => {
                            if !text.trim().is_empty() {
                                creators.push(text);
                            }
                        }
                    }
                }
            }
            Ok(Event::End(_)) => target = None,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (title, creators)
}

/// document.xml：段落文本流 + 段落样式（Heading1/2/3 → toc 大纲）+ 嵌入图片引用收集
struct DocContent {
    paragraphs: Vec<(Option<u8>, String)>, // (heading level, text)
    text: String,
    first_image: Option<Vec<u8>>, // 首个嵌入图（从 word/media/ 取）
}

fn parse_document(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    xml: &str,
) -> DocContent {
    let mut paragraphs = Vec::new();
    let mut text = String::new();
    let mut first_image = None;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut current_style = String::new();
    let mut current_text = String::new();
    let mut in_paragraph = false;
    let mut pending_rels: Vec<String> = Vec::new(); // r:embed 关系 id（按出现顺序）

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let local = local.rsplit(':').next().unwrap_or(&local).to_string();
                match local.as_str() {
                "p" if in_paragraph => {} // 嵌套 p（表格内）忽略层级
                "p" => {
                    in_paragraph = true;
                    current_style.clear();
                    current_text.clear();
                }
                "pStyle" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().ends_with(b"val") {
                            current_style = String::from_utf8_lossy(&attr.value).into_owned();
                        }
                    }
                }
                "blip" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().ends_with(b"embed") {
                            pending_rels.push(String::from_utf8_lossy(&attr.value).into_owned());
                        }
                    }
                }
                _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let local = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let local = local.rsplit(':').next().unwrap_or(&local).to_string();
                if local == "blip" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().ends_with(b"embed") {
                            pending_rels.push(String::from_utf8_lossy(&attr.value).into_owned());
                        }
                    }
                }
                if local == "pStyle" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().ends_with(b"val") {
                            current_style = String::from_utf8_lossy(&attr.value).into_owned();
                        }
                    }
                }
            }
            Ok(Event::Text(t)) if in_paragraph => {
                current_text.push_str(&t.unescape().unwrap_or_default());
            }
            Ok(Event::End(e)) if String::from_utf8_lossy(e.name().as_ref()).ends_with("p") => {
                in_paragraph = false;
                let level = heading_level(&current_style);
                let para = current_text.trim().to_string();
                if !para.is_empty() {
                    paragraphs.push((level, para.clone()));
                    text.push_str(&para);
                    text.push('\n');
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    // 关系表解析：rId → media 路径，取首个图片字节
    if !pending_rels.is_empty() {
        if let Some(rels) = read_zip_text(archive, "word/_rels/document.xml.rels") {
            let mut mapping = std::collections::HashMap::new();
            let mut reader = Reader::from_str(&rels);
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Empty(e)) | Ok(Event::Start(e)) if e.name().as_ref() == b"Relationship" => {
                        let mut id = String::new();
                        let mut target = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"Id" => id = String::from_utf8_lossy(&attr.value).into_owned(),
                                b"Target" => target = String::from_utf8_lossy(&attr.value).into_owned(),
                                _ => {}
                            }
                        }
                        mapping.insert(id, target);
                    }
                    Ok(Event::Eof) => break,
                    Err(_) => break,
                    _ => {}
                }
                buf.clear();
            }
            for rid in &pending_rels {
                if let Some(target) = mapping.get(rid) {
                    let media = if target.starts_with('/') {
                        target.trim_start_matches('/').to_string()
                    } else {
                        format!("word/{target}")
                    };
                    if let Ok(mut file) = archive.by_name(&media) {
                        let mut buf2 = Vec::new();
                        if file.read_to_end(&mut buf2).is_ok() {
                            first_image = Some(buf2);
                            break;
                        }
                    }
                }
            }
        }
    }

    DocContent { paragraphs, text, first_image }
}

/// Heading1/标题1/1 → Some(level)
fn heading_level(style: &str) -> Option<u8> {
    let lower = style.to_lowercase();
    if let Some(n) = lower.strip_prefix("heading") {
        let n = n.trim();
        if let Ok(level) = n.parse::<u8>() {
            if (1..=6).contains(&level) {
                return Some(level);
            }
        }
    }
    // 中文样式名：标题1 → 1
    if let Some(n) = style.strip_prefix("标题") {
        if let Ok(level) = n.parse::<u8>() {
            if (1..=6).contains(&level) {
                return Some(level);
            }
        }
    }
    None
}

pub fn parse_docx(name: &str, bytes: &[u8]) -> ParsedBook {
    let mut book = ParsedBook {
        title: name.to_string(),
        ..Default::default()
    };
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return book;
    };
    if let Some(core) = read_zip_text(&mut archive, "docProps/core.xml") {
        let (title, creators) = parse_core(&core);
        if !title.trim().is_empty() {
            book.title = title;
        }
        book.authors = creators;
    }
    if let Some(doc) = read_zip_text(&mut archive, "word/document.xml") {
        let content = parse_document(&mut archive, &doc);
        book.cover = content.first_image;
        book.fulltext = Some(content.text.clone());
        // 段落索引锚点（字符偏移）
        let mut toc: Vec<(u8, TocEntry)> = Vec::new();
        let mut offset = 0usize;
        for (level, para) in &content.paragraphs {
            let para_len = para.chars().count() + 1;
            if let Some(level) = level {
                toc.push((*level, TocEntry {
                    title: truncate_title(para),
                    anchor: format!("anchor:char-{offset}"),
                    children: Vec::new(),
                }));
            }
            offset += para_len;
        }
        book.toc = build_tree(&toc);
    }
    book
}

fn truncate_title(para: &str) -> String {
    para.chars().take(60).collect()
}

fn build_tree(flat: &[(u8, TocEntry)]) -> Vec<TocEntry> {
    fn build(flat: &[(u8, TocEntry)], start: &mut usize, parent_level: u8) -> Vec<TocEntry> {
        let mut out = Vec::new();
        while *start < flat.len() {
            let (level, entry) = &flat[*start];
            if *level <= parent_level {
                break;
            }
            let entry = entry.clone();
            *start += 1;
            let children = build(flat, start, *level);
            out.push(TocEntry { children, ..entry });
        }
        out
    }
    let mut idx = 0;
    build(flat, &mut idx, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn build_docx() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            let core = r#"<?xml version="1.0"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title>论文标题</dc:title>
  <dc:creator>张三</dc:creator>
</cp:coreProperties>"#;
            zip.start_file("docProps/core.xml", opts).unwrap();
            zip.write_all(core.as_bytes()).unwrap();

            let img: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
            zip.start_file("word/media/image1.png", opts).unwrap();
            zip.write_all(img).unwrap();

            let rels = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId5" Type=".../image" Target="media/image1.png"/>
</Relationships>"#;
            zip.start_file("word/_rels/document.xml.rels", opts).unwrap();
            zip.write_all(rels.as_bytes()).unwrap();

            let doc = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>引言</w:t></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="rId5"/></w:drawing></w:r><w:r><w:t>这里是正文内容，包含深度学习的方法。</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>方法</w:t></w:r></w:p>
<w:p><w:r><w:t>正文第二章内容。</w:t></w:r></w:p>
</w:body></w:document>"#;
            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(doc.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn docx_full_parse() {
        let bytes = build_docx();
        let book = parse_docx("备份名", &bytes);
        assert_eq!(book.title, "论文标题");
        assert_eq!(book.authors, vec!["张三".to_string()]);
        // 首图封面
        assert_eq!(&book.cover.unwrap()[..4], &[0x89, 0x50, 0x4E, 0x47]);
        // 正文
        let text = book.fulltext.unwrap();
        assert!(text.contains("深度学习"));
        assert!(text.contains("正文第二章内容"));
        // 标题大纲
        assert_eq!(book.toc.len(), 1);
        assert_eq!(book.toc[0].title, "引言");
        assert_eq!(book.toc[0].children.len(), 1);
        assert_eq!(book.toc[0].children[0].title, "方法");
    }

    #[test]
    fn heading_level_variants() {
        assert_eq!(heading_level("Heading1"), Some(1));
        assert_eq!(heading_level("heading3"), Some(3));
        assert_eq!(heading_level("标题2"), Some(2));
        assert_eq!(heading_level("Normal"), None);
        assert_eq!(heading_level("Heading9"), None);
    }
}
