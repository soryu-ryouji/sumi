//! 书库路径工具：负责 `.sumi/` 内部路径、库内相对路径换算与安全校验。
//! 索引与 API 层统一使用正斜杠相对路径（相对书库根目录），如 "novels/三体.epub"。
//! 派生缓存（自动封面/全文索引/归一化内容/元数据镜像）是内容寻址的，位于库外系统缓存目录，
//! 避免库在 iCloud/Dropbox 等同步盘时 `.sumi/` 膨胀拖累同步。

use sha2::{Digest, Sha256};

pub const SUMI_DIR_NAME: &str = ".sumi";
pub const TRASH_DIR_NAME: &str = "trash";
/// 回收站位置的路径前缀（相对库根目录）
pub const TRASH_PREFIX: &str = ".sumi/trash/";

#[derive(Clone)]
pub struct LibraryPaths {
    pub root: String,
    pub sumi_dir: String,
    pub metadata_dir: String,
    /// 自定义封面目录（用户数据，参与同步）
    pub covers_dir: String,
    /// 自动提取封面派生缓存目录（库外系统缓存目录或测试覆盖）
    pub cache_covers_dir: String,
    /// 归一化阅读内容缓存目录（库外，item/content 按需转换产物）
    pub cache_content_dir: String,
    /// SQLite 派生缓存文件：元数据镜像 + FTS5 全文索引（库外系统缓存目录或测试覆盖）
    pub index_db_file: String,
    pub trash_dir: String,
    pub config_file: String,
    /// 数据库模式的权威元数据存储（.sumi/metadata.db）
    pub metadata_db_file: String,
    /// 存储方案标记文件（.sumi/storage_mode）：迁移完成后最后写入，探测时优先于文件存在性探测
    pub storage_mode_file: String,
    pub categories_file: String,
    pub tags_file: String,
    pub view_file: String,
    /// 全局列表隐藏项注册表（.sumi/global_filter.toml，参与同步）
    pub global_filter_file: String,
    /// 文件夹/分类/标签/作者锁注册表（.sumi/locks.toml，参与同步）
    pub locks_file: String,
    /// 缓存目录（自动封面/内容/index.db 所在）：默认 <系统缓存>/sumi/cache/<库名>_<哈希>，
    /// 可经 --cache-parent 指定全局父目录（桌面端设置面板配置）
    pub cache_dir: String,
}

impl LibraryPaths {
    /// cache_parent：缓存父目录覆盖（--cache-parent / 测试指向临时目录）；
    /// None 时用系统缓存目录下的 sumi/cache。库缓存子目录 <库名>_<路径哈希16位> 在其下拼接
    pub fn new(root: &str, cache_parent: Option<String>) -> LibraryPaths {
        let root = canonicalize_root(root);
        let sumi_dir = join_path(&root, SUMI_DIR_NAME);
        let metadata_dir = join_path(&sumi_dir, "metadata");
        let covers_dir = join_path(&sumi_dir, "covers");
        // 缓存父目录同样归一化为正斜杠：与 root 的包含关系校验（cache_location_error）口径一致
        let parent = cache_parent
            .map(|p| full_path(&p))
            .unwrap_or_else(default_cache_parent_sumi);
        let cache_dir = join_path(&parent, &cache_dir_name(&root));
        let cache_covers_dir = join_path(&cache_dir, "covers");
        let cache_content_dir = join_path(&cache_dir, "content");
        let trash_dir = join_path(&sumi_dir, TRASH_DIR_NAME);
        let config_file = join_path(&sumi_dir, "config.toml");
        let metadata_db_file = join_path(&sumi_dir, "metadata.db");
        let storage_mode_file = join_path(&sumi_dir, "storage_mode");
        let categories_file = join_path(&sumi_dir, "categories.toml");
        let tags_file = join_path(&sumi_dir, "tags.toml");
        let view_file = join_path(&sumi_dir, "view.toml");
        let global_filter_file = join_path(&sumi_dir, "global_filter.toml");
        let locks_file = join_path(&sumi_dir, "locks.toml");
        let index_db_file = join_path(&cache_dir, "index.db");
        LibraryPaths {
            root,
            sumi_dir,
            metadata_dir,
            covers_dir,
            cache_covers_dir,
            cache_content_dir,
            index_db_file,
            trash_dir,
            config_file,
            metadata_db_file,
            storage_mode_file,
            categories_file,
            tags_file,
            view_file,
            global_filter_file,
            locks_file,
            cache_dir,
        }
    }

    /// 缓存目录与库根不得互相包含（缓存放进库内会被扫描/监听污染索引；库嵌进缓存目录同理）。
    /// 返回 None 表示位置合法；Some(原因) 供启动期拒绝
    pub fn cache_location_error(&self) -> Option<String> {
        if self.cache_dir == self.root || self.cache_dir.starts_with(&format!("{}/", self.root)) {
            return Some("缓存目录不能位于书库内".to_string());
        }
        if self.root.starts_with(&format!("{}/", self.cache_dir)) {
            return Some("书库不能位于缓存目录内".to_string());
        }
        None
    }

    /// 创建 `.sumi/` 与库外缓存目录结构，并生成排除 trash/ 的 .gitignore（缺失的排除项会补上）
    pub fn ensure_layout(&self) {
        let _ = std::fs::create_dir_all(&self.metadata_dir);
        let _ = std::fs::create_dir_all(&self.covers_dir);
        let _ = std::fs::create_dir_all(&self.cache_covers_dir);
        let _ = std::fs::create_dir_all(&self.cache_content_dir);
        let _ = std::fs::create_dir_all(&self.trash_dir);

        let gitignore = join_path(&self.sumi_dir, ".gitignore");
        let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
        let lines: Vec<&str> = existing.lines().collect();
        if !lines.contains(&"trash/") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&gitignore)
            {
                let _ = writeln!(f, "trash/");
            }
        }
    }

    /// 绝对路径 → 库内相对路径（正斜杠）。库根目录返回 ""，不在库内时返回 None
    pub fn to_relative(&self, abs: &str) -> Option<String> {
        let abs = normalize_separators(abs);
        let rel = strip_prefix_components(&self.root, &abs);
        let rel = rel?;
        if rel.is_empty() {
            return Some(String::new());
        }
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            return Some(String::new());
        }
        if rel.split('/').any(|s| s == "..") {
            return None;
        }
        Some(rel.to_string())
    }

    /// 库内相对路径 → 绝对路径。含越界成分（..、绝对路径）时返回 None
    pub fn to_absolute(&self, rel: &str) -> Option<String> {
        if rel.trim().is_empty() || is_absolute_path(rel) {
            return None;
        }
        let segments: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() || segments.iter().any(|s| *s == "." || *s == "..") {
            return None;
        }
        let abs = full_path(&join_path(&self.root, &segments.join("/")));
        if abs == self.root
            || abs.starts_with(&(self.root.clone() + "/"))
            || abs.starts_with(&(self.root.clone() + "\\"))
        {
            Some(abs)
        } else {
            None
        }
    }

    /// 是否属于 `.sumi/` 内部（回收站除外，回收站参与索引）
    pub fn is_internal(rel: &str) -> bool {
        if rel.starts_with(TRASH_PREFIX) {
            return false;
        }
        rel == SUMI_DIR_NAME || rel.starts_with(&format!("{SUMI_DIR_NAME}/"))
    }

    /// 路径含 `.` 开头组件（回收站位置除外）。.DS_Store、.stfolder 等隐藏文件/目录
    /// 不入索引、不派发解析；已入库的残留位置由 cleanup_index / 消失对账清理
    pub fn is_hidden(rel: &str) -> bool {
        if rel.starts_with(TRASH_PREFIX) {
            return false;
        }
        rel.split('/')
            .filter(|s| !s.is_empty())
            .any(|s| s.starts_with('.'))
    }

    pub fn is_in_trash(rel: &str) -> bool {
        rel.starts_with(TRASH_PREFIX)
    }

    /// 回收站位置 → 原库内路径（去掉 .sumi/trash/ 前缀）
    pub fn trash_to_library_path(rel: &str) -> &str {
        rel.strip_prefix(TRASH_PREFIX).unwrap_or(rel)
    }

    /// 库内路径 → 回收站位置
    pub fn library_to_trash_path(rel: &str) -> String {
        format!("{TRASH_PREFIX}{rel}")
    }

    /// 校验 API 传入的库内相对路径：非空、不指向 `.sumi/` 内部、不含越界成分
    pub fn is_valid_library_path(rel: Option<&str>) -> bool {
        let rel = match rel {
            Some(r) if !r.trim().is_empty() && !r.starts_with('/') && !r.contains('\\') => r,
            _ => return false,
        };
        let segments: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        if segments.iter().any(|s| *s == "." || *s == "..") {
            return false;
        }
        !Self::is_internal(rel)
    }

    /// 取相对路径的父目录部分，根目录返回 ""
    pub fn dir_of(rel: &str) -> &str {
        match rel.rfind('/') {
            Some(i) => &rel[..i],
            None => "",
        }
    }

    /// 文件名（不含扩展名）
    pub fn name_of(rel: &str) -> &str {
        let file_name = &rel[rel.rfind('/').map(|i| i + 1).unwrap_or(0)..];
        match file_name.rfind('.') {
            Some(i) if i > 0 => &file_name[..i],
            _ => file_name,
        }
    }

    /// 扩展名，小写，不含点
    pub fn ext_of(rel: &str) -> String {
        let file_name = &rel[rel.rfind('/').map(|i| i + 1).unwrap_or(0)..];
        match file_name.rfind('.') {
            Some(i) if i > 0 => file_name[i + 1..].to_lowercase(),
            _ => String::new(),
        }
    }
}

/// Unix 毫秒时间戳
pub fn unix_ms(system_time: std::time::SystemTime) -> i64 {
    match system_time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(_) => 0,
    }
}

/// 文件的修改时间（Unix 毫秒）；读取失败返回 0
pub fn file_mtime_ms(path: &str) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(unix_ms)
        .unwrap_or(0)
}

// ---------- 纯路径工具 ----------

/// 规范化分隔符为正斜杠后的字符串比较前缀剥离；逐组件比较，大小写敏感
fn strip_prefix_components(prefix: &str, path: &str) -> Option<String> {
    let prefix = normalize_separators(prefix);
    let path = normalize_separators(path);
    if path == prefix {
        return Some(String::new());
    }
    let p = prefix.trim_end_matches('/');
    if let Some(rest) = path.strip_prefix(p) {
        let rest = rest.trim_start_matches('/');
        if rest.split('/').any(|s| s == "..") {
            return None;
        }
        // 仅当 path 确实以 prefix 的完整组件结尾时才算前缀匹配
        if path.len() > p.len() && path.as_bytes()[p.len()] == b'/' {
            return Some(rest.to_string());
        }
    }
    None
}

fn normalize_separators(p: &str) -> String {
    if std::path::MAIN_SEPARATOR == '\\' {
        p.replace('\\', "/")
    } else {
        p.to_string()
    }
}

fn is_absolute_path(p: &str) -> bool {
    let p = p.replace('\\', "/");
    p.starts_with('/') || p.chars().nth(1) == Some(':')
}

/// root 规范化：full_path 之后解析符号链接到真实路径。
/// 文件监听（macOS FSEvents）报告的是真实路径，root 与之不一致会导致 to_relative 全部失配
/// （经典坑：/tmp 是 /private/tmp 的符号链接）。失败时保留 full_path 结果
fn canonicalize_root(root: &str) -> String {
    let p = full_path(root);
    match std::fs::canonicalize(&p) {
        Ok(real) => full_path(&strip_verbatim_prefix(&real.to_string_lossy())),
        Err(_) => p,
    }
}

/// Windows std::fs::canonicalize 返回 verbatim 路径（`\\?\D:\...`）：该前缀关闭路径归一化，
/// 与正斜杠拼接得到的是 InvalidFilename 非法路径，必须剥回常规形态
/// （`\\?\UNC\server\share` → `\\server\share`）。非 Windows 无此前缀，原样返回
fn strip_verbatim_prefix(p: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(target_os = "windows")]
    {
        if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
            return std::borrow::Cow::Owned(format!(r"\\{rest}"));
        }
        if let Some(rest) = p.strip_prefix(r"\\?\") {
            return std::borrow::Cow::Borrowed(rest);
        }
    }
    std::borrow::Cow::Borrowed(p)
}

/// 简易 GetFullPath：绝对化 + 文本化归约 . 与 .. 组件（不解析符号链接，避免 Windows \\?\ 前缀）
pub fn full_path(p: &str) -> String {
    let p = p.replace('\\', "/");
    let unc = p.starts_with("//"); // UNC（\\server\share）：双斜杠前缀必须保留
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    if unc {
        format!("//{joined}")
    } else if p.starts_with('/') {
        format!("/{joined}")
    } else {
        joined
    }
}

pub fn join_path(base: &str, child: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{child}")
    } else {
        format!("{base}/{child}")
    }
}

/// 路径最后一段组件是否等于 name（按组件比较："my.sumi" 不会误命中 ".sumi"）
pub fn last_component_is(p: &str, name: &str) -> bool {
    p.rsplit('/').next().map(|s| s == name).unwrap_or(false)
}

/// 库外系统缓存目录父级（按平台）
fn default_cache_parent() -> String {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
            full_path(&format!(
                "{}/AppData/Local",
                std::env::var("USERPROFILE").unwrap_or_default()
            ))
        })
    }
    #[cfg(target_os = "macos")]
    {
        format!(
            "{}/Library/Application Support",
            std::env::var("HOME").unwrap_or_default()
        )
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            format!("{}/.local/share", std::env::var("HOME").unwrap_or_default())
        })
    }
}

/// 默认缓存父目录（<系统缓存>/sumi/cache）
fn default_cache_parent_sumi() -> String {
    full_path(&join_path(&default_cache_parent(), "sumi/cache"))
}

/// 缓存子目录名：库文件夹名_路径哈希前16位（小写十六进制）
fn cache_dir_name(root: &str) -> String {
    format!("{}_{}", library_label(root), library_key(root))
}

/// 库文件夹名（缓存目录的可识别前缀）；非法字符清洗、末尾点/空格去除、截断 32 字符，空名兜底 library
fn library_label(root: &str) -> String {
    let trimmed = root.trim_end_matches('/');
    let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if name.trim().is_empty() {
        return "library".to_string();
    }
    let cleaned: String = name
        .chars()
        .map(|c| if is_invalid_file_name_char(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        return "library".to_string();
    }
    cleaned.chars().take(32).collect()
}

fn is_invalid_file_name_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (c.is_control())
}

/// 库标识：根路径的 SHA-256 前 16 位（小写十六进制），保证多库/同名库缓存目录唯一
pub fn library_key(root: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.as_bytes());
    let digest = hasher.finalize();
    let mut s = String::with_capacity(16);
    for b in &digest[..8] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_parent_override_and_location_check() {
        let paths = LibraryPaths::new("/data/library", None);
        assert!(paths.cache_dir.contains("sumi/cache/library_"));
        assert!(paths.cache_location_error().is_none());

        let paths = LibraryPaths::new("/data/library", Some("/fast/cache".to_string()));
        assert!(paths.cache_dir.starts_with("/fast/cache/library_"));
        assert_eq!(
            paths.cache_covers_dir,
            format!("{}/covers", paths.cache_dir)
        );
        assert_eq!(paths.index_db_file, format!("{}/index.db", paths.cache_dir));
        assert!(paths.cache_location_error().is_none());

        let bad = LibraryPaths::new("/data/library", Some("/data/library/cache".to_string()));
        assert!(bad.cache_location_error().is_some());
        let bad = LibraryPaths::new("/data/library", Some("/data/library".to_string()));
        assert!(bad.cache_location_error().is_some());
        let ok = LibraryPaths::new("/data/cache/library", Some("/data/cache".to_string()));
        assert!(ok.cache_location_error().is_none());
    }

    #[test]
    fn relative_roundtrip() {
        let paths = LibraryPaths::new("/data/library", Some("/tmp/cache".to_string()));
        assert_eq!(paths.to_relative("/data/library"), Some(String::new()));
        assert_eq!(
            paths.to_relative("/data/library/novels/三体.epub"),
            Some("novels/三体.epub".to_string())
        );
        assert_eq!(paths.to_relative("/data/other/三体.epub"), None);
        assert_eq!(
            paths.to_absolute("novels/三体.epub"),
            Some("/data/library/novels/三体.epub".to_string())
        );
        assert_eq!(paths.to_absolute("../escape.epub"), None);
        assert_eq!(paths.to_absolute("/abs/三体.epub"), None);
        assert_eq!(paths.to_absolute("a/./b.epub"), None);
    }

    #[test]
    fn trash_helpers() {
        assert!(LibraryPaths::is_in_trash(".sumi/trash/novels/三体.epub"));
        assert!(!LibraryPaths::is_internal(".sumi/trash/novels/三体.epub"));
        assert!(LibraryPaths::is_internal(".sumi/metadata/x.toml"));
        assert_eq!(
            LibraryPaths::trash_to_library_path(".sumi/trash/novels/三体.epub"),
            "novels/三体.epub"
        );
        assert_eq!(
            LibraryPaths::library_to_trash_path("novels/三体.epub"),
            ".sumi/trash/novels/三体.epub"
        );
    }

    #[test]
    fn name_and_ext() {
        assert_eq!(LibraryPaths::name_of("novels/三体.epub"), "三体");
        assert_eq!(LibraryPaths::ext_of("novels/三体.EPUB"), "epub");
        assert_eq!(LibraryPaths::ext_of("noext"), "");
        assert_eq!(LibraryPaths::name_of(".hidden"), ".hidden");
        assert_eq!(LibraryPaths::dir_of("novels/科幻/三体.epub"), "novels/科幻");
        assert_eq!(LibraryPaths::dir_of("三体.epub"), "");
    }

    #[test]
    fn full_path_normalizes() {
        assert_eq!(full_path(r"D:\lib\..\lib2"), "D:/lib2");
        assert_eq!(full_path("/data/./lib"), "/data/lib");
        // UNC（\\server\share）双斜杠前缀必须保留，折叠成单斜杠会变成本地路径
        assert_eq!(full_path(r"\\NAS\books\科幻"), "//NAS/books/科幻");
        assert_eq!(full_path("//NAS/books"), "//NAS/books");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn verbatim_prefix_stripped() {
        assert_eq!(strip_verbatim_prefix(r"\\?\D:\lib").as_ref(), r"D:\lib");
        assert_eq!(strip_verbatim_prefix(r"\\?\UNC\nas\books").as_ref(), r"\\nas\books");
        assert_eq!(strip_verbatim_prefix(r"D:\lib").as_ref(), r"D:\lib");
    }

    /// Windows 回归：canonicalize 的 \\\?\\ 前缀曾让 to_absolute 全部失配、拼接路径非法
    #[cfg(target_os = "windows")]
    #[test]
    fn canonicalized_root_roundtrips() {
        let dir = std::env::temp_dir().join(format!("sumi-canon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = LibraryPaths::new(dir.to_str().unwrap(), Some("C:/sumi-test-cache".to_string()));
        assert!(!paths.root.starts_with(r"\\?\"), "root 带 verbatim 前缀: {}", paths.root);
        let abs = paths.to_absolute("novels/x.epub").expect("to_absolute 失配");
        assert_eq!(paths.to_relative(&abs), Some("novels/x.epub".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn last_component_matching() {
        assert!(last_component_is("/lib/.sumi", ".sumi"));
        assert!(!last_component_is("/lib/my.sumi", ".sumi"));
        assert!(!last_component_is("/lib/.sumi/trash", ".sumi"));
        assert!(last_component_is(".sumi", ".sumi"));
    }
}
