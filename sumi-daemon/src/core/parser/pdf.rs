//! pdf 解析（纯解析，无自渲染）：Info 字典元数据 + 首页首图封面。
//! 封面链：首页嵌入位图提取（DCT=JPEG 直出、FlateDecode RGB/灰度转 PNG）；无图则跳过（不生成）。

use crate::core::parser::ParsedBook;
use lopdf::{Document, Object};

/// pdf 解析：Info 字典 Title/Author + 首页首图封面（无图不生成封面）
pub fn parse_pdf(name: &str, bytes: &[u8]) -> ParsedBook {
    let mut book = ParsedBook {
        title: name.to_string(),
        ..Default::default()
    };
    if let Ok(doc) = Document::load_mem(bytes) {
        let (title, author) = read_info(&doc);
        if !title.is_empty() {
            book.title = title;
        }
        if !author.is_empty() {
            book.authors = vec![author];
        }
    }
    book.cover = extract_pdf_first_image(bytes);
    book
}

/// Info 字典的 Title/Author（trailer → Info 间接引用；UTF-16BE BOM 与单字节编码两制）
fn read_info(doc: &Document) -> (String, String) {
    let mut title = String::new();
    let mut author = String::new();
    let info = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok().cloned());
    if let Some(info) = info {
        for (key, out) in [(b"Title".as_slice(), &mut title), (b"Author".as_slice(), &mut author)] {
            if let Ok(Object::String(bytes, _)) = info.get(key) {
                let decoded = decode_pdf_string(bytes);
                if !decoded.trim().is_empty() {
                    *out = decoded;
                }
            }
        }
    }
    (title, author)
}

/// PDF 字符串解码：UTF-16BE BOM 优先，其余按 lossy UTF-8 兜底（PDFDocEncoding 近似）
fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| ((c[0] as u16) << 8) | c[1] as u16)
            .collect();
        String::from_utf16_lossy(&units).trim().to_string()
    } else {
        String::from_utf8_lossy(bytes).trim().to_string()
    }
}

/// 首页嵌入位图提取：遍历第一页 Resources/XObject 中的 Image 对象，取第一个可解码的。
/// 字典序迭代（lopdf Dictionary 为 BTreeMap，稳定序）；"第一张"是启发式——出版社 logo 抢先的风险
/// 存在（封面页带出版社 logo 的书可能取到装饰图），可接受
pub fn extract_pdf_first_image(bytes: &[u8]) -> Option<Vec<u8>> {
    let doc = Document::load_mem(bytes).ok()?;
    let pages = doc.get_pages();
    let (_, first_page_id) = pages.iter().next()?;
    let page = doc.get_dictionary(*first_page_id).ok()?;
    let resources = page.get(b"Resources").ok().and_then(|o| doc.dereference(o).ok())?;
    let resources = resources.1.as_dict().ok()?;
    let xobjects = resources.get(b"XObject").ok().and_then(|o| doc.dereference(o).ok())?.1.as_dict().ok()?;
    for (_, obj) in xobjects.iter() {
        let Some(stream) = doc.dereference(obj).ok().and_then(|(_, o)| o.as_stream().ok()) else {
            continue;
        };
        let dict = &stream.dict;
        if dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image".as_slice()) {
            continue;
        }
        if let Some(bytes) = image_stream_bytes(stream) {
            return Some(bytes);
        }
    }
    None
}

/// 图像流 → 可解码字节（JPEG 原样直出；FlateDecode 的 8bit RGB/灰度转 PNG；其余编码放弃）
fn image_stream_bytes(stream: &lopdf::Stream) -> Option<Vec<u8>> {
    let dict = &stream.dict;
    let filters = filter_names(dict);
    let width = dict.get(b"Width").ok()?.as_i64().ok()? as u32;
    let height = dict.get(b"Height").ok()?.as_i64().ok()? as u32;
    let bpc = dict.get(b"BitsPerComponent").ok().and_then(|o| o.as_i64().ok()).unwrap_or(8);
    let colorspace = dict.get(b"ColorSpace").ok().and_then(|o| o.as_name().ok()).map(String::from_utf8_lossy);
    if bpc != 8 {
        return None;
    }
    if filters == ["DCTDecode"] {
        // JPEG 原样：封面管线（image crate）直接解码
        return Some(stream.content.clone());
    }
    if filters == ["FlateDecode"] {
        // 带预测器（DecodeParms/PNG 预测）的暂不支持，跳过（封面场景少见）
        if dict.get(b"DecodeParms").is_ok() {
            return None;
        }
        let raw = stream.decompressed_content().ok()?;
        let rgb: Vec<u8> = match colorspace.as_deref().as_deref() {
            Some("DeviceRGB") if raw.len() == (width * height * 3) as usize => raw,
            Some("DeviceGray") if raw.len() == (width * height) as usize => {
                // 灰度 → RGB 扩展（统一管线入口）
                raw.into_iter().flat_map(|g| [g, g, g]).collect()
            }
            _ => return None,
        };
        let img = image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(width, height, rgb)?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .ok()?;
        return Some(png);
    }
    None
    // CCITT/JBIG2/JPX/多段 Filter 链（扫描书常见）：无可靠纯 Rust 解码，放弃 → 上层回退排版封面
}

/// /Filter 的名字集合（单个 Name 或 Name 数组）
fn filter_names(dict: &lopdf::Dictionary) -> Vec<String> {
    match dict.get(b"Filter") {
        Ok(Object::Name(n)) => vec![String::from_utf8_lossy(n).into_owned()],
        Ok(Object::Array(items)) => items
            .iter()
            .filter_map(|o| o.as_name().ok().map(|n| String::from_utf8_lossy(n).into_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 手工构造：一页内嵌一张 JPEG（DCTDecode）的 PDF
    fn build_pdf_with_jpeg(jpeg: &[u8]) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        let mut push = |out: &mut Vec<u8>, i: usize, body: &[u8]| {
            offsets.push(out.len());
            out.write_all(format!("{i} 0 obj").as_bytes()).unwrap();
            out.write_all(body).unwrap();
            out.write_all(b"endobj\n").unwrap();
        };
        push(&mut out, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
        push(&mut out, 2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        push(
            &mut out,
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>",
        );
        let img_header = format!(
            "<< /Type /XObject /Subtype /Image /Width 40 /Height 60 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>",
            jpeg.len()
        );
        let mut img_obj = img_header.into_bytes();
        img_obj.write_all(b"stream\n").unwrap();
        img_obj.write_all(jpeg).unwrap();
        img_obj.write_all(b"\nendstream").unwrap();
        push(&mut out, 4, &img_obj);
        let content = b"q 40 0 0 60 0 0 cm /Im1 Do Q";
        let content_obj = [format!("<< /Length {} >>", content.len()).as_bytes(), b"stream\n", content, b"\nendstream"].concat();
        push(&mut out, 5, &content_obj);
        let xref_pos = out.len();
        out.write_all(b"xref\n0 6\n0000000000 65535 f \n").unwrap();
        for off in offsets {
            out.write_all(format!("{off:010} 00000 n \n").as_bytes()).unwrap();
        }
        out.write_all(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n").as_bytes(),
        )
        .unwrap();
        out
    }

    #[test]
    fn first_image_extraction_jpeg() {
        // 40×60 红色 JPEG 嵌入 PDF → 提取应原样命中，且封面管线可解码
        let img = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(40, 60, image::Rgb([200, 30, 30])));
        let mut jpeg = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        let pdf = build_pdf_with_jpeg(&jpeg);
        let extracted = extract_pdf_first_image(&pdf).expect("应提取到首页位图");
        assert_eq!(&extracted[..3], &[0xFF, 0xD8, 0xFF]); // JPEG 魔数原样直出
        let (_, w, h) = crate::core::cover::process_cover(&extracted).expect("封面管线可解码");
        assert_eq!((w, h), (40, 60));
    }

    #[test]
    fn no_image_returns_none() {
        // 无图页（前一版手工构造的文字页）
        let pdf: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n%%EOF\n";
        assert!(extract_pdf_first_image(pdf).is_none());
    }
}
