//! 元数据回写：把库内元数据写回 EPUB/PDF 文件本身（保存时自动触发）。
//! 写回改变文件内容哈希 → item id 漂移由 watcher 经 pipeline::migrate_id_drift 收敛，
//! 本模块只做纯文件改写，不触碰索引与元数据存储。
//! 失败不致命：单路径失败不回滚保存，失败列表经 SSE item.embed_failed 广播。

pub mod epub;
pub mod pdf;

use crate::core::item::ItemCore;
use crate::core::paths::LibraryPaths;

/// 单路径回写失败记录（SSE item.embed_failed 负载元素）
#[derive(Debug, serde::Serialize)]
pub struct EmbedFailure {
    pub path: String,
    pub error: String,
}

/// 该路径是否为可回写格式（epub/pdf 之外不写：txt 无内嵌元数据、mobi 写坏风险高）
pub fn embeddable_ext(rel: &str) -> bool {
    matches!(LibraryPaths::ext_of(rel).as_str(), "epub" | "pdf")
}

/// item 是否包含任一库内（非回收站）的可回写格式路径——API 层快速短路用
pub fn has_embeddable_path(item: &ItemCore) -> bool {
    item.paths
        .iter()
        .any(|p| !LibraryPaths::is_in_trash(&p.path) && embeddable_ext(&p.path))
}

/// 对 item 的全部库内路径回写元数据。
/// 同内容多路径必须全部写入：只写主路径会让副本哈希分叉、被扫描器拆成两个 item。
/// 单路径失败不影响其它路径；返回失败列表。cover_png 为处理好的 PNG 字节（None = 不动封面）。
pub fn embed_metadata(paths: &LibraryPaths, item: &ItemCore, cover_png: Option<&[u8]>) -> Vec<EmbedFailure> {
    let mut failures = Vec::new();
    for record in &item.paths {
        let rel = &record.path;
        if LibraryPaths::is_in_trash(rel) || !embeddable_ext(rel) {
            continue;
        }
        let Some(abs) = paths.to_absolute(rel) else {
            failures.push(EmbedFailure { path: rel.clone(), error: "路径非法".into() });
            continue;
        };
        let result = match LibraryPaths::ext_of(rel).as_str() {
            "epub" => epub::write_epub_metadata(&abs, item, cover_png),
            "pdf" => pdf::write_pdf_metadata(&abs, item),
            _ => continue,
        };
        if let Err(e) = result {
            failures.push(EmbedFailure { path: rel.clone(), error: e });
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::item::PathRecord;
    use std::io::Write;

    fn temp_dir(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sumi-embed-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.to_string_lossy().into_owned()
    }

    fn item_with(paths: &[&str]) -> ItemCore {
        let mut item = ItemCore::new(
            "testhash",
            paths.iter().map(|p| PathRecord::new(*p, 1, 0)).collect(),
            0,
        );
        item.title = "新标题".into();
        item.authors = vec!["作者甲".into(), "Author B".into()];
        item.publisher = "新出版社".into();
        item.description = "新的简介".into();
        item.tags = vec!["科幻".into(), "经典".into()];
        item.categories = vec!["外国文学".into()];
        item.series = "新系列".into();
        item.series_index = 2.0;
        item
    }

    fn root_for(tag: &str) -> LibraryPaths {
        LibraryPaths::new(&temp_dir(tag), None)
    }

    /// PNG 魔数开头的假封面字节（回写不做图像解码校验，字节原样入包）
    fn fake_png(tag: u8) -> Vec<u8> {
        let mut v = vec![0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.push(tag);
        v
    }

    fn write_fixture(tag: &str, name: &str, bytes: &[u8]) -> String {
        let rel = format!("books/{name}");
        let abs = format!("{}/{}", temp_dir(tag), rel);
        std::fs::create_dir_all(std::path::Path::new(&abs).parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(&abs).unwrap();
        f.write_all(bytes).unwrap();
        rel
    }

    #[test]
    fn multi_path_epub_writes_all_copies() {
        let tag = "multi";
        let paths = root_for(tag);
        let bytes = super::epub::tests::build_epub3_fixture();
        let rel1 = write_fixture(tag, "a.epub", &bytes);
        let rel2 = write_fixture(tag, "sub/b.epub", &bytes);
        let item = item_with(&[&rel1, &rel2]);

        let failures = embed_metadata(&paths, &item, Some(&fake_png(1)));
        assert!(failures.is_empty(), "失败: {failures:?}");

        // 两个副本都被改写且内容一致（同内容多路径语义）
        let abs1 = paths.to_absolute(&rel1).unwrap();
        let abs2 = paths.to_absolute(&rel2).unwrap();
        let b1 = std::fs::read(&abs1).unwrap();
        let b2 = std::fs::read(&abs2).unwrap();
        assert_eq!(b1, b2, "两副本内容应一致");
        super::epub::tests::assert_opf_fields(&b1, &item);
    }

    #[test]
    fn skipped_formats_and_missing_files() {
        let tag = "skip";
        let paths = root_for(tag);
        // txt 跳过（文件不存在也不报错）；epub 缺失文件 → 失败记录
        let item = item_with(&["books/x.txt", "books/missing.epub"]);
        let failures = embed_metadata(&paths, &item, None);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, "books/missing.epub");
    }

    #[test]
    fn pdf_roundtrip_via_hook_level() {
        let tag = "pdf";
        let paths = root_for(tag);
        let rel = write_fixture(tag, "doc.pdf", super::pdf::tests::MINIMAL_PDF);
        let item = item_with(&[&rel]);
        let failures = embed_metadata(&paths, &item, None);
        assert!(failures.is_empty(), "失败: {failures:?}");
        super::pdf::tests::assert_info_fields(&paths.to_absolute(&rel).unwrap(), &item);
    }
}
