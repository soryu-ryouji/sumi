//! EPUB 元数据回写：改写 OPF metadata（dc:* 覆盖式更新 + EPUB3 标准系列字段）与封面条目。
//! 结构选择：不整体重建 OPF，而是流式变换——受管元素（dc:title/creator/publisher/description/subject、
//! 自写的系列 meta、封面 meta）整块吞掉后在 metadata/manifest 开始处统一注入，
//! 其余元素（日期/标识符/语言/refines/手稿结构）逐事件原样保留，最大限度不破坏外部制作器数据。
//! zip 重写遵循 OCF：mimetype 必须是首条目且 Stored 不压缩；未涉及条目字节原样保留。
//!
//! 字段映射（库 → OPF）：title→dc:title、authors→dc:creator×N、publisher→dc:publisher、
//! description→dc:description、tags+categories→dc:subject×N（合并去重）、
//! series→belongs-to-collection + collection-type=series + group-position（EPUB 3 规范 vocab，
//! 仅 version="3.x" 包写入；EPUB 2 无标准系列字段，不写）；系列非空时同时移除 calibre:series*
//! 旧约定 meta（解析端 calibre 优先，不移除会读到陈旧值），系列为空时不动任何既有 meta。

use crate::core::item::ItemCore;
use crate::core::parser::epub::{join_href, locate_opf, parse_opf};
use quick_xml::escape::escape;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
/// 自写系列 meta 的 id（再次回写时按它识别去重；外部集合 meta 不受影响）
const SERIES_ID: &str = "sumi-series";
/// 新增封面条目的 manifest id 与 href（相对 OPF 目录）
const COVER_ITEM_ID: &str = "sumi-cover";
const COVER_HREF: &str = "sumi_cover.png";

/// 回写 EPUB 元数据。cover_png 有值时替换/新增封面条目（字节为 PNG）；None 时封面原样保留。
pub fn write_epub_metadata(abs_path: &str, item: &ItemCore, cover_png: Option<&[u8]>) -> Result<(), String> {
    let bytes = std::fs::read(abs_path).map_err(|e| format!("读取失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))
        .map_err(|e| format!("zip 打开失败: {e}"))?;
    let (opf_path, opf_dir) = locate_opf(&mut archive).ok_or("container.xml 未定位到 OPF")?;
    let opf_xml = {
        let mut f = archive.by_name(&opf_path).map_err(|e| format!("OPF 读取失败: {e}"))?;
        let mut s = String::new();
        std::io::Read::read_to_string(&mut f, &mut s).map_err(|e| format!("OPF 读取失败: {e}"))?;
        s
    };
    let opf = parse_opf(&opf_xml);

    // 封面方案：已有声明（EPUB3 properties / item name / meta content 三源合并结果）→ 替换该条目字节；
    // 无声明 → 新增 sumi_cover.png 条目 + manifest item，meta name=cover 与 properties 双写兼容两种版本
    let cover_meta_ref = if cover_png.is_some() {
        opf.cover_item_id.clone().unwrap_or_else(|| COVER_ITEM_ID.to_string())
    } else {
        String::new()
    };
    let cover_is_new = cover_png.is_some() && opf.cover_item_id.is_none();
    let cover_entry = cover_png.map(|_| match &opf.cover_item_id {
        Some(id) => opf
            .manifest
            .iter()
            .find(|i| &i.id == id)
            .map(|i| join_href(&opf_dir, &i.href))
            .unwrap_or_else(|| join_href(&opf_dir, COVER_HREF)),
        None => join_href(&opf_dir, COVER_HREF),
    });

    let new_opf = rewrite_opf(&opf_xml, &opf, item, cover_png.is_some(), &cover_meta_ref, cover_is_new)?;

    // 重写 zip：mimetype 首条目 Stored；其余 deflate；OPF 与封面条目换新字节
    let mut out = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("mimetype", stored).map_err(|e| format!("zip 写入失败: {e}"))?;
        std::io::Write::write_all(&mut w, b"application/epub+zip").map_err(|e| format!("zip 写入失败: {e}"))?;
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).map_err(|e| format!("zip 读取失败: {e}"))?;
            let name = f.name().to_string();
            if name == "mimetype" {
                continue;
            }
            if name == opf_path {
                w.start_file(&name, deflated).map_err(zip_err)?;
                std::io::Write::write_all(&mut w, &new_opf).map_err(zip_err)?;
                continue;
            }
            if !cover_is_new && cover_entry.as_deref() == Some(name.as_str()) {
                w.start_file(&name, deflated).map_err(zip_err)?;
                std::io::Write::write_all(&mut w, cover_png.unwrap_or(&[])).map_err(zip_err)?;
                continue;
            }
            if f.is_dir() {
                w.add_directory(&name, deflated).map_err(zip_err)?;
                continue;
            }
            w.start_file(&name, deflated).map_err(zip_err)?;
            std::io::copy(&mut f, &mut w).map_err(zip_err)?;
        }
        if cover_is_new {
            if let Some(entry) = &cover_entry {
                w.start_file(entry, deflated).map_err(zip_err)?;
                std::io::Write::write_all(&mut w, cover_png.unwrap_or(&[])).map_err(zip_err)?;
            }
        }
        w.finish().map_err(zip_err)?;
    }
    crate::core::config::atomic_write(abs_path, &out).map_err(|e| format!("写入失败: {e}"))?;
    Ok(())
}

fn zip_err(e: impl Into<std::io::Error>) -> String {
    format!("zip 写入失败: {}", e.into())
}

/// OPF 流式改写。受管元素吞掉不写，注入发生在 metadata / manifest 开始标签之后。
fn rewrite_opf(
    opf_xml: &str,
    opf: &crate::core::parser::epub::Opf,
    item: &ItemCore,
    rewrite_cover: bool,
    cover_meta_ref: &str,
    cover_is_new: bool,
) -> Result<Vec<u8>, String> {
    // BOM 剥离（quick-xml 对 &str 输入的 BOM 容忍度不确定，输出统一无 BOM）
    let opf_xml = opf_xml.strip_prefix('\u{feff}').unwrap_or(opf_xml);
    let mut reader = Reader::from_str(opf_xml);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    // section：0 包级 1 metadata 2 manifest（spine/guide 不特判）
    let mut section = 0u8;
    let mut epub3 = false;
    // 子树跳过深度（受管元素整块吞掉；Enter +1 / End -1，归零时对应的 End 也吞）
    let mut skip_depth = 0usize;
    // 封面替换目标 manifest id（仅替换已存在条目时非空）
    let cover_target = if rewrite_cover { opf.cover_item_id.clone() } else { None };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("OPF 解析失败: {e}")),
            Ok(ev) => match &ev {
                Event::Start(e) => {
                    let name = local_name(e.name().as_ref());
                    match name.as_str() {
                        "package" => {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"version" {
                                    epub3 = String::from_utf8_lossy(&attr.value).starts_with('3');
                                }
                            }
                            writer.write_event(ev).map_err(io_err)?;
                        }
                        "metadata" => {
                            section = 1;
                            writer.write_event(ev).map_err(io_err)?;
                            for inj in inject_metadata(item, epub3, rewrite_cover, cover_meta_ref) {
                                writer.write_event(inj).map_err(io_err)?;
                            }
                        }
                        "manifest" => {
                            section = 2;
                            writer.write_event(ev).map_err(io_err)?;
                            if cover_is_new {
                                writer.write_event(new_cover_item()).map_err(io_err)?;
                            }
                        }
                        _ => {
                            if section == 1 && skip_depth == 0 && managed_dc(&name) {
                                skip_depth = 1;
                            } else if section == 1 && skip_depth == 0 && managed_meta(e, item, epub3, rewrite_cover) {
                                skip_depth = 1;
                            } else if section == 2 && skip_depth == 0 && is_cover_item(e, &cover_target) {
                                // 封面条目原位改写：字节已换 PNG，媒体类型同步改；properties 补 cover-image
                                writer.write_event(Event::Start(rebuild_cover_item(e))).map_err(io_err)?;
                            } else if skip_depth > 0 {
                                skip_depth += 1;
                            } else {
                                writer.write_event(ev).map_err(io_err)?;
                            }
                        }
                    }
                }
                Event::Empty(e) => {
                    let name = local_name(e.name().as_ref());
                    let managed = section == 1
                        && skip_depth == 0
                        && (managed_dc(&name) || managed_meta(e, item, epub3, rewrite_cover));
                    let cover_item = section == 2 && skip_depth == 0 && is_cover_item(e, &cover_target);
                    if managed {
                        // 自闭合受管元素：吞掉即可，无子深度
                    } else if cover_item {
                        writer.write_event(Event::Empty(rebuild_cover_item(e))).map_err(io_err)?;
                    } else if skip_depth == 0 {
                        writer.write_event(ev).map_err(io_err)?;
                    }
                }
                Event::End(e) => {
                    let name = local_name(e.name().as_ref());
                    if (name == "metadata" || name == "manifest") && section != 0 {
                        section = 0;
                        writer.write_event(ev).map_err(io_err)?;
                    } else if skip_depth > 0 {
                        skip_depth -= 1;
                        // 归零时这个 End 是受管子树的收口，同样不写
                    } else {
                        writer.write_event(ev).map_err(io_err)?;
                    }
                }
                _ => {
                    if skip_depth == 0 {
                        writer.write_event(ev).map_err(io_err)?;
                    }
                }
            },
        }
        buf.clear();
    }
    Ok(writer.into_inner())
}

fn io_err(e: std::io::Error) -> String {
    format!("OPF 序列化失败: {e}")
}

/// 本地名（剥命名空间前缀）：与解析端同口径
fn local_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).rsplit(':').next().unwrap_or("").to_string()
}

/// 受管 dc 元素（覆盖式重写；dc:title/creator/publisher/description/subject）
fn managed_dc(name: &str) -> bool {
    matches!(name, "title" | "creator" | "publisher" | "description" | "subject")
}

/// 受管 meta 元素：自写的 EPUB3 系列 meta（按 id/refines 识别，外部集合不动）、
/// EPUB3 包且系列非空时的 calibre:series*（解析端 calibre 优先，不移除会读到陈旧值；
/// EPUB2 无标准替代字段，保留不动）、重写封面时的 meta name=cover（注入统一指到目标条目）
fn managed_meta(e: &BytesStart, item: &ItemCore, epub3: bool, rewrite_cover: bool) -> bool {
    let mut name = String::new();
    let mut property = String::new();
    let mut id = String::new();
    let mut refines = String::new();
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            b"name" => name = String::from_utf8_lossy(&attr.value).into_owned(),
            b"property" => property = String::from_utf8_lossy(&attr.value).into_owned(),
            b"id" => id = String::from_utf8_lossy(&attr.value).into_owned(),
            b"refines" => refines = String::from_utf8_lossy(&attr.value).into_owned(),
            _ => {}
        }
    }
    if !property.is_empty() {
        return (property == "belongs-to-collection" && id == SERIES_ID)
            || (refines == format!("#{SERIES_ID}") && matches!(property.as_str(), "collection-type" | "group-position"));
    }
    if epub3 && !item.series.is_empty() && (name == "calibre:series" || name == "calibre:series_index") {
        return true;
    }
    rewrite_cover && name == "cover"
}

/// 封面替换目标命中判断
fn is_cover_item(e: &BytesStart, cover_target: &Option<String>) -> bool {
    let Some(target) = cover_target else { return false };
    if local_name(e.name().as_ref()) != "item" {
        return false;
    }
    e.attributes().flatten().any(|a| a.key.as_ref() == b"id" && a.value.as_ref() == target.as_bytes())
}

/// 封面 manifest item 原位改写：保留 id/href 与其余属性，media-type 改 image/png，
/// properties 并入 cover-image（属性值保持源码转义态原样透传，不做反转义）
fn rebuild_cover_item(e: &BytesStart) -> BytesStart<'static> {
    let mut id = String::new();
    let mut href = String::new();
    let mut properties = String::new();
    let mut others: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref().to_vec();
        let value = attr.value.as_ref().to_vec();
        match key.as_slice() {
            b"id" => id = String::from_utf8_lossy(&value).into_owned(),
            b"href" => href = String::from_utf8_lossy(&value).into_owned(),
            b"media-type" => {}
            b"properties" => properties = String::from_utf8_lossy(&value).into_owned(),
            _ => others.push((key, value)),
        }
    }
    let mut props: Vec<String> = properties.split_whitespace().map(|s| s.to_string()).collect();
    if !props.iter().any(|p| p == "cover-image") {
        props.push("cover-image".into());
    }
    let mut out = BytesStart::new("item");
    out.push_attribute(("id", id.as_str()));
    out.push_attribute(("href", href.as_str()));
    out.push_attribute(("media-type", "image/png"));
    out.push_attribute(("properties", props.join(" ").as_str()));
    for (k, v) in others {
        out.push_attribute((
            std::str::from_utf8(&k).unwrap_or(""),
            std::str::from_utf8(&v).unwrap_or(""),
        ));
    }
    out
}

/// 新增封面条目的 manifest item（EPUB2/3 双兼容的 properties 写法）
fn new_cover_item() -> Event<'static> {
    Event::Empty(
        BytesStart::new("item").with_attributes([
            ("id", COVER_ITEM_ID),
            ("href", COVER_HREF),
            ("media-type", "image/png"),
            ("properties", "cover-image"),
        ]),
    )
}

/// metadata 开始处注入的受管元素序列（dc:* → 封面 meta → EPUB3 系列）
fn inject_metadata(item: &ItemCore, epub3: bool, rewrite_cover: bool, cover_meta_ref: &str) -> Vec<Event<'static>> {
    let mut evs: Vec<Event<'static>> = Vec::new();
    if !item.title.trim().is_empty() {
        evs.extend(dc_elem("title", &item.title));
    }
    for a in &item.authors {
        if !a.trim().is_empty() {
            evs.extend(dc_elem("creator", a));
        }
    }
    if !item.publisher.trim().is_empty() {
        evs.extend(dc_elem("publisher", &item.publisher));
    }
    if !item.description.trim().is_empty() {
        evs.extend(dc_elem("description", &item.description));
    }
    // subject：tags + categories 合并去重保序
    let mut seen = std::collections::HashSet::new();
    for s in item.tags.iter().chain(item.categories.iter()) {
        if !s.trim().is_empty() && seen.insert(s.clone()) {
            evs.extend(dc_elem("subject", s));
        }
    }
    if rewrite_cover {
        // EPUB2 形态声明（EPUB3 阅读器忽略，EPUB2 阅读器依赖）
        let mut meta = BytesStart::new("meta");
        meta.push_attribute(("name", "cover"));
        meta.push_attribute(("content", std::borrow::Cow::Owned(cover_meta_ref.to_string())));
        evs.push(Event::Empty(meta));
    }
    if epub3 && !item.series.trim().is_empty() {
        // EPUB 3 规范 vocab：belongs-to-collection + refines 修饰；EPUB 2 无标准字段不写
        evs.push(Event::Start(
            BytesStart::new("meta").with_attributes([("property", "belongs-to-collection"), ("id", SERIES_ID)]),
        ));
        evs.push(Event::Text(BytesText::from_escaped(escape(item.series.trim()).into_owned())));
        evs.push(Event::End(BytesEnd::new("meta")));
        evs.extend(meta_refines("collection-type", "series"));
        if item.series_index > 0.0 {
            evs.extend(meta_refines("group-position", &format!("{}", item.series_index)));
        }
    }
    evs
}

/// dc 元素事件三元组（内联 xmlns:dc 声明，不依赖外层前缀声明存在）
fn dc_elem(name: &str, value: &str) -> Vec<Event<'static>> {
    vec![
        Event::Start(
            BytesStart::new(format!("dc:{name}")).with_attributes([("xmlns:dc", DC_NS)]),
        ),
        Event::Text(BytesText::from_escaped(escape(value).into_owned())),
        Event::End(BytesEnd::new(format!("dc:{name}"))),
    ]
}

/// refines 修饰 meta（文本体形式）：<meta refines="#sumi-series" property="…">value</meta>
fn meta_refines(property: &str, value: &str) -> Vec<Event<'static>> {
    let mut start = BytesStart::new("meta");
    start.push_attribute(("refines", "#sumi-series"));
    start.push_attribute(("property", std::borrow::Cow::Owned(property.to_string())));
    vec![
        Event::Start(start),
        Event::Text(BytesText::from_escaped(escape(value).into_owned())),
        Event::End(BytesEnd::new("meta")),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// EPUB3 fixture：OPF 带 calibre 系列 meta + cover-image 封面 + 两章正文
    pub fn build_epub3_fixture() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("mimetype", opts).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();
            let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(container.as_bytes()).unwrap();
            zip.start_file("OEBPS/cover.png", opts).unwrap();
            zip.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00]).unwrap();
            let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>旧标题</dc:title>
    <dc:creator>旧作者</dc:creator>
    <dc:publisher>旧出版社</dc:publisher>
    <dc:date>2000-01-01</dc:date>
    <dc:identifier>urn:isbn:0000000000000</dc:identifier>
    <dc:language>zh</dc:language>
    <dc:subject>旧标签</dc:subject>
    <meta name="calibre:series" content="旧系列"/>
    <meta name="calibre:series_index" content="9"/>
    <meta name="cover" content="cover"/>
  </metadata>
  <manifest>
    <item id="cover" href="cover.png" media-type="image/png" properties="cover-image"/>
    <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(opf.as_bytes()).unwrap();
            let ch1 = r#"<?xml version="1.0"?><html><body><p>无关条目正文保持不变</p></body></html>"#;
            zip.start_file("OEBPS/ch1.xhtml", opts).unwrap();
            zip.write_all(ch1.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    /// 回写后断言：dc:* 字段、EPUB3 系列 meta、calibre 移除、封面字节、mimetype 合规
    pub fn assert_opf_fields(bytes: &[u8], item: &crate::core::item::ItemCore) {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();

        // mimetype：首条目、Stored、内容不变（OCF 规范）
        let m = archive.by_index(0).unwrap();
        assert_eq!(m.name(), "mimetype");
        assert_eq!(m.compression(), zip::CompressionMethod::Stored);
        let mut ms = String::new();
        {
            let mut m = m;
            std::io::Read::read_to_string(&mut m, &mut ms).unwrap();
        }
        assert_eq!(ms, "application/epub+zip");

        // 无关条目字节不变
        let ch1_content = {
            let mut ch1 = archive.by_name("OEBPS/ch1.xhtml").unwrap();
            let mut s = String::new();
            ch1.read_to_string(&mut s).unwrap();
            s
        };
        assert!(ch1_content.contains("无关条目正文保持不变"));

        let opf_xml = {
            let mut f = archive.by_name("OEBPS/content.opf").unwrap();
            let mut s = String::new();
            f.read_to_string(&mut s).unwrap();
            s
        };
        // 注入元素带内联 xmlns:dc 声明，断言按收口标签锚定
        assert!(opf_xml.contains(&format!("</dc:title>")) && opf_xml.contains(&format!(">{}</dc:title>", item.title)), "title 未写入: {opf_xml}");
        for a in &item.authors {
            assert!(opf_xml.contains(&format!(">{a}</dc:creator>")), "creator 未写入");
        }
        assert!(opf_xml.contains(&format!(">{}</dc:publisher>", item.publisher)));
        assert!(opf_xml.contains(&format!(">{}</dc:description>", item.description)));
        assert!(opf_xml.contains(">科幻</dc:subject>"));
        assert!(opf_xml.contains(">外国文学</dc:subject>"), "categories 应并入 subject");
        assert!(!opf_xml.contains("旧标签"));
        // 非受管元素存活且值不变（回写只动受管 dc:* 与自写 meta，其余元素逐事件原样保留）
        assert!(opf_xml.contains("<dc:date>2000-01-01</dc:date>"), "非受管 dc:date 应保留");
        assert!(opf_xml.contains("<dc:identifier>urn:isbn:0000000000000</dc:identifier>"), "非受管 dc:identifier 应保留");
        assert!(opf_xml.contains("<dc:language>zh</dc:language>"), "非受管 dc:language 应保留");
        // EPUB3 标准系列 + calibre 旧约定移除
        assert!(opf_xml.contains("property=\"belongs-to-collection\""));
        assert!(opf_xml.contains("<meta refines=\"#sumi-series\" property=\"collection-type\">series</meta>"));
        assert!(opf_xml.contains("<meta refines=\"#sumi-series\" property=\"group-position\">2</meta>"));
        assert!(!opf_xml.contains("calibre:series"), "calibre 系列 meta 应移除");
        // 解析端回读系列生效
        let parsed = crate::core::parser::epub::parse_epub("x", bytes).unwrap();
        assert_eq!(parsed.book.series, item.series);
        assert_eq!(parsed.book.series_index, item.series_index);
        assert_eq!(parsed.book.title, item.title);
        assert_eq!(parsed.book.authors, item.authors);
        // 封面条目替换为新 PNG 字节
        assert_eq!(parsed.book.cover.unwrap(), vec![0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01]);
    }

    /// EPUB2 包：系列不写（无标准字段），calibre meta 原样保留
    #[test]
    fn epub2_series_not_written() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("mimetype", opts).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(r#"<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.as_bytes()).unwrap();
            let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>v2</dc:title>
  <meta name="calibre:series" content="保留"/></metadata>
  <manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;
            zip.start_file("content.opf", opts).unwrap();
            zip.write_all(opf.as_bytes()).unwrap();
            zip.start_file("c1.xhtml", opts).unwrap();
            zip.write_all(r#"<html><body><p>t</p></body></html>"#.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("sumi-embed-epub2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("v2.epub");
        std::fs::write(&abs, buf.into_inner()).unwrap();

        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "V2 新标题".into();
        item.series = "不应写入".into();
        item.series_index = 1.0;
        write_epub_metadata(abs.to_str().unwrap(), &item, None).unwrap();

        let out = std::fs::read(&abs).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&out[..])).unwrap();
        let mut opf = String::new();
        archive.by_name("content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(opf.contains("V2 新标题"));
        assert!(!opf.contains("belongs-to-collection"), "EPUB2 包不写系列");
        assert!(opf.contains("保留"), "EPUB2 的 calibre 系列 meta 原样保留");
    }

    /// 坏 zip 报错不 panic
    #[test]
    fn corrupt_epub_returns_err() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("bad.epub");
        std::fs::write(&abs, b"not a zip").unwrap();
        let item = crate::core::item::ItemCore::new("h", vec![], 0);
        assert!(write_epub_metadata(abs.to_str().unwrap(), &item, None).is_err());
    }
}
