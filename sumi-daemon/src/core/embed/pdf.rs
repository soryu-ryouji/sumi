//! PDF 元数据回写：lopdf 改 trailer /Info 字典（Title/Author/Subject/Keywords）并同步 XMP
//! （Catalog /Metadata 流），全量重写保存。双写原因：规范上 XMP 优先于 Info 字典，
//! 只改 Info 会被 Acrobat 等遵循规范的工具忽略。
//! PDF 无系列/分类独立概念：series 不写；tags+categories 并入 Keywords（逗号连接）。
//! 文本字符串编码：ASCII 走 Literal；含非 ASCII 用 UTF-16BE（带 BOM）Hexadecimal——
//! PDF 1.7 规范 §7.9.2 的 text string 标准 Encoding，兼顾各阅读器兼容。
//! XMP 同步：已有 packet 就地更新受管元素（dc:title/creator/description、pdf:Keywords），
//! 其余内容（PDF/A 声明等）逐事件保留；无 packet 创建最小合规 XMP；空值字段删除。
//! XMP 解析失败视为硬错误（不做 Info-only 的半新半旧写入）。加密/损坏 PDF 直接报错不写。

use crate::core::item::ItemCore;
use lopdf::{Dictionary, Document, Object, Stream, StringFormat};
use quick_xml::escape::escape;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const PDF_NS: &str = "http://ns.adobe.com/pdf/1.3/";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// 回写 PDF 元数据（Info 字典 + XMP）。只管书目四字段：Title/Author/Subject/Keywords。
pub fn write_pdf_metadata(abs_path: &str, item: &ItemCore) -> Result<(), String> {
    let mut doc = Document::load(abs_path).map_err(|e| format!("PDF 打开失败: {e}"))?;
    if doc.is_encrypted() {
        return Err("加密 PDF 不支持写入元数据".into());
    }

    // trailer /Info：引用形态直接取；极少数内联字典搬进独立对象再挂引用；
    // 缺失则新建空字典对象
    let info_id = match doc.trailer.get(b"Info") {
        Ok(Object::Reference(id)) => *id,
        Ok(Object::Dictionary(d)) => {
            let id = doc.add_object(d.clone());
            doc.trailer.set("Info", Object::Reference(id));
            id
        }
        _ => {
            let id = doc.add_object(Dictionary::new());
            doc.trailer.set("Info", Object::Reference(id));
            id
        }
    };
    let dict = doc
        .get_object_mut(info_id)
        .map_err(|e| format!("Info 对象读取失败: {e}"))?
        .as_dict_mut()
        .map_err(|_| "Info 对象不是字典".to_string())?;

    let author = item.authors.join(", ");
    let mut keywords: Vec<&str> = item.tags.iter().map(|s| s.as_str()).collect();
    keywords.extend(item.categories.iter().map(|s| s.as_str()));
    let keywords = keywords.join(", ");

    set_or_remove(dict, b"Title", &item.title);
    set_or_remove(dict, b"Author", &author);
    set_or_remove(dict, b"Subject", &item.description);
    set_or_remove(dict, b"Keywords", &keywords);

    // XMP 与 Info 双写（Info 已改完；XMP 失败则整体失败，不落盘半新半旧状态）
    sync_xmp(&mut doc, item)?;

    let mut buf = Vec::new();
    doc.save_to(&mut buf).map_err(|e| format!("PDF 序列化失败: {e}"))?;
    crate::core::config::atomic_write(abs_path, &buf).map_err(|e| format!("写入失败: {e}"))?;
    Ok(())
}

// ---------- XMP 同步 ----------

/// Catalog /Metadata XMP 流同步：已有 packet 就地更新受管元素，无则创建最小合规 packet。
fn sync_xmp(doc: &mut Document, item: &ItemCore) -> Result<(), String> {
    let catalog_id = match doc.trailer.get(b"Root") {
        Ok(Object::Reference(id)) => *id,
        Ok(Object::Dictionary(d)) => {
            let id = doc.add_object(d.clone());
            doc.trailer.set("Root", Object::Reference(id));
            id
        }
        _ => return Err("PDF 缺少 Catalog".into()),
    };
    // 先取既有 XMP 字节（借用结束后再改 Catalog）
    let existing: Option<Vec<u8>> = {
        let catalog = doc
            .get_object(catalog_id)
            .map_err(|e| format!("Catalog 读取失败: {e}"))?
            .as_dict()
            .map_err(|_| "Catalog 不是字典".to_string())?;
        match catalog.get(b"Metadata") {
            Ok(Object::Reference(id)) => doc
                .get_object(*id)
                .ok()
                .and_then(|o| o.as_stream().ok())
                .map(|s| s.content.clone()),
            Ok(Object::Stream(s)) => Some(s.content.clone()),
            _ => None,
        }
    };
    let bytes = match existing {
        Some(raw) => update_xmp(&String::from_utf8_lossy(&raw), item)?,
        None => build_xmp_packet(item),
    };
    let stream_id = doc.add_object(Stream::new(
        Dictionary::from_iter(vec![
            (b"Type".to_vec(), Object::Name(b"Metadata".to_vec())),
            (b"Subtype".to_vec(), Object::Name(b"XML".to_vec())),
        ]),
        bytes,
    ));
    let catalog = doc
        .get_object_mut(catalog_id)
        .map_err(|e| format!("Catalog 读取失败: {e}"))?
        .as_dict_mut()
        .map_err(|_| "Catalog 不是字典".to_string())?;
    catalog.set("Metadata", Object::Reference(stream_id));
    Ok(())
}

/// 就地更新 XMP：受管元素（dc:title/creator/description、pdf:Keywords）整块吞掉，
/// 新值注入首个受管位置；全无受管元素时注入首个 rdf:Description 收口前（属性属
/// Description 子级，不能直接挂在 RDF 下）；空值字段不注入（= 删除）。
/// 其余内容（xpacket PI、注释、其它命名空间元素）逐事件原样保留。
fn update_xmp(xmp: &str, item: &ItemCore) -> Result<Vec<u8>, String> {
    let xmp = xmp.strip_prefix('\u{feff}').unwrap_or(xmp);
    let mut reader = Reader::from_str(xmp);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    // 受管子树跳过深度（Enter +1 / End -1，归零时对应的 End 也吞）
    let mut skip_depth = 0usize;
    let mut injected = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XMP 解析失败: {e}")),
            Ok(ev) => match &ev {
                Event::Start(e) => {
                    if skip_depth > 0 {
                        // 受管子树内部：吞掉
                        skip_depth += 1;
                    } else if managed_xmp_name(e.name().as_ref()) {
                        // 受管子树吞掉；首个位置注入新值
                        skip_depth = 1;
                        inject_at(&mut writer, item, &mut injected)?;
                    } else {
                        writer.write_event(ev).map_err(io_err)?;
                    }
                }
                Event::Empty(e) => {
                    // 自闭合受管元素：吞掉并注入新值；受管子树内部的 Empty 一并吞掉
                    if skip_depth == 0 && managed_xmp_name(e.name().as_ref()) {
                        inject_at(&mut writer, item, &mut injected)?;
                    } else if skip_depth == 0 {
                        writer.write_event(ev).map_err(io_err)?;
                    }
                }
                Event::End(e) => {
                    let name = local_name(e.name().as_ref());
                    if skip_depth > 0 {
                        skip_depth -= 1;
                        // 归零时这个 End 是受管子树的收口，同样不写
                    } else if !injected && name == "RDF" {
                        // 整包无 Description（空 RDF）：RDF 收口前包一层 Description 注入
                        writer
                            .write_event(Event::Start(
                                BytesStart::new("rdf:Description").with_attributes([("rdf:about", "")]),
                            ))
                            .map_err(io_err)?;
                        inject_at(&mut writer, item, &mut injected)?;
                        writer.write_event(Event::End(BytesEnd::new("rdf:Description"))).map_err(io_err)?;
                        writer.write_event(ev).map_err(io_err)?;
                    } else {
                        if !injected && name == "Description" {
                            // 无受管元素时的注入点：首个 Description 收口前
                            inject_at(&mut writer, item, &mut injected)?;
                        }
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
    format!("XMP 序列化失败: {e}")
}

/// XMP 受管元素判定（前缀 + 本地名）：dc:title / dc:creator / dc:description / pdf:Keywords
fn managed_xmp_name(raw: &[u8]) -> bool {
    match String::from_utf8_lossy(raw).rsplit_once(':') {
        Some(("dc", local)) => matches!(local, "title" | "creator" | "description"),
        Some(("pdf", "Keywords")) => true,
        _ => false,
    }
}

fn local_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).rsplit(':').next().unwrap_or("").to_string()
}

/// 受管字段的注入元素片段（值全空的字段不产元素 = 文件内删除；全空返回空串 = 仅删除）
fn managed_xmp_fragment(item: &ItemCore) -> String {
    let mut s = String::new();
    let title = item.title.trim();
    if !title.is_empty() {
        s.push_str(&format!(
            "<dc:title xmlns:dc=\"{DC_NS}\"><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>",
            escape(title)
        ));
    }
    let authors: Vec<&str> = item.authors.iter().map(|a| a.trim()).filter(|a| !a.is_empty()).collect();
    if !authors.is_empty() {
        s.push_str(&format!("<dc:creator xmlns:dc=\"{DC_NS}\"><rdf:Seq>"));
        for a in authors {
            s.push_str(&format!("<rdf:li>{}</rdf:li>", escape(a)));
        }
        s.push_str("</rdf:Seq></dc:creator>");
    }
    let desc = item.description.trim();
    if !desc.is_empty() {
        s.push_str(&format!(
            "<dc:description xmlns:dc=\"{DC_NS}\"><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>",
            escape(desc)
        ));
    }
    let mut kws: Vec<&str> = item.tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    kws.extend(item.categories.iter().map(|c| c.trim()).filter(|c| !c.is_empty()));
    if !kws.is_empty() {
        s.push_str(&format!(
            "<pdf:Keywords xmlns:pdf=\"{PDF_NS}\">{}</pdf:Keywords>",
            escape(&kws.join(", "))
        ));
    }
    s
}

/// 在注入点写入受管元素片段（幂等：已注入则跳过）
fn inject_at(writer: &mut Writer<Vec<u8>>, item: &ItemCore, injected: &mut bool) -> Result<(), String> {
    if *injected {
        return Ok(());
    }
    let frag = managed_xmp_fragment(item);
    if !frag.is_empty() {
        let mut reader = Reader::from_str(&frag);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) => break,
                Ok(ev) => writer.write_event(ev).map_err(io_err)?,
                Err(e) => return Err(format!("XMP 注入片段解析失败: {e}")),
            }
            buf.clear();
        }
    }
    *injected = true;
    Ok(())
}

/// 无既有 XMP 时创建最小合规 packet（xpacket 头尾 + RDF/Description 骨架）
fn build_xmp_packet(item: &ItemCore) -> Vec<u8> {
    let frag = managed_xmp_fragment(item);
    // 无可写字段时也给合法空 packet：后续回写可走就地更新路径
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"sumi\">\n\
          <rdf:RDF xmlns:rdf=\"{RDF_NS}\">\n\
           <rdf:Description rdf:about=\"\">\n\
        {frag}   </rdf:Description>\n\
          </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n"
    )
    .into_bytes()
}

/// 空值删键（清空语义），非空写入
fn set_or_remove(dict: &mut Dictionary, key: &[u8], value: &str) {
    if value.trim().is_empty() {
        dict.remove(key);
    } else {
        dict.set(key.to_vec(), pdf_text(value.trim()));
    }
}

/// PDF text string：ASCII 走 Literal；非 ASCII 用 UTF-16BE + BOM 的 Hexadecimal
fn pdf_text(s: &str) -> Object {
    if s.is_ascii() {
        Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend(s.encode_utf16().flat_map(|u| u.to_be_bytes()));
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 手工构造的单页 PDF（与 parser::pdf 测试同一形态：合法 xref，无 Info）
    pub const MINIMAL_PDF: &[u8] = &[
        37, 80, 68, 70, 45, 49, 46, 52, 10, 49, 32, 48, 32, 111, 98, 106, 60, 60, 47, 84, 121, 112, 101, 47, 67, 97, 116, 97, 108, 111, 103, 47, 80, 97, 103, 101, 115, 32, 50, 32, 48, 32, 82, 62, 62, 101, 110, 100, 111, 98, 106, 10, 50, 32, 48, 32, 111, 98, 106, 60, 60, 47, 84, 121, 112, 101, 47, 80, 97, 103, 101, 115, 47, 75, 105, 100, 115, 91, 51, 32, 48, 32, 82, 93, 47, 67, 111, 117, 110, 116, 32, 49, 62, 62, 101, 110, 100, 111, 98, 106, 10, 51, 32, 48, 32, 111, 98, 106, 60, 60, 47, 84, 121, 112, 101, 47, 80, 97, 103, 101, 47, 80, 97, 114, 101, 110, 116, 32, 50, 32, 48, 32, 82, 47, 77, 101, 100, 105, 97, 66, 111, 120, 91, 48, 32, 48, 32, 50, 48, 48, 32, 51, 48, 48, 93, 47, 67, 111, 110, 116, 101, 110, 116, 115, 32, 52, 32, 48, 32, 82, 47, 82, 101, 115, 111, 117, 114, 99, 101, 115, 60, 60, 62, 62, 62, 62, 101, 110, 100, 111, 98, 106, 10, 52, 32, 48, 32, 111, 98, 106, 60, 60, 47, 76, 101, 110, 103, 116, 104, 32, 51, 49, 62, 62, 115, 116, 114, 101, 97, 109, 10, 49, 32, 48, 32, 48, 32, 82, 71, 32, 50, 32, 119, 32, 50, 48, 32, 50, 48, 32, 49, 54, 48, 32, 50, 54, 48, 32, 114, 101, 32, 83, 10, 101, 110, 100, 115, 116, 114, 101, 97, 109, 101, 110, 100, 111, 98, 106, 10, 120, 114, 101, 102, 10, 48, 32, 53, 10, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 32, 54, 53, 53, 51, 53, 32, 102, 32, 10, 48, 48, 48, 48, 48, 48, 48, 48, 48, 57, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 48, 48, 48, 48, 48, 48, 48, 48, 53, 50, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 48, 48, 48, 48, 48, 48, 48, 49, 48, 49, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 48, 48, 48, 48, 48, 48, 48, 49, 57, 51, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 116, 114, 97, 105, 108, 101, 114, 10, 60, 60, 47, 83, 105, 122, 101, 32, 53, 47, 82, 111, 111, 116, 32, 49, 32, 48, 32, 82, 62, 62, 10, 115, 116, 97, 114, 116, 120, 114, 101, 102, 10, 50, 54, 57, 10, 37, 37, 69, 79, 70, 10,
    ];

    /// 从 Info 字典读文本（lopdf 读取时已把 Hexadecimal 解回字节；解 UTF-16BE/Literal），测试断言用
    fn info_text(dict: &Dictionary, key: &[u8]) -> Option<String> {
        match dict.get(key).ok()? {
            Object::String(bytes, _) => decode_utf16be(bytes),
            _ => None,
        }
    }

    fn decode_utf16be(bytes: &[u8]) -> Option<String> {
        if bytes.starts_with(&[0xFE, 0xFF]) {
            let units: Vec<u16> = bytes[2..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16(&units).ok()?)
        } else {
            String::from_utf8(bytes.to_vec()).ok()
        }
    }

    /// 回写后重开断言 Info 字段（中英混合，覆盖 UTF-16 编码路径）
    pub fn assert_info_fields(abs_path: &str, item: &crate::core::item::ItemCore) {
        let doc = Document::load(abs_path).expect("回写后的 PDF 应可打开");
        assert!(!doc.is_encrypted());
        let info = doc
            .trailer
            .get(b"Info")
            .ok()
            .and_then(|o| match o {
                Object::Reference(id) => doc.get_object(*id).ok(),
                other => Some(other),
            })
            .and_then(|o| o.as_dict().ok())
            .expect("trailer 应有 Info 字典");
        assert_eq!(info_text(info, b"Title").as_deref(), Some(item.title.as_str()));
        assert_eq!(info_text(info, b"Author").as_deref(), Some("作者甲, Author B"));
        assert_eq!(info_text(info, b"Subject").as_deref(), Some(item.description.as_str()));
        assert_eq!(info_text(info, b"Keywords").as_deref(), Some("科幻, 经典, 外国文学"));
    }

    #[test]
    fn corrupt_pdf_returns_err() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-pdf-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("bad.pdf");
        std::fs::write(&abs, b"%PDF-1.4 broken").unwrap();
        let item = crate::core::item::ItemCore::new("h", vec![], 0);
        assert!(write_pdf_metadata(abs.to_str().unwrap(), &item).is_err());
    }

    /// 给 MINIMAL_PDF 挂上既有 XMP（构造就地更新场景）
    fn pdf_with_xmp(xmp: &str) -> Vec<u8> {
        let mut doc = Document::load_mem(MINIMAL_PDF).expect("fixture 加载失败");
        let catalog_id = match doc.trailer.get(b"Root") {
            Ok(Object::Reference(id)) => *id,
            _ => panic!("fixture 无 Catalog"),
        };
        let stream_id = doc.add_object(Stream::new(Dictionary::new(), xmp.as_bytes().to_vec()));
        doc.get_object_mut(catalog_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Metadata", Object::Reference(stream_id));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// 回写后重开读 Catalog /Metadata XMP 文本
    fn xmp_of(abs_path: &str) -> String {
        let doc = Document::load(abs_path).expect("回写后的 PDF 应可打开");
        let catalog_id = match doc.trailer.get(b"Root") {
            Ok(Object::Reference(id)) => *id,
            _ => panic!("无 Catalog"),
        };
        let dict = doc.get_object(catalog_id).unwrap().as_dict().unwrap();
        match dict.get(b"Metadata").expect("无 XMP") {
            Object::Reference(id) => match doc.get_object(*id).unwrap() {
                Object::Stream(s) => String::from_utf8_lossy(&s.content).into_owned(),
                _ => panic!("Metadata 不是流"),
            },
            Object::Stream(s) => String::from_utf8_lossy(&s.content).into_owned(),
            _ => panic!("Metadata 类型异常"),
        }
    }

    /// 既有 XMP：受管字段就地更新，其余内容（xpacket PI、其它命名空间元素）存活
    #[test]
    fn pdf_xmp_update_preserves_existing() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-xmp-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("xmp.pdf");
        let old_xmp = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">XMP 旧标题</rdf:li></rdf:Alt></dc:title>\n   <dc:creator><rdf:Seq><rdf:li>XMP 旧作者</rdf:li></rdf:Seq></dc:creator>\n  </rdf:Description>\n  <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:xmpMM=\"http://ns.adobe.com/xap/1.0/mm/\">\n   <xmp:CreatorTool>sumi fixture</xmp:CreatorTool>\n   <xmpMM:DocumentID>uuid:fixed-marker</xmpMM:DocumentID>\n  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>";
        std::fs::write(&abs, pdf_with_xmp(old_xmp)).unwrap();

        let item = item_for_xmp();
        write_pdf_metadata(abs.to_str().unwrap(), &item).unwrap();

        let xmp = xmp_of(abs.to_str().unwrap());
        // 受管字段更新（作者多值 Seq；首包 Description 收口前注入 description/keywords）
        assert!(xmp.contains("&lt;&gt;特殊 &amp; 字符"), "标题转义与更新: {xmp}");
        assert!(!xmp.contains("XMP 旧标题"));
        assert!(xmp.contains("<rdf:li>作者甲</rdf:li>") && xmp.contains("<rdf:li>Author B</rdf:li>"));
        assert!(!xmp.contains("XMP 旧作者"));
        assert!(xmp.contains(">新的简介</rdf:li>"));
        assert!(xmp.contains("<pdf:Keywords"));
        assert!(xmp.contains("科幻, 经典, 外国文学"));
        // 非受管内容与 packet 骨架存活
        assert!(xmp.contains("<xmp:CreatorTool>sumi fixture</xmp:CreatorTool>"));
        assert!(xmp.contains("<xmpMM:DocumentID>uuid:fixed-marker</xmpMM:DocumentID>"));
        assert!(xmp.contains("<?xpacket begin=") && xmp.contains("end=\"w\"?>"));
        // Info 字典同口径更新
        assert_info_fields(abs.to_str().unwrap(), &item);
    }

    /// 受管字段全空的 item：XMP 对应元素删除（旧值不残留）；Info 同步删键
    #[test]
    fn pdf_xmp_empty_fields_removed() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-xmp-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("empty.pdf");
        let old_xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">".to_string()
            + "<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">"
            + "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">旧</rdf:li></rdf:Alt></dc:title>"
            + "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">旧简介</rdf:li></rdf:Alt></dc:description>"
            + "<pdf:Keywords>旧</pdf:Keywords>"
            + "</rdf:Description></rdf:RDF></x:xmpmeta>";
        std::fs::write(&abs, pdf_with_xmp(&old_xmp)).unwrap();

        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "保留标题".into();
        // description/tags/categories/authors 全空 → 对应元素删除，title 保留
        write_pdf_metadata(abs.to_str().unwrap(), &item).unwrap();

        let xmp = xmp_of(abs.to_str().unwrap());
        assert!(xmp.contains("保留标题"));
        assert!(!xmp.contains("dc:description"), "空值字段应删除: {xmp}");
        assert!(!xmp.contains("dc:creator"));
        assert!(!xmp.contains("pdf:Keywords"));
        assert!(!xmp.contains("旧简介"));
    }

    /// 无 XMP 的 PDF：创建最小合规 packet（xpacket 骨架 + 受管字段）
    #[test]
    fn pdf_xmp_created_when_missing() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-xmp-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("new.pdf");
        std::fs::write(&abs, MINIMAL_PDF).unwrap();

        let item = item_for_xmp();
        write_pdf_metadata(abs.to_str().unwrap(), &item).unwrap();

        let xmp = xmp_of(abs.to_str().unwrap());
        assert!(xmp.contains("<?xpacket begin=") && xmp.contains("<?xpacket end="), "packet 骨架: {xmp}");
        assert!(xmp.contains("&lt;&gt;特殊 &amp; 字符"));
        assert!(xmp.contains("<rdf:li>作者甲</rdf:li>"));
        assert!(xmp.contains("科幻, 经典, 外国文学"));
    }

    /// 损坏 XMP：硬错误不落盘（不做 Info-only 的半新半旧写入）
    #[test]
    fn pdf_malformed_xmp_returns_err() {
        let dir = std::env::temp_dir().join(format!("sumi-embed-xmp-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join("bad-xmp.pdf");
        std::fs::write(&abs, pdf_with_xmp("<not well-formed")).unwrap();
        let item = item_for_xmp();
        assert!(write_pdf_metadata(abs.to_str().unwrap(), &item).is_err());
    }

    /// XMP 测试共用 item（含 XML 特殊字符，覆盖转义路径）
    fn item_for_xmp() -> crate::core::item::ItemCore {
        let mut item = crate::core::item::ItemCore::new("h", vec![], 0);
        item.title = "<>特殊 & 字符".into();
        item.authors = vec!["作者甲".into(), "Author B".into()];
        item.description = "新的简介".into();
        item.tags = vec!["科幻".into(), "经典".into()];
        item.categories = vec!["外国文学".into()];
        item
    }
}
