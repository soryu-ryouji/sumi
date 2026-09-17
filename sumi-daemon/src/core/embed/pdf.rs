//! PDF 元数据回写：lopdf 改 trailer /Info 字典（Title/Author/Subject/Keywords），全量重写保存。
//! PDF 无系列/分类独立概念：series 不写；tags+categories 并入 Keywords（逗号连接）。
//! 文本字符串编码：ASCII 走 Literal；含非 ASCII 用 UTF-16BE（带 BOM）Hexadecimal——
//! PDF 1.7 规范 §7.9.2 的 text string 标准 Encoding，兼顾各阅读器兼容。
//! 空字段删除对应键（保存时清空 = 文件同步清空）；加密/损坏 PDF 直接报错不写。

use crate::core::item::ItemCore;
use lopdf::{Dictionary, Document, Object, StringFormat};

/// 回写 PDF 元数据（Info 字典）。只管书目四字段：Title/Author/Subject/Keywords。
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

    let mut buf = Vec::new();
    doc.save_to(&mut buf).map_err(|e| format!("PDF 序列化失败: {e}"))?;
    crate::core::config::atomic_write(abs_path, &buf).map_err(|e| format!("写入失败: {e}"))?;
    Ok(())
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
}
