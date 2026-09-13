//! 格式解析器统一入口：按扩展名分发（契约见 API 文档「支持格式」表）。
//! 产出 ParsedBook（书目元数据 + 封面字节 + 目录 + 全文正文），供流水线入库时回填
//! 与 S7 的 toc/content 端点复用。解析失败的格式走降级路径（文件名回退、封面 404 兜底）。

pub mod cbz;
pub mod docx;
pub mod epub;
pub mod pdf;
pub mod text;

use crate::core::item::ItemCore;
use serde::Serialize;

/// 目录节点（item/toc 契约；href 语义见 API 文档：anchor:/page:/空串）
#[derive(Clone, Debug, Default, Serialize)]
pub struct TocEntry {
    pub title: String,
    pub anchor: String,
    pub children: Vec<TocEntry>,
}

/// 解析产物：书目元数据 + 封面字节 + 目录 + 全文正文
#[derive(Default)]
pub struct ParsedBook {
    pub title: String,
    pub authors: Vec<String>,
    pub publisher: String,
    pub pubdate: String,
    pub isbn: String,
    pub language: String,
    pub series: String,
    pub series_index: f64,
    pub description: String,
    /// 封面原始字节（提取链；由封面管线解码→缩放→webp）
    pub cover: Option<Vec<u8>>,
    pub toc: Vec<TocEntry>,
    /// 全文正文（txt/md/epub/docx；None = 该格式不做全文索引）
    pub fulltext: Option<String>,
}

/// 按扩展名分发解析。解析器内部错误一律降级（空 ParsedBook 由调用方以文件名兜底），不阻断入库
pub fn parse(name: &str, ext: &str, bytes: &[u8]) -> ParsedBook {
    match ext {
        "txt" | "md" => text::parse_text(name, ext, bytes),
        "epub" => epub::parse_epub(name, bytes).map(|e| e.book).unwrap_or_default(),
        "docx" => docx::parse_docx(name, bytes),
        "cbz" => cbz::parse_cbz(name, bytes),
        // mobi/azw3：EXTH 元数据与封面（KF8 正文提取列后续版本，v1 不产 fulltext）
        "mobi" | "azw3" => parse_mobi(name, bytes),
        // pdf：pdfium 首页渲染作封面 + Info 字典元数据；缺库降级为首图提取（嵌入位图），
        // 仍无则由封面管线排版生成；阅读走 item/file + pdf.js
        "pdf" => pdf::parse_pdf(name, bytes),
        _ => ParsedBook { title: name.to_string(), ..Default::default() },
    }
}

/// 解析结果回填 item（只覆盖非用户编辑字段）
pub fn apply_to_item(item: &mut ItemCore, parsed: &ParsedBook) {
    let fields: [(&str, Box<dyn Fn(&mut ItemCore)>); 9] = [
        ("title", Box::new(|i: &mut ItemCore| i.title = parsed.title.clone())),
        ("authors", Box::new(|i: &mut ItemCore| i.authors = parsed.authors.clone())),
        ("publisher", Box::new(|i: &mut ItemCore| i.publisher = parsed.publisher.clone())),
        ("pubdate", Box::new(|i: &mut ItemCore| i.pubdate = parsed.pubdate.clone())),
        ("isbn", Box::new(|i: &mut ItemCore| i.isbn = parsed.isbn.clone())),
        ("language", Box::new(|i: &mut ItemCore| i.language = parsed.language.clone())),
        ("series", Box::new(|i: &mut ItemCore| i.series = parsed.series.clone())),
        ("series_index", Box::new(|i: &mut ItemCore| i.series_index = parsed.series_index)),
        ("description", Box::new(|i: &mut ItemCore| i.description = parsed.description.clone())),
    ];
    for (field, apply) in fields {
        if !item.is_overridden(field) {
            apply(item);
        }
    }
}

/// mobi/azw3：mobi crate 读 header/EXTH（title/author/publisher/isbn/language/封面 record）。
/// 封面定位：EXTH CoverOffset + MobiHeader.first_image_index → PDB record 区间切片
fn parse_mobi(name: &str, bytes: &[u8]) -> ParsedBook {
    let mut book = ParsedBook {
        title: name.to_string(),
        ..Default::default()
    };
    match mobi::Mobi::new(bytes.to_vec()) {
        Ok(m) => {
            let title = m.title();
            if !title.trim().is_empty() {
                book.title = title;
            }
            if let Some(a) = m.author() {
                if !a.trim().is_empty() {
                    book.authors = vec![a];
                }
            }
            if let Some(p) = m.publisher() {
                book.publisher = p;
            }
            if let Some(i) = m.isbn() {
                book.isbn = i;
            }
            if let Some(d) = m.description() {
                book.description = d;
            }
            if let Some(pd) = m.publish_date() {
                book.pubdate = pd;
            }
            book.language = format!("{:?}", m.language()).to_lowercase();

            // 封面：EXTH CoverOffset（u32 BE）+ first_image_index 定位 record
            let cover_offset = m
                .metadata
                .exth
                .get_record(mobi::headers::ExthRecord::CoverOffset)
                .and_then(|vals| vals.first())
                .and_then(|v| {
                    if v.len() >= 4 {
                        Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]]))
                    } else {
                        None
                    }
                });
            if let Some(offset) = cover_offset {
                let first_image = m.metadata.mobi.first_image_index;
                let records = &m.metadata.records.records;
                let idx = (first_image as usize).saturating_add(offset as usize);
                if idx < records.len() {
                    let start = records[idx].offset as usize;
                    let end = records.get(idx + 1).map(|r| r.offset as usize).unwrap_or(m.content.len());
                    if start < end && end <= m.content.len() {
                        book.cover = Some(m.content[start..end].to_vec());
                    }
                }
            }
        }
        Err(e) => {
            tracing::debug!("mobi 解析失败（降级文件名）: {e}");
        }
    }
    book
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::item::PathRecord;

    #[test]
    fn apply_respects_overrides() {
        let mut item = ItemCore::new("a", vec![PathRecord::new("x.epub", 1, 1)], 1);
        item.title = "用户改过的标题".into();
        item.mark_overridden("title");
        let parsed = ParsedBook {
            title: "解析标题".into(),
            authors: vec!["作者".into()],
            ..Default::default()
        };
        apply_to_item(&mut item, &parsed);
        assert_eq!(item.title, "用户改过的标题"); // 用户编辑优先
        assert_eq!(item.authors, vec!["作者".to_string()]); // 非编辑字段覆盖
    }

    #[test]
    fn unknown_ext_falls_back() {
        let book = parse("随便", "exe", b"binary");
        assert_eq!(book.title, "随便");
        assert!(book.cover.is_none());
        assert!(book.toc.is_empty());
    }

    /// 手工构造的单页 PDF（正确 xref；红色描边矩形，MediaBox 200×300）
    const MINIMAL_PDF: &[u8] = &[
        37, 80, 68, 70, 45, 49, 46, 52, 10, 49, 32, 48, 32, 111, 98, 106, 60, 60, 47, 84, 121, 112, 101, 47, 67, 97, 116, 97, 108, 111, 103, 47, 80, 97, 103, 101, 115, 32, 50, 32, 48, 32, 82, 62, 62, 101, 110, 100, 111, 98, 106, 10, 50, 32, 48, 32, 111, 98, 106, 60, 60, 47, 84, 121, 112, 101, 47, 80, 97, 103, 101, 115, 47, 75, 105, 100, 115, 91, 51, 32, 48, 32, 82, 93, 47, 67, 111, 117, 110, 116, 32, 49, 62, 62, 101, 110, 100, 111, 98, 106, 10, 51, 32, 48, 32, 111, 98, 106, 60, 60, 47, 84, 121, 112, 101, 47, 80, 97, 103, 101, 47, 80, 97, 114, 101, 110, 116, 32, 50, 32, 48, 32, 82, 47, 77, 101, 100, 105, 97, 66, 111, 120, 91, 48, 32, 48, 32, 50, 48, 48, 32, 51, 48, 48, 93, 47, 67, 111, 110, 116, 101, 110, 116, 115, 32, 52, 32, 48, 32, 82, 47, 82, 101, 115, 111, 117, 114, 99, 101, 115, 60, 60, 62, 62, 62, 62, 101, 110, 100, 111, 98, 106, 10, 52, 32, 48, 32, 111, 98, 106, 60, 60, 47, 76, 101, 110, 103, 116, 104, 32, 51, 49, 62, 62, 115, 116, 114, 101, 97, 109, 10, 49, 32, 48, 32, 48, 32, 82, 71, 32, 50, 32, 119, 32, 50, 48, 32, 50, 48, 32, 49, 54, 48, 32, 50, 54, 48, 32, 114, 101, 32, 83, 10, 101, 110, 100, 115, 116, 114, 101, 97, 109, 101, 110, 100, 111, 98, 106, 10, 120, 114, 101, 102, 10, 48, 32, 53, 10, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 32, 54, 53, 53, 51, 53, 32, 102, 32, 10, 48, 48, 48, 48, 48, 48, 48, 48, 48, 57, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 48, 48, 48, 48, 48, 48, 48, 48, 53, 50, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 48, 48, 48, 48, 48, 48, 48, 49, 48, 49, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 48, 48, 48, 48, 48, 48, 48, 49, 57, 51, 32, 48, 48, 48, 48, 48, 32, 110, 32, 10, 116, 114, 97, 105, 108, 101, 114, 10, 60, 60, 47, 83, 105, 122, 101, 32, 53, 47, 82, 111, 111, 116, 32, 49, 32, 48, 32, 82, 62, 62, 10, 115, 116, 97, 114, 116, 120, 114, 101, 102, 10, 50, 54, 57, 10, 37, 37, 69, 79, 70, 10,
    ];

    /// pdf 解析：无嵌入位图时不产封面（不自渲染、不排版生成）
    #[test]
    fn pdf_without_image_has_no_cover() {
        let book = parse("凤凰项目", "pdf", MINIMAL_PDF);
        assert_eq!(book.title, "凤凰项目"); // Info 字典无 Title → 回退文件名
        assert!(book.cover.is_none(), "无首图时不产封面");
    }
}
