//! EPUB 元数据回写：改写 OPF metadata（dc:* 覆盖式更新 + EPUB3 标准系列字段）与封面条目。
//! 结构选择：不整体重建 OPF，而是流式变换——受管元素（dc:title/creator/publisher/description/subject、
//! 系列与封面相关 meta）整块吞掉后在 metadata/manifest 开始处统一注入，
//! 其余元素（日期/标识符/语言/refines/手稿结构）逐事件原样保留，最大限度不破坏外部制作器数据。
//! zip 重写遵循 OCF：mimetype 必须是首条目且 Stored 不压缩；未涉及条目字节原样保留。
//!
//! 字段映射（库 → OPF）：title→dc:title、authors→dc:creator×N、publisher→dc:publisher、
//! description→dc:description、tags+categories→dc:subject×N（合并去重）、
//! series→belongs-to-collection + collection-type=series + group-position（EPUB 3 规范 vocab，
//! 仅 version="3.x" 包写入；EPUB 2 无标准系列字段，不写）。
//! calibre:series* 私有约定 meta 无论系列是否非空一律移除——残留会在重新导入时经
//! 解析端 calibre 优先读到陈旧值（旧系列「复活」）；系列清空时同步移除自写的
//! sumi-series 系列 meta，使文件与库状态一致。
//!
//! 封面三态（见 EmbedCover）：Embed 替换既有封面条目（非 .png 条目改名同目录同 stem
//! 的 .png，撞名加数字后缀）或新增 sumi_cover.png（撞名探号）；Remove 移除 OPF 封面
//! 引用（meta name=cover 与 manifest cover-image 标记），图片文件条目保留包内
//! （可能被封面页 xhtml 引用，删文件会破坏内容）；Keep 不动既有封面。

use super::EmbedCover;
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

/// OPF 变换的封面上下文（Keep = None；Embed/Remove 各带参数）
struct CoverCtx {
    /// Remove 模式：剥 manifest item 的 cover-image 标记（Embed 模式为 false）
    strip: bool,
    /// Embed：封面目标 manifest item id（替换既有条目）
    target_id: Option<String>,
    /// Embed：改名后的 href 属性值（None = 原 href 不变）
    new_href: Option<String>,
    /// Embed：新增封面 item 时的 href（Some = 包内无封面声明，manifest 处注入新 item）
    new_item_href: Option<String>,
    /// Embed：注入 meta name=cover 的 content 值（指向 manifest id）
    meta_ref: String,
}

/// 回写 EPUB 元数据。cover 为三态封面方案（Embed 字节为 PNG）。
pub(crate) fn write_epub_metadata(abs_path: &str, item: &ItemCore, cover: &EmbedCover) -> Result<(), String> {
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

    // href → zip 条目路径（与解析端同口径：percent 解码 + 归一化）
    let zip_of = |href: &str| join_href(&opf_dir, href);
    // 包内既有条目名集合（封面改名 / 新增条目的撞名探测）
    let taken: std::collections::HashSet<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    let cover_ctx: Option<CoverCtx> = match cover {
        EmbedCover::Keep => None,
        EmbedCover::Remove => Some(CoverCtx {
            strip: true,
            target_id: None,
            new_href: None,
            new_item_href: None,
            meta_ref: String::new(),
        }),
        EmbedCover::Embed(_) => match opf.cover_item_id.clone() {
            Some(id) => {
                let old_href = opf
                    .manifest
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.href.clone())
                    .unwrap_or_else(|| COVER_HREF.to_string());
                // 非 .png 封面条目改名（同目录同 stem）；候选与包内既有条目撞名时加数字后缀
                let new_href = png_href(&old_href).map(|cand| {
                    let dir = cand.rsplit_once('/').map(|(d, _)| d);
                    let stem = cand
                        .rsplit('/')
                        .next()
                        .and_then(|f| f.rsplit_once('.'))
                        .map(|(s, _)| s)
                        .unwrap_or("cover");
                    probe_free_name(dir, stem, |name| taken.contains(&zip_of(name)))
                });
                Some(CoverCtx {
                    strip: false,
                    target_id: Some(id.clone()),
                    new_href,
                    new_item_href: None,
                    meta_ref: id,
                })
            }
            None => {
                // 无封面声明：新增条目（sumi_cover.png 已存在 → sumi_cover2.png… 探号到不撞）
                let href = probe_free_name(None, "sumi_cover", |name| taken.contains(&zip_of(name)));
                Some(CoverCtx {
                    strip: false,
                    target_id: None,
                    new_href: None,
                    new_item_href: Some(href),
                    meta_ref: COVER_ITEM_ID.to_string(),
                })
            }
        },
    };

    // zip 层写入方案：(旧条目, 新条目) —— 原位替换两者同名；改名/新增时旧跳过、新循环后写入
    let cover_entry: Option<(Option<String>, String)> = match (&cover_ctx, cover) {
        (Some(ctx), EmbedCover::Embed(_)) if !ctx.strip => {
            if let Some(id) = &ctx.target_id {
                let old = opf
                    .manifest
                    .iter()
                    .find(|i| &i.id == id)
                    .map(|i| zip_of(&i.href))
                    .unwrap_or_else(|| zip_of(COVER_HREF));
                let new = ctx.new_href.as_ref().map(|h| zip_of(h)).unwrap_or_else(|| old.clone());
                Some((Some(old), new))
            } else {
                let href = ctx.new_item_href.as_deref().unwrap_or(COVER_HREF);
                Some((None, zip_of(href)))
            }
        }
        _ => None,
    };
    let png_bytes = match cover {
        EmbedCover::Embed(b) => b.as_slice(),
        _ => &[] as &[u8],
    };

    let new_opf = rewrite_opf(&opf_xml, &opf, item, cover_ctx.as_ref())?;

    // 重写 zip：mimetype 首条目 Stored；其余 deflate；OPF 换新字节；封面条目按方案处理
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
            if cover_entry.as_ref().and_then(|(o, _)| o.as_deref()) == Some(name.as_str()) {
                // 原位替换：同名写新字节；改名：旧条目跳过（新名循环后另写）
                if cover_entry.as_ref().map(|(_, n)| n.as_str()) == Some(name.as_str()) {
                    w.start_file(&name, deflated).map_err(zip_err)?;
                    std::io::Write::write_all(&mut w, png_bytes).map_err(zip_err)?;
                }
                continue;
            }
            if f.is_dir() {
                w.add_directory(&name, deflated).map_err(zip_err)?;
                continue;
            }
            w.start_file(&name, deflated).map_err(zip_err)?;
            std::io::copy(&mut f, &mut w).map_err(zip_err)?;
        }
        // 改名 / 新增条目：旧条目路径不等于新条目路径时在此写入
        if let Some((old, new)) = &cover_entry {
            if old.as_deref() != Some(new.as_str()) {
                w.start_file(new, deflated).map_err(zip_err)?;
                std::io::Write::write_all(&mut w, png_bytes).map_err(zip_err)?;
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
fn rewrite_opf(opf_xml: &str, _opf: &crate::core::parser::epub::Opf, item: &ItemCore, cover_ctx: Option<&CoverCtx>) -> Result<Vec<u8>, String> {
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
    // Embed 模式（注入 meta name=cover / 改写目标条目）；Remove 模式只刻 strip
    let embed_cover = matches!(cover_ctx, Some(c) if !c.strip);
    let cover_meta_ref = cover_ctx.map(|c| c.meta_ref.clone()).unwrap_or_default();

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
                            for inj in inject_metadata(item, epub3, embed_cover, &cover_meta_ref) {
                                writer.write_event(inj).map_err(io_err)?;
                            }
                        }
                        "manifest" => {
                            section = 2;
                            writer.write_event(ev).map_err(io_err)?;
                            if let Some(href) = cover_ctx.and_then(|c| c.new_item_href.as_deref()) {
                                writer.write_event(new_cover_item(href)).map_err(io_err)?;
                            }
                        }
                        _ => {
                            if section == 1 && skip_depth == 0 && managed_dc(&name) {
                                skip_depth = 1;
                            } else if section == 1 && skip_depth == 0 && managed_meta(e, cover_ctx.is_some()) {
                                skip_depth = 1;
                            } else if section == 2 && skip_depth == 0 {
                                if let Some(ctx) = cover_ctx {
                                    let replaced = if ctx.strip {
                                        is_marked_cover_item(e).then(|| Event::Start(strip_cover_item(e)))
                                    } else {
                                        is_cover_item(e, &ctx.target_id)
                                            .then(|| Event::Start(rebuild_cover_item(e, ctx.new_href.as_deref())))
                                    };
                                    match replaced {
                                        Some(out) => writer.write_event(out).map_err(io_err)?,
                                        None => writer.write_event(ev).map_err(io_err)?,
                                    }
                                } else if skip_depth > 0 {
                                    skip_depth += 1;
                                } else {
                                    writer.write_event(ev).map_err(io_err)?;
                                }
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
                    let managed =
                        section == 1 && skip_depth == 0 && (managed_dc(&name) || managed_meta(e, cover_ctx.is_some()));
                    let cover_item: Option<Event<'static>> = if section == 2 && skip_depth == 0 {
                        match cover_ctx {
                            Some(ctx) if ctx.strip => {
                                is_marked_cover_item(e).then(|| Event::Empty(strip_cover_item(e)))
                            }
                            Some(ctx) => is_cover_item(e, &ctx.target_id)
                                .then(|| Event::Empty(rebuild_cover_item(e, ctx.new_href.as_deref()))),
                            None => None,
                        }
                    } else {
                        None
                    };
                    if managed {
                        // 自闭合受管元素：吞掉即可，无子深度
                    } else if let Some(out) = cover_item {
                        writer.write_event(out).map_err(io_err)?;
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
/// calibre:series*（私有约定，无论 EPUB2/3、系列是否非空一律移除——残留会在重新导入时
/// 经解析端 calibre 优先复活旧系列）、封面 meta（Embed 重指目标条目 / Remove 剥除）
fn managed_meta(e: &BytesStart, cover_touched: bool) -> bool {
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
    if name == "calibre:series" || name == "calibre:series_index" {
        return true;
    }
    cover_touched && name == "cover"
}

/// 封面替换目标命中判断
fn is_cover_item(e: &BytesStart, cover_target: &Option<String>) -> bool {
    let Some(target) = cover_target else { return false };
    if local_name(e.name().as_ref()) != "item" {
        return false;
    }
    e.attributes().flatten().any(|a| a.key.as_ref() == b"id" && a.value.as_ref() == target.as_bytes())
}

/// Remove 模式命中判断：manifest item 的 properties 含 cover-image 标记
fn is_marked_cover_item(e: &BytesStart) -> bool {
    local_name(e.name().as_ref()) == "item"
        && e.attributes().flatten().any(|a| {
            a.key.as_ref() == b"properties"
                && String::from_utf8_lossy(&a.value).split_whitespace().any(|p| p == "cover-image")
        })
}

/// Remove 模式的封面条目改写：剥掉 cover-image 标记，其余属性（含 href/media-type）原样保留；
/// 图片文件条目本身留在包内（可能被封面页 xhtml 引用）
fn strip_cover_item(e: &BytesStart) -> BytesStart<'static> {
    let mut out = BytesStart::new("item");
    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref().to_vec();
        if key == b"properties" {
            let owned = String::from_utf8_lossy(&attr.value).into_owned();
            let kept: Vec<&str> = owned.split_whitespace().filter(|p| *p != "cover-image").collect();
            if !kept.is_empty() {
                out.push_attribute(("properties", kept.join(" ").as_str()));
            }
            continue;
        }
        out.push_attribute((
            std::str::from_utf8(&key).unwrap_or(""),
            std::str::from_utf8(&attr.value).unwrap_or(""),
        ));
    }
    out
}

/// 封面 manifest item 原位改写：保留 id 与其余属性，media-type 改 image/png，
/// properties 并入 cover-image；href 可选改名（替换非 .png 条目时同步新扩展名）
fn rebuild_cover_item(e: &BytesStart, new_href: Option<&str>) -> BytesStart<'static> {
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
    if let Some(h) = new_href {
        href = h.to_string();
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

/// href 文件名部分换 .png 扩展名（目录部分原样保留）；已 .png 结尾返回 None = 无需改名
fn png_href(href: &str) -> Option<String> {
    let (dir, file) = match href.rsplit_once('/') {
        Some((d, f)) => (Some(d), f),
        None => (None, href),
    };
    if file.to_ascii_lowercase().ends_with(".png") {
        return None;
    }
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    let new_file = format!("{stem}.png");
    Some(match dir {
        Some(d) => format!("{d}/{new_file}"),
        None => new_file,
    })
}

/// 目录内探测不撞名的文件名（stem.png → stem2.png → …）；is_taken 收全路径判定，
/// 返回带目录前缀的相对 href（新增封面条目撞名与封面改名撞名共用）
fn probe_free_name(dir: Option<&str>, stem: &str, is_taken: impl Fn(&str) -> bool) -> String {
    let mut i = 1u32;
    loop {
        let file = if i == 1 { format!("{stem}.png") } else { format!("{stem}{i}.png") };
        let full = match dir {
            Some(d) => format!("{d}/{file}"),
            None => file,
        };
        if !is_taken(&full) {
            return full;
        }
        i += 1;
    }
}

/// 新增封面条目的 manifest item（EPUB2/3 双兼容的 properties 写法；href 为探号后的最终值）
fn new_cover_item(href: &str) -> Event<'static> {
    Event::Empty(
        BytesStart::new("item").with_attributes([
            ("id", COVER_ITEM_ID),
            ("href", href),
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

    /// EPUB2 包：无标准系列字段不写系列，calibre 私有 meta 同口径移除（残留会复活旧系列）
    #[test]
    fn epub2_series_not_written_and_calibre_removed() {
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
        // 清空系列：文件内 calibre 残留同步擦除（EPUB2/3 同口径）
        item.series = "".into();
        item.series_index = 0.0;
        write_epub_metadata(abs.to_str().unwrap(), &item, &EmbedCover::Keep).unwrap();

        let out = std::fs::read(&abs).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&out[..])).unwrap();
        let mut opf = String::new();
        archive.by_name("content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(opf.contains("V2 新标题"));
        assert!(!opf.contains("belongs-to-collection"), "EPUB2 包不写系列");
        assert!(!opf.contains("calibre:series"), "EPUB2 包的 calibre 系列 meta 也应移除");
        let parsed = crate::core::parser::epub::parse_epub("x", &out).unwrap();
        assert_eq!(parsed.book.series, "");
    }

    /// 清空系列：EPUB3 的 sumi-series 注入元素与 calibre 私有 meta 全部消失，解析端读不到系列
    #[test]
    fn clear_series_removes_all_series_meta() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-clear-series-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("clear.epub");
        std::fs::write(&abs, build_epub3_fixture()).unwrap();

        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "清系列".into();
        item.series = "".into();
        item.series_index = 0.0;
        write_epub_metadata(abs.to_str().unwrap(), &item, &EmbedCover::Keep).unwrap();

        let out = std::fs::read(&abs).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&out[..])).unwrap();
        let mut opf = String::new();
        archive.by_name("OEBPS/content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(!opf.contains("belongs-to-collection"), "sumi-series 应被移除: {opf}");
        assert!(!opf.contains("sumi-series"));
        assert!(!opf.contains("calibre:series"), "calibre 残留会复活旧系列");
        let parsed = crate::core::parser::epub::parse_epub("x", &out).unwrap();
        assert_eq!(parsed.book.series, "");
    }

    /// 封面 Remove：meta name=cover 与 cover-image 标记消失，图片条目保留包内，解析端读不到封面
    #[test]
    fn remove_cover_strips_references() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-remove-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("rm.epub");
        std::fs::write(&abs, build_epub3_fixture()).unwrap();

        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "删封面".into();
        write_epub_metadata(abs.to_str().unwrap(), &item, &EmbedCover::Remove).unwrap();

        let out = std::fs::read(&abs).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&out[..])).unwrap();
        let mut opf = String::new();
        archive.by_name("OEBPS/content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(!opf.contains("name=\"cover\""), "meta name=cover 应移除: {opf}");
        assert!(!opf.contains("cover-image"), "cover-image 标记应剥除");
        assert!(opf.contains("<item id=\"cover\""), "图片条目本身保留（封面页可能引用）");
        assert!(archive.by_name("OEBPS/cover.png").is_ok(), "图片文件保留包内");
        let parsed = crate::core::parser::epub::parse_epub("x", &out).unwrap();
        assert!(parsed.book.cover.is_none(), "解析端不再读到封面");
    }

    /// 替换非 .png 封面条目：改名同目录同 stem 的 .png（manifest href 同步）；
    /// 候选名与包内既有条目撞名时加数字后缀
    #[test]
    fn cover_rename_png_with_collision_suffix() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-rename-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("rename.epub");
        std::fs::write(&abs, build_epub3_fixture_cover_jpg_with_collision()).unwrap();

        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "改名".into();
        write_epub_metadata(abs.to_str().unwrap(), &item, &EmbedCover::Embed(vec![0x89, 0x50, 0x4E, 0x47, 9])).unwrap();

        let out = std::fs::read(&abs).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&out[..])).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        assert!(!names.iter().any(|n| n == "OEBPS/cover.jpg"), "旧条目不再保留: {names:?}");
        assert!(names.iter().any(|n| n == "OEBPS/cover2.png"), "撞名探号落到 cover2.png: {names:?}");
        // 撞名的无关条目不被覆盖（保持原字节）
        let unrelated = {
            let mut f = archive.by_name("OEBPS/cover.png").unwrap();
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut b).unwrap();
            b
        };
        assert_eq!(unrelated, b"unrelated".to_vec(), "撞名条目不被覆盖");
        let mut opf = String::new();
        archive.by_name("OEBPS/content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(opf.contains("href=\"cover2.png\""), "manifest href 同步改名: {opf}");
        assert!(!opf.contains("cover.jpg"));
        let parsed = crate::core::parser::epub::parse_epub("x", &out).unwrap();
        assert_eq!(parsed.book.cover.unwrap(), vec![0x89, 0x50, 0x4E, 0x47, 9]);
    }

    /// 子目录封面改名：href 为 images/cover.jpg 时改名应保留目录前缀（images/cover.png），不搬到 OPF 根目录
    #[test]
    fn cover_rename_keeps_subdir_prefix() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-subdir-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("sub.epub");
        std::fs::write(&abs, build_epub3_fixture_cover_subdir()).unwrap();

        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "子目录".into();
        write_epub_metadata(abs.to_str().unwrap(), &item, &EmbedCover::Embed(vec![0x89, 0x50, 0x4E, 0x47, 3])).unwrap();

        let out = std::fs::read(&abs).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&out[..])).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        assert!(!names.iter().any(|n| n == "OEBPS/images/cover.jpg"), "旧条目不再保留: {names:?}");
        assert!(names.iter().any(|n| n == "OEBPS/images/cover.png"), "改名保留目录前缀: {names:?}");
        assert!(!names.iter().any(|n| n == "OEBPS/cover.png"), "封面不应搬到 OPF 根目录: {names:?}");
        let mut opf = String::new();
        archive.by_name("OEBPS/content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(opf.contains("href=\"images/cover.png\""), "manifest href 同步改名: {opf}");
        let parsed = crate::core::parser::epub::parse_epub("x", &out).unwrap();
        assert_eq!(parsed.book.cover.unwrap(), vec![0x89, 0x50, 0x4E, 0x47, 3]);
    }

    /// 无封面声明的新增：包内已有无关 sumi_cover.png 条目时探号 sumi_cover2.png，不覆盖既有条目
    #[test]
    fn new_cover_entry_probes_free_name() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-new-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("new.epub");
        std::fs::write(&abs, build_epub3_fixture_no_cover_with_stray()).unwrap();

        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "新增".into();
        write_epub_metadata(abs.to_str().unwrap(), &item, &EmbedCover::Embed(vec![0x89, 0x50, 0x4E, 0x47, 7])).unwrap();

        let out = std::fs::read(&abs).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&out[..])).unwrap();
        // 既有无关条目字节不变（不被覆盖为封面）
        let stray = {
            let mut f = archive.by_name("OEBPS/sumi_cover.png").unwrap();
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut b).unwrap();
            b
        };
        assert_eq!(stray, b"stray".to_vec(), "既有同名条目不被覆盖");
        assert!(archive.by_name("OEBPS/sumi_cover2.png").is_ok(), "封面探号到 sumi_cover2.png");
        let mut opf = String::new();
        archive.by_name("OEBPS/content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(opf.contains("href=\"sumi_cover2.png\""), "manifest 指向探号后的条目: {opf}");
        assert!(!opf.contains("href=\"sumi_cover.png\""));
    }

    /// EPUB3 fixture 变体：封面为 cover.jpg（JPEG media-type）且包内另有无关 cover.png 条目
    /// （改名候选 cover.png 被占 → 探号 cover2.png；同时验证旧条目不再保留）
    fn build_epub3_fixture_cover_jpg_with_collision() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("mimetype", opts).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(r#"<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.as_bytes()).unwrap();
            zip.start_file("OEBPS/cover.jpg", opts).unwrap();
            zip.write_all(b"jpeg-bytes").unwrap();
            zip.start_file("OEBPS/cover.png", opts).unwrap();
            zip.write_all(b"unrelated").unwrap();
            let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>旧标题</dc:title></metadata>
  <manifest>
    <item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>
    <item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(opf.as_bytes()).unwrap();
            zip.start_file("OEBPS/c1.xhtml", opts).unwrap();
            zip.write_all(r#"<html><body><p>t</p></body></html>"#.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    /// EPUB3 fixture 变体：封面在子目录（images/cover.jpg），检验改名保留目录前缀
    fn build_epub3_fixture_cover_subdir() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("mimetype", opts).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(r#"<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.as_bytes()).unwrap();
            zip.start_file("OEBPS/images/cover.jpg", opts).unwrap();
            zip.write_all(b"jpeg-bytes").unwrap();
            let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>旧标题</dc:title></metadata>
  <manifest>
    <item id="cover" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
    <item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(opf.as_bytes()).unwrap();
            zip.start_file("OEBPS/c1.xhtml", opts).unwrap();
            zip.write_all(r#"<html><body><p>t</p></body></html>"#.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    /// EPUB3 fixture 变体：无封面声明，包内另有无关 sumi_cover.png 条目（新增封面探号用）
    fn build_epub3_fixture_no_cover_with_stray() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("mimetype", opts).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();
            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(r#"<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.as_bytes()).unwrap();
            zip.start_file("OEBPS/sumi_cover.png", opts).unwrap();
            zip.write_all(b"stray").unwrap();
            let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>旧标题</dc:title></metadata>
  <manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;
            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(opf.as_bytes()).unwrap();
            zip.start_file("OEBPS/c1.xhtml", opts).unwrap();
            zip.write_all(r#"<html><body><p>t</p></body></html>"#.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    /// 坏 zip 报错不 panic
    #[test]
    fn corrupt_epub_returns_err() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("bad.epub");
        std::fs::write(&abs, b"not a zip").unwrap();
        let item = crate::core::item::ItemCore::new("h", vec![], 0);
        assert!(write_epub_metadata(abs.to_str().unwrap(), &item, &EmbedCover::Keep).is_err());
    }
}
