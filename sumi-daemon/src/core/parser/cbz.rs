//! cbz 解析：zip 枚举图片条目，首张做封面；无目录概念（toc 为空）。

use crate::core::parser::ParsedBook;
use std::io::Read;

/// cbz 内合法图片条目（按名称排序后的首张即封面）
fn image_entries(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Vec<String> {
    let mut names: Vec<String> = archive
        .file_names()
        .filter(|n| {
            !n.ends_with('/')
                && matches!(
                    n.rsplit('.').next().unwrap_or("").to_lowercase().as_str(),
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp"
                )
        })
        .map(|n| n.to_string())
        .collect();
    names.sort();
    names
}

pub fn parse_cbz(name: &str, bytes: &[u8]) -> ParsedBook {
    let mut book = ParsedBook {
        title: name.to_string(),
        ..Default::default()
    };
    if let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
        if let Some(first) = image_entries(&mut archive).into_iter().next() {
            if let Ok(mut file) = archive.by_name(&first) {
                let mut buf = Vec::new();
                if file.read_to_end(&mut buf).is_ok() {
                    book.cover = Some(buf);
                }
            }
        }
    }
    book
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn build_cbz() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let tiny_png: &[u8] = &[
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
            ];
            for n in ["002.png", "001.png", "notes.txt"] {
                zip.start_file(n, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(tiny_png).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn first_image_as_cover() {
        let cbz = build_cbz();
        let book = parse_cbz("漫画", &cbz);
        assert_eq!(book.title, "漫画");
        // 001.png（排序后首张）而非 notes.txt 或 002
        let cover = book.cover.expect("封面缺失");
        assert_eq!(&cover[..4], &[0x89, 0x50, 0x4E, 0x47]);
        assert!(book.toc.is_empty());
    }
}
