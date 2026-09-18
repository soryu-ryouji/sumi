//! 库级配置（`.sumi/config.toml`）：模板生成、解析、快照读、带注释写回。
//! 首次打开自动生成默认模板（已存在绝不覆盖）；解析错误保留上次有效配置继续运行
//! （config_error 上报，修复后自动清除）；写回经 toml_edit 就地改键值保注释 + 原子写。

use std::path::Path;
use std::sync::RwLock;

/// v1 支持格式全集（可见扩展名白名单缺省值）
pub const SUPPORTED_EXTS: &[&str] = &[
    "epub", "pdf", "txt", "md", "mobi", "azw3", "docx", "cbz",
];

pub const DEFAULT_WEB_PORT: u16 = 27382;
pub const DEFAULT_SCAN_INTERVAL_SECS: u64 = 900;

/// `[web]` 局域网 web 查看配置（token 三档权限见 storage.md「项目配置」）
#[derive(Clone, Debug, PartialEq)]
pub struct WebConfig {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    pub writable: bool,
    pub separate_write_token: bool,
    pub write_token: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        WebConfig {
            enabled: false,
            port: DEFAULT_WEB_PORT,
            token: String::new(),
            writable: false,
            separate_write_token: false,
            write_token: String::new(),
        }
    }
}

/// `[scan]` 周期兜底重扫配置
#[derive(Clone, Debug, PartialEq)]
pub struct ScanConfig {
    pub periodic: bool,
    pub interval: u64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        ScanConfig {
            periodic: true,
            interval: DEFAULT_SCAN_INTERVAL_SECS,
        }
    }
}

/// config.toml 的内存快照
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryConfigSnapshot {
    pub name: Option<String>,
    pub ignore: Vec<String>,
    /// None = 缺省（v1 支持格式全集）；Some = 用户显式配置
    pub extensions: Option<Vec<String>>,
    /// 打开方式：扩展名（小写，无点）→ 指定应用（.app 路径/应用名/可执行文件路径）；
    /// 未配置的扩展名走系统默认应用
    pub openers: std::collections::HashMap<String, String>,
    pub scan: ScanConfig,
    pub web: WebConfig,
    /// 保存时自动回写 EPUB/PDF 文件元数据（缺省开启；不能用 derive Default —— bool 默认 false）
    pub embed_metadata: bool,
}

impl Default for LibraryConfigSnapshot {
    fn default() -> Self {
        LibraryConfigSnapshot {
            name: None,
            ignore: Vec::new(),
            extensions: None,
            openers: std::collections::HashMap::new(),
            scan: ScanConfig::default(),
            web: WebConfig::default(),
            embed_metadata: true,
        }
    }
}

impl LibraryConfigSnapshot {
    /// 可见扩展名集合（Some 展开，None 用全集）
    pub fn extension_set(&self) -> std::collections::HashSet<String> {
        match &self.extensions {
            Some(exts) => exts.iter().map(|e| e.to_lowercase()).collect(),
            None => SUPPORTED_EXTS.iter().map(|e| e.to_string()).collect(),
        }
    }

    /// ignore 规则命中判定（当前实现：路径任一段精确匹配或通配符后缀，
    /// 与 hawk 同语义——glob 完整实现留待需要时增强，行为已在文档标注）
    pub fn matches_ignore(&self, rel: &str) -> bool {
        for pattern in &self.ignore {
            if pattern.is_empty() {
                continue;
            }
            if let Some(suffix) = pattern.strip_prefix('*') {
                if rel.ends_with(suffix) {
                    return true;
                }
            } else {
                // 段级精确匹配：pattern 命中路径任一段（如 node_modules）
                if rel.split('/').any(|seg| seg == pattern) {
                    return true;
                }
                // 整路径精确匹配
                if rel == pattern {
                    return true;
                }
            }
        }
        false
    }
}

/// 配置持有者：文件 → 快照的装载与热重载（文件监听触发 reload）
pub struct LibraryConfig {
    path: String,
    inner: RwLock<LibraryConfigSnapshot>,
    /// 解析错误（保留上次有效配置继续运行；正常为 None）
    error: RwLock<Option<String>>,
}

const CONFIG_TEMPLATE: &str = r#"# sumi 书库配置

# 书库名（界面显示用；缺省为库目录名）
# name = "我的书库"

# 索引时忽略的路径
ignore = []

# 可见扩展名白名单：只索引这些后缀的文件
# （缺省为 v1 支持格式全集：epub/pdf/txt/md/mobi/azw3/docx/cbz）
# extensions = ["epub", "pdf", "txt", "md"]

# 打开方式：按扩展名指定打开应用（不配置 = 系统默认应用）。
# macOS 支持 .app 路径（/Applications/Calibre.app）、应用名（Calibre）与可执行文件路径；
# Windows/Linux 填可执行文件路径
# [openers]
# pdf = "/Applications/Adobe Acrobat Reader.app"
# epub = "Calibre"

# 保存时自动回写 EPUB/PDF 文件元数据（书名/作者/封面等写进原文件；网盘同步场景可关）
# embed_metadata = true

# 周期兜底重扫（监听漏事件的最终一致保证）
[scan]
periodic = true
interval = 900

# 局域网 web 书库查看
[web]
enabled = false      # 开启后 server 追加监听 0.0.0.0:<port>，并托管前端页面
port = 27382
token = ""           # viewer token；浏览器打开 http://<电脑IP>:<port> 后输入
writable = false     # 允许写：开启后查看端可上传/删除/修改（与桌面端同等操作），请谨慎授权
separate_write_token = false  # 拆分只读/可写 token：token 降为只读，write_token 可写（不拆分时 token 读写兼具）
write_token = ""     # 拆分模式下的可写 token
"#;

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    #[serde(default)]
    openers: std::collections::HashMap<String, String>,
    #[serde(default)]
    scan: RawScan,
    #[serde(default)]
    web: RawWeb,
    #[serde(default = "default_true")]
    embed_metadata: bool,
}

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawScan {
    #[serde(default = "default_true")]
    periodic: bool,
    #[serde(default = "default_scan_interval")]
    interval: u64,
}

fn default_true() -> bool {
    true
}

fn default_scan_interval() -> u64 {
    DEFAULT_SCAN_INTERVAL_SECS
}

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawWeb {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_web_port")]
    port: u16,
    #[serde(default)]
    token: String,
    #[serde(default)]
    writable: bool,
    #[serde(default)]
    separate_write_token: bool,
    #[serde(default)]
    write_token: String,
}

fn default_web_port() -> u16 {
    DEFAULT_WEB_PORT
}

impl LibraryConfig {
    /// 装载配置：文件缺失时生成默认模板；解析失败保留默认值并记录错误（不 panic）
    pub fn load(path: &str) -> LibraryConfig {
        if !Path::new(path).exists() {
            if let Some(parent) = Path::new(path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, CONFIG_TEMPLATE);
        }

        let config = LibraryConfig {
            path: path.to_string(),
            inner: RwLock::new(LibraryConfigSnapshot::default()),
            error: RwLock::new(None),
        };
        config.reload();
        config
    }

    /// 重新解析文件（文件监听重载入口）。解析失败保留上次有效快照，错误置入 config_error
    pub fn reload(&self) {
        match self.parse_file() {
            Ok(snapshot) => {
                *self.inner.write().unwrap() = snapshot;
                *self.error.write().unwrap() = None;
            }
            Err(message) => {
                // 文件不存在 = 用户删除，回退默认（保持运行）
                let missing = !Path::new(&self.path).exists();
                if missing {
                    *self.inner.write().unwrap() = LibraryConfigSnapshot::default();
                    *self.error.write().unwrap() = None;
                } else {
                    *self.error.write().unwrap() = Some(message);
                    tracing::warn!("config.toml 解析失败，保留上次有效配置: {}", self.path);
                }
            }
        }
    }

    fn parse_file(&self) -> Result<LibraryConfigSnapshot, String> {
        let text = std::fs::read_to_string(&self.path).map_err(|e| format!("读取失败: {e}"))?;
        let raw: RawConfig = toml::from_str(&text).map_err(|e| format!("解析失败: {e}"))?;
        let scan = ScanConfig {
            periodic: raw.scan.periodic,
            interval: raw.scan.interval.max(60),
        };
        let web = WebConfig {
            enabled: raw.web.enabled,
            port: raw.web.port,
            token: raw.web.token,
            writable: raw.web.writable,
            separate_write_token: raw.web.separate_write_token,
            write_token: raw.web.write_token,
        };
        // 扩展名键规范化为小写无点（配置里写 PDF / .pdf 都能命中）
        let openers: std::collections::HashMap<String, String> = raw
            .openers
            .into_iter()
            .filter_map(|(k, v)| {
                let key = k.trim().trim_start_matches('.').to_lowercase();
                let value = v.trim().to_string();
                if key.is_empty() || value.is_empty() {
                    None
                } else {
                    Some((key, value))
                }
            })
            .collect();
        Ok(LibraryConfigSnapshot {
            name: raw.name,
            ignore: raw.ignore,
            extensions: raw.extensions,
            openers,
            scan,
            web,
            embed_metadata: raw.embed_metadata,
        })
    }

    /// 当前快照（读路径每请求调用，配置热生效）
    pub fn current(&self) -> LibraryConfigSnapshot {
        self.inner.read().unwrap().clone()
    }

    /// 解析错误（app/status 的 config_error）；正常为 None
    pub fn config_error(&self) -> Option<String> {
        self.error.read().unwrap().clone()
    }

    /// 库显示名：config.name 优先，缺省为库目录名
    pub fn display_name(&self, library_root: &str) -> String {
        let snapshot = self.current();
        match snapshot.name {
            Some(name) if !name.trim().is_empty() => name,
            _ => crate::core::paths::LibraryPaths::name_of(
                library_root.trim_end_matches('/'),
            )
            .to_string(),
        }
    }

    /// 就地修改 config.toml 的顶层键（toml_edit 保注释，原子写），成功后热重载。
    /// 修改器操作 toml_edit::DocumentMut，返回 Err 时放弃写盘
    pub fn edit(
        &self,
        edit: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<(), String>,
    ) -> Result<(), String> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut doc: toml_edit::DocumentMut =
            text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("解析失败: {e}"))?;
        edit(&mut doc)?;
        let new_text = doc.to_string();
        atomic_write(&self.path, new_text.as_bytes()).map_err(|e| format!("写入失败: {e}"))?;
        self.reload();
        if let Some(err) = self.config_error() {
            return Err(format!("写回后校验失败: {err}"));
        }
        Ok(())
    }
}

/// 临时文件 + rename 的原子写（网盘同步不走写了一半的文件）
pub fn atomic_write(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = format!("{path}.tmp");
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parse_roundtrip() {
        // 生成的默认模板必须能被解析回默认快照
        let raw: RawConfig = toml::from_str(CONFIG_TEMPLATE).expect("模板解析失败");
        assert!(raw.name.is_none());
        assert!(raw.ignore.is_empty());
        assert!(raw.extensions.is_none());
        assert!(raw.scan.periodic);
        assert_eq!(raw.scan.interval, 900);
        assert!(!raw.web.enabled);
        assert_eq!(raw.web.port, 27382);
    }

    #[test]
    fn embed_metadata_config_roundtrip() {
        // 缺省开启；显式关闭经 edit 持久化后仍生效
        let dir = std::env::temp_dir().join(format!("sumi-embed-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let cfg = LibraryConfig::load(path.to_str().unwrap());
        assert!(cfg.current().embed_metadata, "缺省应开启");
        cfg.edit(|doc| {
            doc["embed_metadata"] = toml_edit::value(false);
            Ok(())
        })
        .unwrap();
        assert!(!cfg.current().embed_metadata, "关闭应持久化");
    }

    #[test]
    fn extension_set_defaults() {
        let snapshot = LibraryConfigSnapshot::default();
        let set = snapshot.extension_set();
        assert!(set.contains("epub") && set.contains("cbz"));
        assert!(!set.contains("doc"));

        let custom = LibraryConfigSnapshot {
            extensions: Some(vec!["epub".to_string()]),
            ..Default::default()
        };
        let set = custom.extension_set();
        assert!(set.contains("epub") && !set.contains("pdf"));
    }

    #[test]
    fn ignore_matching() {
        let snapshot = LibraryConfigSnapshot {
            ignore: vec![
                "node_modules".to_string(),
                "*.tmp".to_string(),
                "drafts/wip.epub".to_string(),
            ],
            ..Default::default()
        };
        assert!(snapshot.matches_ignore("books/node_modules/x.epub"));
        assert!(snapshot.matches_ignore("a/b.epub.tmp"));
        assert!(snapshot.matches_ignore("drafts/wip.epub"));
        assert!(!snapshot.matches_ignore("books/node.epub"));
        assert!(!snapshot.matches_ignore("a/tmp"));
    }

    #[test]
    fn bad_file_keeps_last_good() {
        let dir = std::env::temp_dir().join(format!("sumi-cfg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        let path_str = path.to_str().unwrap();

        let config = LibraryConfig::load(path_str);
        assert!(config.config_error().is_none());
        assert!(config.current().scan.periodic);

        // 写入非法内容：保留上次有效配置 + config_error 上报
        std::fs::write(&path, "not = [valid").unwrap();
        config.reload();
        assert!(config.config_error().is_some());
        assert!(config.current().scan.periodic);

        // 修复后自动清除
        std::fs::write(&path, "name = \"修好\"\n").unwrap();
        config.reload();
        assert!(config.config_error().is_none());
        assert_eq!(config.current().name.as_deref(), Some("修好"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
