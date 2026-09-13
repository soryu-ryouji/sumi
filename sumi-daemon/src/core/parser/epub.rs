//! epub 解析：zip 容器 + 自写 OPF/NCX/nav 解析（quick-xml）。
//! 产出：书目元数据（OPF metadata，含 calibre 系列）、封面（cover-image 属性或
//! cover.xhtml 首图）、目录（nav 优先，NCX 回退）、正文全文文本（spine 顺序拼接，
//! 供全文索引）、归一化 HTML（item/content 用：章节拼接 + 锚点注入 + 资源改写）。

use crate::core::parser::{ParsedBook, TocEntry};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Read;

pub struct EpubBook {
    pub book: ParsedBook,
    /// spine 顺序的 XHTML 文档（href 相对 OPF 路径，供归一化转换）
    pub spine_docs: Vec<String>,
    /// 解析上下文（资源改写的相对路径基准）
    pub opf_dir: String,
}

/// 解析 META-INF/container.xml 定位 OPF
fn locate_opf(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Option<(String, String)> {
    let mut file = archive.by_name("META-INF/container.xml").ok()?;
    let mut xml = String::new();
    file.read_to_string(&mut xml).ok()?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut full_path = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"rootfile" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"full-path" {
                            full_path = Some(String::from_utf8_lossy(&attr.value).into_owned());
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    let path = full_path?;
    let dir = match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    };
    Some((path, dir))
}

#[derive(Default)]
struct OpfMetadata {
    title: String,
    creators: Vec<String>,
    publisher: String,
    pubdate: String,
    identifier: String,
    language: String,
    description: String,
    // calibre 系列约定：<meta name="calibre:series" content="..."/> + series_index
    series: String,
    series_index: f64,
}

#[derive(Default, Clone)]
struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
    properties: String,
}

#[derive(Default)]
struct Opf {
    metadata: OpfMetadata,
    manifest: Vec<ManifestItem>,
    spine: Vec<String>,
    cover_item_id: Option<String>,
}

fn parse_opf(xml: &str) -> Opf {
    let mut opf = Opf::default();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut section = String::new();
    let mut text_target: Option<&'static str> = None;
    let mut pending_meta_name = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let name_str = String::from_utf8_lossy(name.as_ref()).into_owned();
                match name_str.as_str() {
                    "metadata" => section = "m".into(),
                    "manifest" => section = "mf".into(),
                    "spine" => {
                        section = "s".into();
                        // spine 上的 toc 属性（NCX id）与 cover 属性（部分制作器）
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"toc" {
                                opf.cover_item_id.get_or_insert_with(Default::default);
                                let _ = attr;
                            }
                        }
                    }
                    _ => {}
                }
                if section == "m" {
                    // dc:title 等带命名空间前缀，按本地名后缀匹配
                    if name_str == "meta" || name_str.ends_with(":meta") {
                        pending_meta_name.clear();
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"name" {
                                pending_meta_name = String::from_utf8_lossy(&attr.value).into_owned();
                            }
                        }
                    } else {
                        for (suffix, target) in [
                            ("title", "title"),
                            ("creator", "creator"),
                            ("publisher", "publisher"),
                            ("date", "date"),
                            ("identifier", "identifier"),
                            ("language", "language"),
                            ("description", "description"),
                        ] {
                            if name_str == suffix || name_str.ends_with(&format!(":{suffix}")) {
                                text_target = Some(target);
                                break;
                            }
                        }
                    }
                } else if section == "mf" && (name_str == "item" || name_str.ends_with(":item")) {
                    let mut item = ManifestItem::default();
                    let mut is_cover = false;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"id" => item.id = String::from_utf8_lossy(&attr.value).into_owned(),
                            b"href" => item.href = String::from_utf8_lossy(&attr.value).into_owned(),
                            b"media-type" => item.media_type = String::from_utf8_lossy(&attr.value).into_owned(),
                            b"properties" => item.properties = String::from_utf8_lossy(&attr.value).into_owned(),
                            _ => {}
                        }
                        if attr.key.as_ref() == b"name" && attr.value.as_ref() == b"cover" {
                            is_cover = true;
                        }
                    }
                    if is_cover {
                        opf.cover_item_id = Some(item.id.clone());
                    }
                    if item.properties.contains("cover-image") {
                        opf.cover_item_id = Some(item.id.clone());
                    }
                    opf.manifest.push(item);
                } else if section == "s" && name_str == "itemref" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"idref" {
                            opf.spine.push(String::from_utf8_lossy(&attr.value).into_owned());
                        }
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name_str = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if section == "m" && name_str == "meta" {
                    let mut name = String::new();
                    let mut content = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"name" => name = String::from_utf8_lossy(&attr.value).into_owned(),
                            b"content" => content = String::from_utf8_lossy(&attr.value).into_owned(),
                            _ => {}
                        }
                    }
                    apply_meta(&mut opf, &name, &content);
                }
                if section == "mf" && name_str == "item" {
                    let mut item = ManifestItem::default();
                    let mut is_cover = false;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"id" => item.id = String::from_utf8_lossy(&attr.value).into_owned(),
                            b"href" => item.href = String::from_utf8_lossy(&attr.value).into_owned(),
                            b"media-type" => item.media_type = String::from_utf8_lossy(&attr.value).into_owned(),
                            b"properties" => item.properties = String::from_utf8_lossy(&attr.value).into_owned(),
                            _ => {}
                        }
                        if attr.key.as_ref() == b"name" && attr.value.as_ref() == b"cover" {
                            is_cover = true;
                        }
                    }
                    if is_cover || item.properties.contains("cover-image") {
                        opf.cover_item_id = Some(item.id.clone());
                    }
                    opf.manifest.push(item);
                }
                if section == "s" && name_str == "itemref" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"idref" {
                            opf.spine.push(String::from_utf8_lossy(&attr.value).into_owned());
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(target) = text_target {
                    let text = t.unescape().unwrap_or_default().into_owned();
                    match target {
                        "title" => opf.metadata.title.push_str(&text),
                        "creator" => opf.metadata.creators.push(text),
                        "publisher" => opf.metadata.publisher.push_str(&text),
                        "date" => opf.metadata.pubdate.push_str(&text),
                        "identifier" => opf.metadata.identifier.push_str(&text),
                        "language" => opf.metadata.language.push_str(&text),
                        "description" => opf.metadata.description.push_str(&text),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name_str = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                match name_str.as_str() {
                    "metadata" | "manifest" | "spine" => section.clear(),
                    _ => {}
                }
                text_target = None;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    opf
}

fn apply_meta(opf: &mut Opf, name: &str, content: &str) {
    if name == "calibre:series" {
        opf.metadata.series = content.to_string();
    } else if name == "calibre:series_index" {
        opf.metadata.series_index = content.parse().unwrap_or(0.0);
    }
}

/// href 相对路径拼接（相对 OPF 所在目录）+ `.`/`..` 组件文本归约
/// （zip 内条目名是字面量，相对上行段必须折叠，否则 by_name 取不到）；
/// 归一化 content 的资源改写也复用
pub(crate) fn join_href(opf_dir: &str, href: &str) -> String {
    let decoded = percent_encoding::percent_decode_str(href)
        .decode_utf8_lossy()
        .into_owned();
    let joined = if opf_dir.is_empty() {
        decoded
    } else if decoded.starts_with('/') {
        decoded.trim_start_matches('/').to_string()
    } else {
        format!("{opf_dir}/{decoded}")
    };
    let mut out: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// nav（EPUB3）解析：嵌套 <nav epub:type="toc"> 的 <ol><li><a>
fn parse_nav(xml: &str, nav_path: &str) -> Vec<TocEntry> {
    let nav_dir = match nav_path.rfind('/') {
        Some(i) => &nav_path[..i],
        None => "",
    };
    let mut flat: Vec<(u8, TocEntry)> = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut in_toc_nav = false;
    let mut depth = 0u8;
    let mut href = String::new();
    let mut in_a = false;
    let mut text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"nav" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"type" || attr.key.as_ref().ends_with(b"type") {
                            if attr.value.as_ref() == b"toc" {
                                in_toc_nav = true;
                            }
                        }
                    }
                }
                b"ol" if in_toc_nav => depth += 1,
                b"a" if in_toc_nav => {
                    href.clear();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            href = String::from_utf8_lossy(&attr.value).into_owned();
                        }
                    }
                    in_a = true;
                    text.clear();
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if in_toc_nav && e.name().as_ref() == b"a" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            href = String::from_utf8_lossy(&attr.value).into_owned();
                        }
                    }
                    if !href.is_empty() {
                        let h = join_href(nav_dir, &href);
                        flat.push((depth.max(1), TocEntry {
                            title: String::new(),
                            anchor: format!("anchor:href:{h}"),
                            children: Vec::new(),
                        }));
                        href.clear();
                    }
                }
            }
            Ok(Event::Text(t)) if in_a => {
                text.push_str(&t.unescape().unwrap_or_default());
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"nav" => in_toc_nav = false,
                b"ol" if in_toc_nav => depth = depth.saturating_sub(1),
                b"a" if in_a => {
                    in_a = false;
                    if !href.is_empty() {
                        let h = join_href(nav_dir, &href);
                        flat.push((depth.max(1), TocEntry {
                            title: text.trim().to_string(),
                            anchor: format!("anchor:href:{h}"),
                            children: Vec::new(),
                        }));
                    }
                    href.clear();
                    text.clear();
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

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
    build(&flat, &mut idx, 0)
}

/// NCX（EPUB2）解析：<navMap><navPoint><navLabel><text> + <content src>
fn parse_ncx(xml: &str, ncx_path: &str) -> Vec<TocEntry> {
    let ncx_dir = match ncx_path.rfind('/') {
        Some(i) => &ncx_path[..i],
        None => "",
    };
    let mut flat: Vec<(u8, TocEntry)> = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut depth = 0u8;
    let mut in_label_text = false;
    let mut title = String::new();
    let mut src = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"navPoint" => {
                    depth += 1;
                    title.clear();
                    src.clear();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"playOrder" {
                            let _ = attr;
                        }
                    }
                }
                b"text" if depth > 0 => in_label_text = true,
                b"content" if depth > 0 => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"src" {
                            src = String::from_utf8_lossy(&attr.value).into_owned();
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"content" && depth > 0 {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"src" {
                            src = String::from_utf8_lossy(&attr.value).into_owned();
                        }
                    }
                }
            }
            Ok(Event::Text(t)) if in_label_text => {
                title.push_str(&t.unescape().unwrap_or_default());
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"navPoint" => {
                    if !src.is_empty() {
                        let h = join_href(ncx_dir, &src);
                        flat.push((depth, TocEntry {
                            title: title.trim().to_string(),
                            anchor: format!("anchor:href:{h}"),
                            children: Vec::new(),
                        }));
                    }
                    depth = depth.saturating_sub(1);
                }
                b"text" => in_label_text = false,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

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
    build(&flat, &mut idx, 0)
}

/// XHTML 全文文本提取（供全文索引；去标签、保留脚本/样式剔除）
pub fn xhtml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut skip_depth = 0i32;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if e.name().as_ref() == b"style" || e.name().as_ref() == b"script" {
                    skip_depth += 1;
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"style" || e.name().as_ref() == b"script" {
                    skip_depth -= 1;
                }
            }
            Ok(Event::Text(t)) if skip_depth == 0 => {
                out.push_str(&t.unescape().unwrap_or_default());
                out.push(' ');
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// epub 解析入口
pub fn parse_epub(name: &str, bytes: &[u8]) -> Option<EpubBook> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let (opf_path, opf_dir) = locate_opf(&mut archive)?;
    let opf_xml = {
        let mut opf_file = archive.by_name(&opf_path).ok()?;
        let mut xml = String::new();
        opf_file.read_to_string(&mut xml).ok()?;
        xml
    };
    let opf = parse_opf(&opf_xml);

    // 封面：cover-item（manifest）字节
    let mut cover = None;
    if let Some(cover_id) = &opf.cover_item_id {
        if let Some(item) = opf.manifest.iter().find(|i| &i.id == cover_id) {
            let href = join_href(&opf_dir, &item.href);
            if let Ok(mut file) = archive.by_name(&href) {
                let mut buf = Vec::new();
                if file.read_to_end(&mut buf).is_ok() {
                    cover = Some(buf);
                }
            }
        }
    }

    // 目录：nav（EPUB3）优先，NCX 回退
    let nav_item = opf
        .manifest
        .iter()
        .find(|i| i.properties.contains("nav"));
    let ncx_item = opf.manifest.iter().find(|i| i.media_type == "application/x-dtbncx+xml");
    let mut toc = Vec::new();
    if let Some(nav) = nav_item {
        let href = join_href(&opf_dir, &nav.href);
        if let Ok(mut file) = archive.by_name(&href) {
            let mut xml = String::new();
            if file.read_to_string(&mut xml).is_ok() {
                toc = parse_nav(&xml, &href);
            }
        }
    }
    if toc.is_empty() {
        if let Some(ncx) = ncx_item {
            let href = join_href(&opf_dir, &ncx.href);
            if let Ok(mut file) = archive.by_name(&href) {
                let mut xml = String::new();
                if file.read_to_string(&mut xml).is_ok() {
                    toc = parse_ncx(&xml, &href);
                }
            }
        }
    }

    // spine 文档：正文（全文文本）与归一化 HTML 素材
    let mut spine_docs = Vec::new();
    let mut fulltext = String::new();
    for idref in &opf.spine {
        if let Some(item) = opf.manifest.iter().find(|i| &i.id == idref) {
            let href = join_href(&opf_dir, &item.href);
            if let Ok(mut file) = archive.by_name(&href) {
                let mut xml = String::new();
                if file.read_to_string(&mut xml).is_ok() {
                    fulltext.push_str(&xhtml_text(&xml));
                    spine_docs.push(href);
                }
            }
        }
    }

    let m = opf.metadata;
    let isbn = m
        .identifier
        .strip_prefix("urn:isbn:")
        .unwrap_or(&m.identifier)
        .to_string();
    let book = ParsedBook {
        title: if m.title.is_empty() { name.to_string() } else { m.title },
        authors: m.creators,
        publisher: m.publisher,
        pubdate: m.pubdate,
        isbn,
        language: m.language,
        series: m.series,
        series_index: m.series_index,
        description: m.description,
        cover,
        toc,
        fulltext: Some(fulltext),
    };
    Some(EpubBook { book, spine_docs, opf_dir })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 程序化构造最小合法 epub（EPUB3 nav + OPF metadata + spine 两章 + 封面）
    fn build_epub() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            let mimetype: &[u8] = b"application/epub+zip";
            zip.start_file("mimetype", opts).unwrap();
            zip.write_all(mimetype).unwrap();

            let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(container.as_bytes()).unwrap();

            let cover_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
            zip.start_file("OEBPS/cover.png", opts).unwrap();
            zip.write_all(cover_bytes).unwrap();

            let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>三体</dc:title>
    <dc:creator>刘慈欣</dc:creator>
    <dc:publisher>重庆出版社</dc:publisher>
    <dc:date>2008-01-01</dc:date>
    <dc:identifier>urn:isbn:9787536692930</dc:identifier>
    <dc:language>zh</dc:language>
    <dc:description>文化大革命之际…</dc:description>
    <meta name="calibre:series" content="地球往事"/>
    <meta name="calibre:series_index" content="1"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="cover" href="cover.png" media-type="image/png" properties="cover-image"/>
    <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
</package>"#;
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(opf.as_bytes()).unwrap();

            let nav = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<body><nav epub:type="toc"><ol>
<li><a href="ch1.xhtml">第一章 科学边界</a></li>
<li><a href="text/ch2.xhtml">第二章 台球</a></li>
</ol></nav></body></html>"#;
            zip.start_file("OEBPS/nav.xhtml", opts).unwrap();
            zip.write_all(nav.as_bytes()).unwrap();

            let ch1 = r#"<?xml version="1.0"?><html><head><title>t</title></head><body><p>黑暗森林法则的秘密</p></body></html>"#;
            zip.start_file("OEBPS/ch1.xhtml", opts).unwrap();
            zip.write_all(ch1.as_bytes()).unwrap();

            let ch2 = r#"<?xml version="1.0"?><html><body><p>台球与物理学</p></body></html>"#;
            zip.start_file("OEBPS/text/ch2.xhtml", opts).unwrap();
            zip.write_all(ch2.as_bytes()).unwrap();

            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn join_href_normalizes_relative_segments() {
        assert_eq!(join_href("OEBPS", "images/pic.png"), "OEBPS/images/pic.png");
        // 子目录上行引用折叠为规范 zip 路径
        assert_eq!(join_href("OEBPS/text", "../images/pic.png"), "OEBPS/images/pic.png");
        assert_eq!(join_href("OEBPS", "./x/../y.xhtml"), "OEBPS/y.xhtml");
        // 绝对路径与根目录
        assert_eq!(join_href("OEBPS", "/abs/img.png"), "abs/img.png");
        assert_eq!(join_href("", "ch1.xhtml"), "ch1.xhtml");
        // 上行越界不逃出根
        assert_eq!(join_href("OEBPS", "../../x.png"), "x.png");
    }

    #[test]
    fn full_epub_parse() {
        let bytes = build_epub();
        let epub = parse_epub("三体", &bytes).expect("epub 解析失败");
        let b = epub.book;
        assert_eq!(b.title, "三体");
        assert_eq!(b.authors, vec!["刘慈欣".to_string()]);
        assert_eq!(b.publisher, "重庆出版社");
        assert_eq!(b.pubdate, "2008-01-01");
        assert_eq!(b.isbn, "9787536692930");
        assert_eq!(b.language, "zh");
        assert_eq!(b.series, "地球往事");
        assert_eq!(b.series_index, 1.0);
        // 封面（cover-image 属性）
        assert_eq!(&b.cover.unwrap()[..4], &[0x89, 0x50, 0x4E, 0x47]);
        // 目录（nav 优先，相对 OPF 目录解析）
        assert_eq!(b.toc.len(), 2);
        assert_eq!(b.toc[0].title, "第一章 科学边界");
        assert_eq!(b.toc[0].anchor, "anchor:href:OEBPS/ch1.xhtml");
        assert_eq!(b.toc[1].anchor, "anchor:href:OEBPS/text/ch2.xhtml");
        // 正文全文（spine 顺序拼接）
        let fulltext = b.fulltext.unwrap();
        assert!(fulltext.contains("黑暗森林法则"));
        assert!(fulltext.contains("台球与物理学"));
        // spine 文档与 OPF 相对目录
        assert_eq!(epub.spine_docs, vec!["OEBPS/ch1.xhtml".to_string(), "OEBPS/text/ch2.xhtml".to_string()]);
        assert_eq!(epub.opf_dir, "OEBPS");
    }

    #[test]
    fn xhtml_text_strips_style_script() {
        let xml = r#"<html><head><style>body{}</style></head><body><p>正文</p><script>var x = 1;</script></body></html>"#;
        let text = xhtml_text(xml);
        assert!(text.contains("正文"));
        assert!(!text.contains("var x"));
    }
}
