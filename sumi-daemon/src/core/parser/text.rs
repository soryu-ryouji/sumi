//! txt / md 解析：编码探测归一 UTF-8（GBK / Big5 / Shift_JIS / UTF-16 等，中文网文刚需）、
//! front matter 书名提取（md）、章节模式启发式目录（`第N章` / `Chapter N` 等）。

use crate::core::parser::{ParsedBook, TocEntry};

/// 探测编码并解码为 UTF-8。探测失败或纯 ASCII 时原样返回（UTF-8 兼容）
pub fn decode_to_utf8(bytes: &[u8]) -> String {
    // BOM 快路径
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE.decode(&bytes[2..]).0.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE.decode(&bytes[2..]).0.into_owned();
    }
    // UTF-8 优先（合法 UTF-8 且含多字节序列时直接采用）
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);
    encoding.decode(bytes).0.into_owned()
}

/// md front matter（`---` YAML 头）中的 title 提取；无 front matter 返回 None
pub fn markdown_title(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let front = &rest[..end];
    for line in front.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("title:") {
            let value = value.trim().trim_matches(['"', '\'']).trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 按行迭代（保留 \r，行宽含换行符）：
/// (行内容不含 \n, 行首字符偏移, 该行含换行的字符宽度)。
/// \r 保留在行内容里参与计数——CRLF 文件的 \r 也是内容字符（锚点偏移按原始文本计）
fn lines_with_offsets(text: &str) -> impl Iterator<Item = (&str, usize, usize)> {
    let mut rest = text;
    let mut offset = 0usize;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (line, next) = match rest.find('\n') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        let start = offset;
        let width = line.chars().count() + 1; // +1 = \n（\r 已含在 line 内）
        offset += width;
        rest = next;
        Some((line, start, width))
    })
}

/// txt 章节模式启发式（识别失败由调用方回退单节点）；锚点为原始文本的字符偏移
pub fn txt_toc(text: &str, max_entries: usize) -> Vec<TocEntry> {
    let mut entries = Vec::new();
    for (line, start, _) in lines_with_offsets(text) {
        if is_chapter_heading(line) {
            entries.push(TocEntry {
                title: line.trim().to_string(),
                anchor: format!("anchor:char-{start}"),
                children: Vec::new(),
            });
            if entries.len() >= max_entries {
                break;
            }
        }
    }
    entries
}

/// 章节标题判定：`第N章/节/卷/回`、`Chapter N`、`序章/前言/后记/尾声` 等常见模式
fn is_chapter_heading(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() || line.chars().count() > 40 {
        return false;
    }
    // 行首模式：第 + 数字/汉字数字 + 章/节/卷/回/部
    if line.starts_with("第") {
        let rest = &line[3..];
        let digits_len = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || "零一二三四五六七八九十百千万两".contains(*c))
            .count();
        if digits_len > 0 {
            let after = rest.chars().nth(digits_len);
            return matches!(after, Some(c) if "章节卷回部篇".contains(c));
        }
    }
    // Chapter N（大小写不敏感）
    let lower = line.to_lowercase();
    if lower.starts_with("chapter") {
        let rest = lower[7..].trim();
        return rest.chars().next().is_some_and(|c| c.is_ascii_digit());
    }
    // 特殊章节名（整行精确匹配）
    matches!(line, "序章" | "序言" | "前言" | "楔子" | "后记" | "尾声" | "终章" | "番外")
}

/// txt/md 解析入口
pub fn parse_text(name: &str, ext: &str, bytes: &[u8]) -> ParsedBook {
    let text = decode_to_utf8(bytes);
    let title = match ext {
        "md" => markdown_title(&text),
        _ => None,
    }
    .unwrap_or_else(|| name.to_string());
    ParsedBook {
        title,
        authors: Vec::new(),
        publisher: String::new(),
        pubdate: String::new(),
        isbn: String::new(),
        language: String::new(),
        series: String::new(),
        series_index: 0.0,
        description: String::new(),
        cover: None,
        toc: match ext {
            "md" => md_toc(&text),
            _ => txt_toc(&text, 2000),
        },
        fulltext: Some(text),
    }
}

/// md 按 `#` 标题层级建目录（展平收集 + 索引栈建树）；锚点为原始文本的字符偏移
fn md_toc(text: &str) -> Vec<TocEntry> {
    // 1. 展平收集 (level, title, anchor)
    let mut flat: Vec<(u8, TocEntry)> = Vec::new();
    for (line, start, _) in lines_with_offsets(text) {
        let trimmed = line.trim_start();
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            let title = trimmed[level..].trim().to_string();
            let anchor = format!("anchor:char-{start}");
            flat.push((level as u8, TocEntry { title, anchor, children: Vec::new() }));
        }
    }
    // 2. 递归下降建树（flat 数组小，性能无忧）
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gbk_decoding() {
        // 「三体 黑暗森林」的 GBK 编码
        let gbk = encoding_rs::GBK.encode("三体 黑暗森林").0.into_owned();
        let decoded = decode_to_utf8(&gbk);
        assert_eq!(decoded, "三体 黑暗森林");
    }

    #[test]
    fn utf8_and_bom_passthrough() {
        assert_eq!(decode_to_utf8("纯 UTF-8".as_bytes()), "纯 UTF-8");
        let bom = [0xEF, 0xBB, 0xBF].iter().chain("带 BOM".as_bytes().iter()).cloned().collect::<Vec<u8>>();
        assert_eq!(decode_to_utf8(&bom), "带 BOM");
        let mut utf16: Vec<u8> = vec![0xFF, 0xFE];
        utf16.extend("UTF16 中文".encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(decode_to_utf8(&utf16), "UTF16 中文");
    }

    #[test]
    fn chapter_headings() {
        assert!(is_chapter_heading("第一章 科学边界"));
        assert!(is_chapter_heading("第1234章"));
        assert!(is_chapter_heading("第十二章 诅咒"));
        assert!(is_chapter_heading("Chapter 5 The Discovery"));
        assert!(is_chapter_heading("序章"));
        assert!(!is_chapter_heading("第二次世界大战史")); // 「二」后是「次」非章节字
        assert!(!is_chapter_heading("普通文本行"));
        assert!(!is_chapter_heading("这一行太长了，长得不像章节标题，因为章节标题一般都很短"));
    }

    #[test]
    fn txt_toc_anchors() {
        let text = "开头\n第一章 起点\n正文...\n第二章 转折\n结尾";
        let toc = txt_toc(text, 100);
        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].title, "第一章 起点");
        assert_eq!(toc[0].anchor, "anchor:char-3"); // 「开头\n」= 3 字符
        assert_eq!(toc[1].title, "第二章 转折");
    }

    #[test]
    fn txt_toc_anchors_crlf() {
        // CRLF：\r 也是内容字符，锚点按原始文本字符偏移（不再漂移）
        let text = "开头\r\n第一章 起点\r\n正文";
        let toc = txt_toc(text, 100);
        assert_eq!(toc.len(), 1);
        assert_eq!(toc[0].anchor, "anchor:char-4"); // 「开头\r\n」= 4 字符
        // 标题不含尾部 \r
        assert_eq!(toc[0].title, "第一章 起点");
    }

    #[test]
    fn md_toc_anchors_crlf() {
        let md = "# 一级\r\n正文\r\n## 二级\r\n";
        let toc = md_toc(md);
        assert_eq!(toc.len(), 1);
        assert_eq!(toc[0].title, "一级");
        assert_eq!(toc[0].children[0].title, "二级");
        assert_eq!(toc[0].children[0].anchor, "anchor:char-10"); // 4+2+4=10
    }

    #[test]
    fn markdown_front_matter() {
        let md = "---\ntitle: 我的书\nauthor: 无效字段\n---\n\n# 章节\n正文";
        assert_eq!(markdown_title(md).as_deref(), Some("我的书"));
        assert!(markdown_title("# 无 front matter").is_none());
    }

    #[test]
    fn md_toc_hierarchy() {
        let md = "# 一级\n正文\n## 二级A\n### 三级\n## 二级B\n# 又一级";
        let toc = md_toc(md);
        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].title, "一级");
        assert_eq!(toc[0].children.len(), 2);
        assert_eq!(toc[0].children[0].title, "二级A");
        assert_eq!(toc[0].children[0].children.len(), 1);
        assert_eq!(toc[1].title, "又一级");
    }
}
