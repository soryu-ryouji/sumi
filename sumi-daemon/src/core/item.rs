//! Item 领域模型：内容寻址的书目条目（契约见 API 文档「Item 对象」节）。
//! 内存索引持有 ItemCore（全部持久字段 + 全部文件位置）；DTO 投影（name/ext/size/folders/
//! custom_cover）在 api 层按主路径与文件系统派生。

use serde::{Deserialize, Serialize};

/// 单个文件位置（`[[paths]]`）：path 为库内相对路径，回收站位置以 .sumi/trash/ 前缀表达
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathRecord {
    pub path: String,
    pub size: u64,
    pub modification_time: i64,
}

impl PathRecord {
    pub fn new(path: impl Into<String>, size: u64, modification_time: i64) -> PathRecord {
        PathRecord {
            path: path.into(),
            size,
            modification_time,
        }
    }
}

/// 阅读状态（契约枚举）
pub const READ_STATUS_UNREAD: &str = "unread";
pub const READ_STATUS_READING: &str = "reading";
pub const READ_STATUS_FINISHED: &str = "finished";
pub const READ_STATUS_ABANDONED: &str = "abandoned";

pub fn valid_read_status(s: &str) -> bool {
    matches!(
        s,
        READ_STATUS_UNREAD | READ_STATUS_READING | READ_STATUS_FINISHED | READ_STATUS_ABANDONED
    )
}

/// order_by 白名单（view 偏好与 item/list 共用）
pub const ORDER_FIELDS: &[&str] = &[
    "added_time",
    "modification_time",
    "title",
    "author",
    "size",
    "star",
    "progress",
    "last_read_time",
    "pubdate",
];

pub fn valid_order_field(s: &str) -> bool {
    ORDER_FIELDS.contains(&s)
}

/// Item 持久数据：写向元数据存储（TOML / metadata.db）的完整字段集
#[derive(Clone, Debug)]
pub struct ItemCore {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub publisher: String,
    pub pubdate: String,
    pub isbn: String,
    pub language: String,
    pub series: String,
    pub series_index: f64,
    pub description: String,
    /// 用户显式编辑过的解析字段（refresh_metadata 不覆盖；force 时清除）
    pub overridden_fields: Vec<String>,
    pub read_status: String,
    pub progress: f64,
    pub progress_loc: String,
    pub last_read_time: i64,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub star: i64,
    pub annotation: String,
    pub url: String,
    pub added_time: i64,
    /// 封面尺寸（内容的纯函数，一台计算全平台复用；未提取时为 0）
    pub cover_width: u32,
    pub cover_height: u32,
    /// 全部文件位置；顺序即主路径优先级（api 层投影时优先非回收站位置）
    pub paths: Vec<PathRecord>,
}

impl ItemCore {
    /// 新文件入库的最小条目（解析字段待解析器回填；progress_loc/read_status 默认）
    pub fn new(id: impl Into<String>, paths: Vec<PathRecord>, added_time: i64) -> ItemCore {
        ItemCore {
            id: id.into(),
            title: String::new(),
            authors: Vec::new(),
            publisher: String::new(),
            pubdate: String::new(),
            isbn: String::new(),
            language: String::new(),
            series: String::new(),
            series_index: 0.0,
            description: String::new(),
            overridden_fields: Vec::new(),
            read_status: READ_STATUS_UNREAD.to_string(),
            progress: 0.0,
            progress_loc: String::new(),
            last_read_time: 0,
            tags: Vec::new(),
            categories: Vec::new(),
            star: 0,
            annotation: String::new(),
            url: String::new(),
            added_time,
            cover_width: 0,
            cover_height: 0,
            paths,
        }
    }

    /// 主路径：优先第一个非回收站位置，否则第一个位置
    pub fn primary_path(&self) -> &str {
        self.paths
            .iter()
            .find(|p| !crate::core::paths::LibraryPaths::is_in_trash(&p.path))
            .or_else(|| self.paths.first())
            .map(|p| p.path.as_str())
            .unwrap_or("")
    }

    /// 非回收站位置数（全部位置都在回收站 = 条目在回收站）
    pub fn library_path_count(&self) -> usize {
        self.paths
            .iter()
            .filter(|p| !crate::core::paths::LibraryPaths::is_in_trash(&p.path))
            .count()
    }

    /// 位置维度是否可见于库内（存在任一非回收站位置）
    pub fn has_library_path(&self) -> bool {
        self.library_path_count() > 0
    }

    pub fn mark_overridden(&mut self, field: &str) {
        if !self.overridden_fields.iter().any(|f| f == field) {
            self.overridden_fields.push(field.to_string());
        }
    }

    pub fn is_overridden(&self, field: &str) -> bool {
        self.overridden_fields.iter().any(|f| f == field)
    }
}

/// 解析字段名（overridden_fields 的合法值集合；refresh_metadata 的保护范围）
pub const PARSED_FIELDS: &[&str] = &[
    "title",
    "authors",
    "publisher",
    "pubdate",
    "isbn",
    "language",
    "series",
    "series_index",
    "description",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_path_prefers_library() {
        let mut item = ItemCore::new(
            "abc",
            vec![
                PathRecord::new(".sumi/trash/novels/a.epub", 1, 0),
                PathRecord::new("backup/a.epub", 1, 0),
            ],
            0,
        );
        assert_eq!(item.primary_path(), "backup/a.epub");
        assert!(item.has_library_path());
        item.paths.remove(1);
        assert_eq!(item.primary_path(), ".sumi/trash/novels/a.epub");
        assert!(!item.has_library_path());
    }

    #[test]
    fn override_tracking() {
        let mut item = ItemCore::new("abc", vec![], 0);
        item.mark_overridden("publisher");
        item.mark_overridden("publisher");
        assert!(item.is_overridden("publisher"));
        assert!(!item.is_overridden("title"));
    }
}
