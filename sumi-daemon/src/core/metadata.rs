//! 配置文件模式的元数据 TOML：`.sumi/metadata/<hash>.toml` 的解析与手写序列化。
//! 输出格式精确可控（标量在前、`[[paths]]` 在后），与 storage.md 的示例逐字段对应。
//! 原子写（临时文件 + rename）；网盘同步冲突副本（非 `<hash>.toml` 命名）由列举方忽略。

use crate::core::item::{ItemCore, PathRecord};
use std::path::Path;

#[derive(serde::Deserialize)]
struct RawItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    publisher: String,
    #[serde(default)]
    pubdate: String,
    #[serde(default)]
    isbn: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    series: String,
    #[serde(default)]
    series_index: f64,
    #[serde(default)]
    description: String,
    #[serde(default)]
    overridden_fields: Vec<String>,
    #[serde(default)]
    read_status: String,
    #[serde(default)]
    progress: f64,
    #[serde(default)]
    progress_loc: String,
    #[serde(default)]
    last_read_time: i64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    star: i64,
    #[serde(default)]
    annotation: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    added_time: i64,
    #[serde(default)]
    cover_width: u32,
    #[serde(default)]
    cover_height: u32,
    #[serde(default)]
    paths: Vec<PathRecord>,
}

/// 解析一个元数据 TOML 文本。id 由文件名承载（调用方注入），解析失败返回 Err
pub fn parse_toml(id: &str, text: &str) -> Result<ItemCore, String> {
    let raw: RawItem =
        toml::from_str(text).map_err(|e| format!("metadata {id} 解析失败: {e}"))?;
    let read_status = if crate::core::item::valid_read_status(&raw.read_status) {
        raw.read_status
    } else {
        crate::core::item::READ_STATUS_UNREAD.to_string()
    };
    Ok(ItemCore {
        id: id.to_string(),
        title: raw.title,
        authors: raw.authors,
        publisher: raw.publisher,
        pubdate: raw.pubdate,
        isbn: raw.isbn,
        language: raw.language,
        series: raw.series,
        series_index: raw.series_index,
        description: raw.description,
        overridden_fields: raw.overridden_fields,
        read_status,
        progress: raw.progress.clamp(0.0, 100.0),
        progress_loc: raw.progress_loc,
        last_read_time: raw.last_read_time,
        tags: raw.tags,
        categories: raw.categories,
        star: raw.star.clamp(0, 5),
        annotation: raw.annotation,
        url: raw.url,
        added_time: raw.added_time,
        cover_width: raw.cover_width,
        cover_height: raw.cover_height,
        paths: raw.paths,
    })
}

/// 序列化为元数据 TOML 文本（字段顺序固定：标量在前、`[[paths]]` 在后）
pub fn to_toml(item: &ItemCore) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("# 书目元数据：解析字段由文件自动填充；overridden_fields 记录用户编辑，refresh_metadata 不覆盖\n");

    let esc = |s: &str| -> String {
        toml::Value::String(s.to_string()).to_string()
    };

    out.push_str(&format!("title = {}\n", esc(&item.title)));
    out.push_str(&format!("authors = {}\n", string_array(&item.authors)));
    out.push_str(&format!("publisher = {}\n", esc(&item.publisher)));
    out.push_str(&format!("pubdate = {}\n", esc(&item.pubdate)));
    out.push_str(&format!("isbn = {}\n", esc(&item.isbn)));
    out.push_str(&format!("language = {}\n", esc(&item.language)));
    out.push_str(&format!("series = {}\n", esc(&item.series)));
    out.push_str(&format!("series_index = {}\n", fmt_f64(item.series_index)));
    out.push_str(&format!("description = {}\n", esc(&item.description)));
    out.push_str(&format!("overridden_fields = {}\n", string_array(&item.overridden_fields)));
    out.push('\n');
    out.push_str(&format!("read_status = {}\n", esc(&item.read_status)));
    out.push_str(&format!("progress = {}\n", fmt_f64(item.progress)));
    out.push_str(&format!("progress_loc = {}\n", esc(&item.progress_loc)));
    out.push_str(&format!("last_read_time = {}\n", item.last_read_time));
    out.push('\n');
    out.push_str(&format!("tags = {}\n", string_array(&item.tags)));
    out.push_str(&format!("categories = {}\n", string_array(&item.categories)));
    out.push_str(&format!("star = {}\n", item.star));
    out.push_str(&format!("annotation = {}\n", esc(&item.annotation)));
    out.push_str(&format!("url = {}\n", esc(&item.url)));
    out.push_str(&format!("added_time = {}\n", item.added_time));
    out.push('\n');
    out.push_str("# 派生信息（内容的纯函数，一台计算全平台复用）\n");
    out.push_str(&format!("cover_width = {}\n", item.cover_width));
    out.push_str(&format!("cover_height = {}\n", item.cover_height));
    out.push('\n');

    out.push_str("# 文件位置：相同内容的文件共享一个 item，可有多条\n");
    for p in &item.paths {
        out.push_str("\n[[paths]]\n");
        out.push_str(&format!("path = {}\n", esc(&p.path)));
        out.push_str(&format!("size = {}\n", p.size));
        out.push_str(&format!("modification_time = {}\n", p.modification_time));
    }
    out
}

fn string_array(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| toml::Value::String(s.clone()).to_string()).collect();
    format!("[{}]", inner.join(", "))
}

fn fmt_f64(v: f64) -> String {
    // 整数值省去小数点，保持 TOML 可读
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// 元数据目录中合法文件名：<64 位 hex>.toml；同步冲突副本（如 *.sync-conflict-*.toml）忽略
pub fn is_metadata_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".toml") else {
        return false;
    };
    stem.len() == 64 && stem.chars().all(|c| c.is_ascii_hexdigit())
}

/// 原子写元数据文件（临时文件 + rename）
pub fn write_metadata_file(path: &str, item: &ItemCore) -> std::io::Result<()> {
    crate::core::config::atomic_write(path, to_toml(item).as_bytes())
}

/// 读取单个元数据文件；文件不存在返回 None
pub fn read_metadata_file(path: &str, id: &str) -> Result<Option<ItemCore>, String> {
    if !Path::new(path).exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    parse_toml(id, &text).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ItemCore {
        let mut item = ItemCore::new(
            "9b1f2c",
            vec![PathRecord::new("novels/三体.epub", 1245760, 1690000000000)],
            1690000000000,
        );
        item.title = "三体".into();
        item.authors = vec!["刘慈欣".into()];
        item.publisher = "重庆出版社".into();
        item.pubdate = "2008-01-01".into();
        item.isbn = "9787536692930".into();
        item.language = "zh".into();
        item.series = "地球往事".into();
        item.series_index = 1.0;
        item.read_status = "finished".into();
        item.progress = 100.0;
        item.progress_loc = "epubcfi(/6/24!/4/2/1:0)".into();
        item.last_read_time = 1700000000000;
        item.tags = vec!["科幻".into()];
        item.categories = vec!["小说".into()];
        item.star = 5;
        item.cover_width = 600;
        item.cover_height = 900;
        item.overridden_fields = vec!["publisher".into()];
        item
    }

    #[test]
    fn roundtrip() {
        let item = sample();
        let text = to_toml(&item);
        let parsed = parse_toml("9b1f2c", &text).expect("解析失败");
        assert_eq!(parsed.id, "9b1f2c");
        assert_eq!(parsed.title, "三体");
        assert_eq!(parsed.authors, vec!["刘慈欣".to_string()]);
        assert_eq!(parsed.publisher, "重庆出版社");
        assert_eq!(parsed.series_index, 1.0);
        assert_eq!(parsed.progress, 100.0);
        assert_eq!(parsed.tags, vec!["科幻".to_string()]);
        assert_eq!(parsed.star, 5);
        assert_eq!(parsed.cover_width, 600);
        assert_eq!(parsed.paths.len(), 1);
        assert_eq!(parsed.paths[0].path, "novels/三体.epub");
        assert_eq!(parsed.overridden_fields, vec!["publisher".to_string()]);
    }

    #[test]
    fn clamps_and_defaults() {
        let text = "read_status = \"bogus\"\nprogress = 150\nstar = 9\n";
        let parsed = parse_toml("abc", text).unwrap();
        assert_eq!(parsed.read_status, "unread");
        assert_eq!(parsed.progress, 100.0);
        assert_eq!(parsed.star, 5);
    }

    #[test]
    fn file_name_filter() {
        assert!(is_metadata_file_name(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.toml"
        ));
        assert!(!is_metadata_file_name(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.sync-conflict-20250101.toml"
        ));
        assert!(!is_metadata_file_name("readme.md"));
        assert!(!is_metadata_file_name("short.toml"));
    }
}
