//! pdf 解析（纯解析，无自渲染）：Info 字典元数据 + 首页首图封面。
//! 封面链：首页嵌入位图提取（DCT=JPEG 直出、FlateDecode RGB/灰度转 PNG）；无图则跳过（不生成）。

use crate::core::parser::ParsedBook;
use lopdf::{Document, Object};

/// pdf 解析：Info 字典 Title/Author + 首页首图封面（无图时首页文字排版作扉页封面）
pub fn parse_pdf(name: &str, bytes: &[u8]) -> ParsedBook {
    let mut book = ParsedBook {
        title: name.to_string(),
        ..Default::default()
    };
    let doc = Document::load_mem(bytes).ok();
    if let Some(doc) = &doc {
        let (title, author) = read_info(doc);
        if !title.is_empty() {
            book.title = title;
        }
        if !author.is_empty() {
            book.authors = vec![author];
        }
    }
    book.cover = extract_pdf_first_image(bytes);
    if book.cover.is_none() {
        // 纯排版首页（无嵌入位图）：首页文字可读才作扉页封面；乱码（subset CID 无 ToUnicode）
        // 回退文件名排版封面
        if let Some(doc) = &doc {
            if let Some((&page_no, _)) = doc.get_pages().iter().next() {
                if let Ok(text) = doc.extract_text(&[page_no]) {
                    if crate::core::cover::looks_readable(&text) {
                        if let Some((bytes, _, _)) = crate::core::cover::text_page_cover(&text) {
                            book.cover = Some(bytes);
                        }
                    } else if !text.trim().is_empty() {
                        let (bytes, _, _) = crate::core::cover::generated_cover(&book.title, &book.authors);
                        if !bytes.is_empty() {
                            book.cover = Some(bytes);
                        }
                    }
                }
            }
        }
    }
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
    // Resources 可继承：页无直接 Resources 时沿 Parent 链向上找
    let mut dict = Some(page);
    while let Some(d) = dict {
        if let Ok(res) = d.get(b"Resources") {
            if let Ok((_, res)) = doc.dereference(res) {
                if let Ok(res) = res.as_dict() {
                    // XObject 遍历（Form 嵌套下钻，深度限 3）
                    if let Some(bytes) = xobject_first_image(&doc, res, 0) {
                        return Some(bytes);
                    }
                    break;
                }
            }
        }
        dict = d.get(b"Parent").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok());
    }
    // 回退：页面自带缩略图（/Thumb 是页面对象上的图像流，恰好是首页缩略）
    if let Ok(thumb) = page.get(b"Thumb") {
        if let Ok((_, thumb)) = doc.dereference(thumb) {
            if let Ok(stream) = thumb.as_stream() {
                if let Some(bytes) = image_stream_bytes(&doc, stream) {
                    return Some(bytes);
                }
            }
        }
    }
    None
}

/// XObject 字典中第一个可解码图像（Form XObject 递归下钻，深度限 3 防循环引用）
fn xobject_first_image(doc: &Document, resources: &lopdf::Dictionary, depth: usize) -> Option<Vec<u8>> {
    if depth >= 3 {
        return None;
    }
    let xobjects = resources.get(b"XObject").ok().and_then(|o| doc.dereference(o).ok())?.1.as_dict().ok()?;
    for (_, obj) in xobjects.iter() {
        let Some(stream) = doc.dereference(obj).ok().and_then(|(_, o)| o.as_stream().ok()) else {
            continue;
        };
        let dict = &stream.dict;
        match dict.get(b"Subtype").and_then(Object::as_name).ok() {
            Some(b"Image") => {
                if let Some(bytes) = image_stream_bytes(doc, stream) {
                    return Some(bytes);
                }
            }
            // Form XObject：容器自带 Resources,下钻其 XObject
            Some(b"Form") => {
                if let Some(res) = dict
                    .get(b"Resources")
                    .ok()
                    .and_then(|o| doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_dict().ok())
                {
                    if let Some(bytes) = xobject_first_image(doc, res, depth + 1) {
                        return Some(bytes);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// 图像流 → 可解码字节（JPEG 原样直出；FlateDecode 的 8bit RGB/灰度转 PNG；其余编码放弃）
fn image_stream_bytes(doc: &Document, stream: &lopdf::Stream) -> Option<Vec<u8>> {
    let dict = &stream.dict;
    let filters = filter_names(dict);
    let width = dict.get(b"Width").ok()?.as_i64().ok()? as u32;
    let height = dict.get(b"Height").ok()?.as_i64().ok()? as u32;
    let bpc = dict.get(b"BitsPerComponent").ok().and_then(|o| o.as_i64().ok()).unwrap_or(8);
    if filters == ["CCITTFaxDecode"] {
        // CCITT 传真编码（扫描书，1bit 位图）：fax crate 解码 G4 → 灰度 → PNG
        return decode_ccitt(stream);
    }
    if bpc != 8 {
        return None;
    }
    if filters == ["DCTDecode"] {
        // JPEG 原样：封面管线（image crate）直接解码
        return Some(stream.content.clone());
    }
    if filters == ["JPXDecode"] {
        // JPEG 2000：jpeg2k 解码 → RGB → PNG（扫描书/部分制作器常见）
        return decode_jpx(&stream.content);
    }
    // 滤镜链：尾部 DCTDecode + 前段全部 FlateDecode（zlib 压缩的 JPEG——先解压再直出）
    if filters.last().map(String::as_str) == Some("DCTDecode")
        && filters[..filters.len() - 1].iter().all(|f| f == "FlateDecode")
    {
        // 借 lopdf 的 flate 解压：合成单 FlateDecode 滤镜的流重放解压，免新增压缩依赖
        let synthetic = lopdf::Stream::new(
            lopdf::Dictionary::from_iter(vec![(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()))]),
            stream.content.clone(),
        );
        return synthetic.decompressed_content().ok().filter(|b| b.starts_with(&[0xFF, 0xD8]));
    }
    if filters == ["FlateDecode"] {
        let raw = stream.decompressed_content().ok()?;
        // DecodeParms 预测器：1 = 原样；2 = TIFF 水平差分；10–15 = PNG 行滤波（行首 1 字节滤波类型）
        let (predictor, columns, colors) = decode_parms(dict);
        // Columns/Colors 缺省按页宽与色空间兜底（真实文件的通用形态）
        let columns = if columns == 0 { width as usize } else { columns };
        let form = colorspace(doc, dict)?;
        let colors = if colors == 0 {
            match &form {
                ColorForm::Rgb => 3,
                ColorForm::Gray | ColorForm::Indexed(_) => 1, // Indexed 每像素 1 字节索引
            }
        } else {
            colors
        };
        let row_bytes = columns * colors;
        let data = match predictor {
            1 => raw,
            2 => tiff_unpredict(&raw, row_bytes, colors)?,
            10..=15 => {
                // PNG 行滤波的行首字节判定（长度自适应）：有的生成器声明了 Predictor 但
                // 数据并未加滤波字节（Quick PDF Library 等）——按实际长度判断
                let plain_len = height as usize * row_bytes;
                let filtered_len = height as usize * (row_bytes + 1);
                if raw.len() == filtered_len && raw.len() != plain_len {
                    png_unfilter(&raw, row_bytes, colors)?
                } else {
                    raw // 无滤波字节，按原样
                }
            }
            _ => return None,
        };
        let rgb: Vec<u8> = match &form {
            ColorForm::Rgb if data.len() >= (width * height * 3) as usize => data,
            ColorForm::Gray if data.len() >= (width * height) as usize => {
                // 灰度 → RGB 扩展（统一管线入口）
                data.into_iter().flat_map(|g| [g, g, g]).collect()
            }
            ColorForm::Indexed(lookup) if data.len() >= (width * height) as usize => {
                // 调色板索引 → RGB 映射
                data.into_iter()
                    .flat_map(|i| {
                        let base = (i as usize) * 3;
                        [lookup[base], lookup[base + 1], lookup[base + 2]]
                    })
                    .collect()
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
    // CCITT/JBIG2/多段 Filter 链（扫描书常见）：无可靠纯 Rust 解码，放弃 → 上层无封面
}

/// JPXDecode（JPEG 2000）解码：jpeg2k 解为 RGB → PNG（扫描书/部分制作器常见）
fn decode_jpx(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = jpeg2k::Image::from_bytes(bytes).ok()?;
    let pixels = img.get_pixels(None).ok()?;
    let jpeg2k::ImagePixelData::Rgb8(data) = pixels.data else {
        return None; // 灰度/RGBA/16bit 等暂不支持（封面场景 RGB 为主）
    };
    let buf = image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(pixels.width, pixels.height, data)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(buf)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// CCITTFaxDecode（Group 4/3 传真，扫描书）解码：fax crate 逐行过渡 → 像素 → 灰度图 → PNG
fn decode_ccitt(stream: &lopdf::Stream) -> Option<Vec<u8>> {
    let dict = &stream.dict;
    let width = dict.get(b"Width").ok()?.as_i64().ok()? as u32;
    let height = dict.get(b"Height").ok()?.as_i64().ok()? as u32;
    // DecodeParms：K=-1 → G4（默认）；K=0 → G3 1D/2D；BlackIs1 默认 false（0=黑，1=白）
    let parms = dict.get(b"DecodeParms").ok().and_then(|o| o.as_dict().ok());
    let get_i64 = |k: &[u8]| parms.and_then(|d| d.get(k).ok()).and_then(|o| o.as_i64().ok());
    let k = get_i64(b"K").unwrap_or(-1);
    let columns = get_i64(b"Columns").unwrap_or(width as i64).max(1) as u32;
    let rows = get_i64(b"Rows").unwrap_or(height as i64).max(1) as u32;
    let black_is_1 = get_i64(b"BlackIs1").unwrap_or(0) != 0;

    let mut gray = vec![255u8; (columns * rows) as usize];
    let mut line_no = 0u32;
    let mut cur: Vec<u8> = Vec::with_capacity(columns as usize);
    // CCITT 默认：0=黑 1=白；BlackIs1 翻转。fax pels 以白起首
    let (black, white) = if black_is_1 { (255u8, 0u8) } else { (0u8, 255u8) };
    let mut line_cb = |transitions: &[u32]| {
        if line_no >= rows {
            return;
        }
        cur.clear();
        for color in fax::decoder::pels(transitions, columns) {
            cur.push(match color {
                fax::Color::Black => black,
                fax::Color::White => white,
            });
        }
        // 行尾截断补齐（过渡序列不足列宽时余为白）
        cur.resize(columns as usize, white);
        let start = (line_no * columns) as usize;
        gray[start..start + columns as usize].copy_from_slice(&cur);
        line_no += 1;
    };
    let input = stream.content.iter().copied();
    if k <= -1 {
        fax::decoder::decode_g4(input, columns, Some(rows), &mut line_cb)?;
    } else {
        // G3(K=0) 行宽自适应无法预分配，封面场景罕见，跳过
        return None;
    }
    let img = image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(columns, rows, gray)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageLuma8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
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

/// 色空间形态：设备 RGB / 灰度 / Indexed（调色板 RGB）
#[derive(Debug)]
enum ColorForm {
    Rgb,
    Gray,
    /// Indexed 调色板（索引 → RGB 查找表，255 级）
    Indexed(Vec<u8>),
}

/// /ColorSpace 解析（支持间接引用与 Indexed 数组形态）
fn colorspace(doc: &Document, dict: &lopdf::Dictionary) -> Option<ColorForm> {
    let obj = dict.get(b"ColorSpace").ok()?;
    let obj = doc.dereference(obj).ok().map(|(_, o)| o).unwrap_or(obj);
    match obj {
        Object::Name(n) => match n.as_slice() {
            b"DeviceRGB" => Some(ColorForm::Rgb),
            b"DeviceGray" => Some(ColorForm::Gray),
            _ => None,
        },
        Object::Array(items) => {
            let first = items.first()?.as_name().ok()?;
            if first == b"Indexed" || first == b"I" {
                // [/Indexed base hival lookup]：lookup 是字符串对象或流（RGB 查找表）
                let lookup_obj = items.get(3)?;
                let lookup_obj = doc.dereference(lookup_obj).ok().map(|(_, o)| o).unwrap_or(lookup_obj);
                let bytes: Vec<u8> = match lookup_obj {
                    Object::String(b, _) => b.clone(),
                    Object::Stream(s) => s.decompressed_content().unwrap_or_else(|_| s.content.clone()),
                    _ => return None,
                };
                if bytes.len() < 768 {
                    return None;
                }
                Some(ColorForm::Indexed(bytes))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// /DecodeParms 解析：(Predictor, Columns, Colors)；缺失按 PDF 标准缺省（1/0 未设）
fn decode_parms(dict: &lopdf::Dictionary) -> (i64, usize, usize) {
    let Ok(parms) = dict.get(b"DecodeParms") else {
        return (1, 0, 0);
    };
    let Ok(d) = parms.as_dict() else {
        return (1, 0, 0);
    };
    let get_i64 = |k: &[u8]| d.get(k).ok().and_then(|o| o.as_i64().ok());
    (
        get_i64(b"Predictor").unwrap_or(1),
        get_i64(b"Columns").unwrap_or(0).max(0) as usize,
        get_i64(b"Colors").unwrap_or(0).max(0) as usize,
    )
}

/// TIFF Predictor 2 反预测：水平差分（同像素分量减左邻同分量，无行首字节）
fn tiff_unpredict(raw: &[u8], row_bytes: usize, bpp: usize) -> Option<Vec<u8>> {
    if row_bytes == 0 || raw.len() % row_bytes != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len());
    for line in raw.chunks_exact(row_bytes) {
        let mut cur = vec![0u8; row_bytes];
        for i in 0..row_bytes {
            let left = if i >= bpp { cur[i - bpp] } else { 0 };
            cur[i] = line[i].wrapping_add(left);
        }
        out.extend_from_slice(&cur);
    }
    Some(out)
}

/// PNG 预测器（Predictor 10–15）反滤波：每行 1 字节滤波类型 + 数据行，四种滤波器
fn png_unfilter(raw: &[u8], row_bytes: usize, bpp: usize) -> Option<Vec<u8>> {
    if row_bytes == 0 || raw.len() % (row_bytes + 1) != 0 {
        return None;
    }
    let rows = raw.len() / (row_bytes + 1);
    let mut out = Vec::with_capacity(rows * row_bytes);
    let mut prev = vec![0u8; row_bytes];
    for r in 0..rows {
        let line = &raw[r * (row_bytes + 1) + 1..(r + 1) * (row_bytes + 1)];
        let filter = raw[r * (row_bytes + 1)];
        let mut cur = vec![0u8; row_bytes];
        match filter {
            0 => cur.copy_from_slice(line),
            1 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { cur[i - bpp] } else { 0 };
                    cur[i] = line[i].wrapping_add(a);
                }
            }
            2 => {
                for i in 0..row_bytes {
                    cur[i] = line[i].wrapping_add(prev[i]);
                }
            }
            3 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { cur[i - bpp] } else { 0 };
                    let b = prev[i];
                    cur[i] = line[i].wrapping_add((((a as u16) + (b as u16)) / 2) as u8);
                }
            }
            4 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { cur[i - bpp] } else { 0 };
                    let b = prev[i];
                    let c = if i >= bpp { prev[i - bpp] } else { 0 };
                    cur[i] = line[i].wrapping_add(paeth(a, b, c));
                }
            }
            _ => return None,
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Some(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i32, b as i32, c as i32);
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
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

    /// 手工 zlib 流（stored block + adler32）：测试夹具不引入压缩依赖
    fn zlib_store(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let mut rest = data;
        while !rest.is_empty() {
            let take = rest.len().min(65535);
            let last = take == rest.len();
            out.push(if last { 1 } else { 0 });
            out.extend_from_slice(&(take as u16).to_le_bytes());
            out.extend_from_slice(&(!(take as u16)).to_le_bytes());
            out.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
        }
        let mut a = 1u32;
        let mut b = 0u32;
        for &byte in data {
            a = (a + byte as u32) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    /// 手工构造：FlateDecode + Predictor 15（PNG 行滤波,行首 0=None）的嵌入图 PDF
    fn build_pdf_with_png_filtered_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
        let row_bytes = width as usize * 3;
        // 加行首滤波字节(0 = None)
        let mut filtered = Vec::with_capacity(height as usize * (row_bytes + 1));
        for r in 0..height as usize {
            filtered.push(0);
            filtered.extend_from_slice(&rgb[r * row_bytes..(r + 1) * row_bytes]);
        }
        let compressed = zlib_store(&filtered);
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
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /XObject << /Im1 4 0 R >> >> >>",
        );
        let header = format!(
            "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 3 /Columns {width} >> /Length {} >>",
            compressed.len()
        );
        let mut img_obj = header.into_bytes();
        img_obj.write_all(b"stream\n").unwrap();
        img_obj.write_all(&compressed).unwrap();
        img_obj.write_all(b"\nendstream").unwrap();
        push(&mut out, 4, &img_obj);
        let xref_pos = out.len();
        out.write_all(b"xref\n0 5\n0000000000 65535 f \n").unwrap();
        for off in offsets {
            out.write_all(format!("{off:010} 00000 n \n").as_bytes()).unwrap();
        }
        out.write_all(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n").as_bytes(),
        )
        .unwrap();
        out
    }

    #[test]
    fn flate_png_predictor_extraction() {
        // 8×8 纯红 RGB,行首 0(None)滤波 + Predictor 15:反滤波应还原,管线解码红色命中
        let rgb = vec![200u8, 30, 30].repeat(64);
        let pdf = build_pdf_with_png_filtered_rgb(8, 8, &rgb);
        let extracted = extract_pdf_first_image(&pdf).expect("PNG 预测器图像应可提取");
        assert_eq!(&extracted[..4], b"\x89PNG"); // 转码为 PNG
        let (_, w, h) = crate::core::cover::process_cover(&extracted).expect("管线可解码");
        assert_eq!((w, h), (8, 8));
    }

    /// PNG Sub 滤波纯函数级验证
    #[test]
    fn png_unfilter_sub() {
        // 一行 3 像素 RGB(row_bytes=9,bpp=3),Sub 滤波:第一像素原值,其余 = 差分
        // 原值: [10,20,30, 12,22,32, 15,25,35]
        // Sub 编码(行首 1 = Sub 滤波器): [1, 10,20,30, 2,2,2, 3,3,3]
        let raw = [1u8, 10, 20, 30, 2, 2, 2, 3, 3, 3];
        let out = png_unfilter(&raw, 9, 3).unwrap();
        assert_eq!(out, vec![10, 20, 30, 12, 22, 32, 15, 25, 35]);
    }

    #[test]
    fn no_image_returns_none() {
        // 无图页（前一版手工构造的文字页）
        let pdf: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n%%EOF\n";
        assert!(extract_pdf_first_image(pdf).is_none());
    }
}








