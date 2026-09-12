//! 注册表文件族：categories.toml / tags.toml（表数组注册表）、view.toml（排序偏好）、
//! global_filter.toml（四维隐藏集）、locks.toml（四维锁，Argon2id 密码哈希）。
//! 全部参与同步、原子写；文件监听重载由 watcher 接线（reload 入口统一为 load）。

use crate::core::config::atomic_write;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

// ---------- 表数组注册表（categories / tags：[[entry]] name = "..."） ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedEntry {
    pub name: String,
}

/// 名称注册表：空名预创建 + item 赋值自动登记的并集存储
pub struct NameRegistry {
    path: String,
    inner: RwLock<Vec<NamedEntry>>,
}

impl NameRegistry {
    pub fn load(path: &str) -> NameRegistry {
        let entries = Self::parse_file(path).unwrap_or_default();
        NameRegistry {
            path: path.to_string(),
            inner: RwLock::new(entries),
        }
    }

    fn parse_file(path: &str) -> Option<Vec<NamedEntry>> {
        let text = std::fs::read_to_string(path).ok()?;
        let parsed: Vec<NamedEntry> = toml::from_str(&text).ok()?;
        Some(parsed)
    }

    /// 文件监听重载入口
    pub fn reload(&self) {
        if let Some(entries) = Self::parse_file(&self.path) {
            *self.inner.write().unwrap() = entries;
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.read().unwrap().iter().map(|e| e.name.clone()).collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.inner.read().unwrap().iter().any(|e| e.name == name)
    }

    pub fn insert(&self, name: &str) -> Result<bool, String> {
        let mut guard = self.inner.write().unwrap();
        if guard.iter().any(|e| e.name == name) {
            return Ok(false);
        }
        guard.push(NamedEntry { name: name.to_string() });
        guard.sort_by(|a, b| a.name.cmp(&b.name));
        self.persist(&guard).map(|_| true)
    }

    /// 重命名（重命名/合并语义由调用方决定——注册表只管名字搬运）；返回是否存在旧名
    pub fn rename(&self, old: &str, new: &str) -> Result<bool, String> {
        let mut guard = self.inner.write().unwrap();
        let exists = guard.iter().any(|e| e.name == old);
        if exists {
            guard.retain(|e| e.name != old);
            if !guard.iter().any(|e| e.name == new) {
                guard.push(NamedEntry { name: new.to_string() });
                guard.sort_by(|a, b| a.name.cmp(&b.name));
            }
            self.persist(&guard)?;
        }
        Ok(exists)
    }

    pub fn remove(&self, name: &str) -> Result<bool, String> {
        let mut guard = self.inner.write().unwrap();
        let before = guard.len();
        guard.retain(|e| e.name != name);
        let removed = before != guard.len();
        if removed {
            self.persist(&guard)?;
        }
        Ok(removed)
    }

    fn persist(&self, entries: &[NamedEntry]) -> Result<(), String> {
        let mut text = String::new();
        for e in entries {
            let name = toml::Value::String(e.name.clone()).to_string();
            text.push_str(&format!("[[entry]]\nname = {name}\n\n"));
        }
        atomic_write(&self.path, text.as_bytes()).map_err(|e| format!("注册表写入失败: {e}"))
    }
}

// ---------- view.toml（[scopes."folder:novels"] order_by/order） ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ViewPreference {
    #[serde(default = "default_order_by")]
    pub order_by: String,
    #[serde(default = "default_order")]
    pub order: String,
}

fn default_order_by() -> String {
    "added_time".into()
}

fn default_order() -> String {
    "desc".into()
}

#[derive(Serialize, Deserialize, Default)]
struct ViewFile {
    #[serde(default)]
    scopes: BTreeMap<String, ViewPreference>,
}

pub struct ViewPreferences {
    path: String,
    inner: RwLock<BTreeMap<String, ViewPreference>>,
}

impl ViewPreferences {
    pub fn load(path: &str) -> ViewPreferences {
        let scopes = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str::<ViewFile>(&t).ok())
            .map(|f| f.scopes)
            .unwrap_or_default();
        ViewPreferences {
            path: path.to_string(),
            inner: RwLock::new(scopes),
        }
    }

    pub fn reload(&self) {
        if let Some(scopes) = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| toml::from_str::<ViewFile>(&t).ok())
            .map(|f| f.scopes)
        {
            *self.inner.write().unwrap() = scopes;
        }
    }

    pub fn all(&self) -> BTreeMap<String, ViewPreference> {
        self.inner.read().unwrap().clone()
    }

    pub fn get(&self, scope: &str) -> Option<ViewPreference> {
        self.inner.read().unwrap().get(scope).cloned()
    }

    /// 覆盖写（非法值由 API 层校验后调用）
    pub fn set(&self, scope: &str, pref: ViewPreference) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        guard.insert(scope.to_string(), pref);
        self.persist(&guard)
    }

    pub fn remove(&self, scope: &str) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        guard.remove(scope);
        self.persist(&guard)
    }

    /// 文件夹移动/重命名级联：folder:<old> → folder:<new>
    pub fn migrate_folder_scope(&self, old: &str, new: &str) -> Result<(), String> {
        let old_key = format!("folder:{old}");
        let new_key = format!("folder:{new}");
        let mut guard = self.inner.write().unwrap();
        if let Some(pref) = guard.remove(&old_key) {
            guard.insert(new_key, pref);
            return self.persist(&guard);
        }
        Ok(())
    }

    /// 前缀删除（folder:<path>/ 子树整体移除）
    pub fn remove_folder_prefix(&self, prefix: &str) -> Result<(), String> {
        let prefix = format!("folder:{prefix}");
        let mut guard = self.inner.write().unwrap();
        let before = guard.len();
        let keys: Vec<String> = guard
            .keys()
            .filter(|k| k.as_str() == prefix || k.starts_with(&format!("{prefix}/")))
            .cloned()
            .collect();
        for k in keys {
            guard.remove(&k);
        }
        if guard.len() != before {
            return self.persist(&guard);
        }
        Ok(())
    }

    /// 名称维度重命名级联（category:/tag:/author:/series:）
    pub fn migrate_named_scope(&self, dimension: &str, old: &str, new: &str) -> Result<(), String> {
        let old_key = format!("{dimension}:{old}");
        let new_key = format!("{dimension}:{new}");
        let mut guard = self.inner.write().unwrap();
        if let Some(pref) = guard.remove(&old_key) {
            // 合并语义：目标已有设置时保留目标
            guard.entry(new_key).or_insert(pref);
            return self.persist(&guard);
        }
        Ok(())
    }

    fn persist(&self, scopes: &BTreeMap<String, ViewPreference>) -> Result<(), String> {
        let mut text = String::new();
        for (scope, pref) in scopes {
            let key = toml::Value::String(scope.clone()).to_string();
            text.push_str(&format!("[scopes.{key}]\n"));
            text.push_str(&format!(
                "order_by = {}\n",
                toml::Value::String(pref.order_by.clone())
            ));
            text.push_str(&format!("order = {}\n\n", toml::Value::String(pref.order.clone())));
        }
        atomic_write(&self.path, text.as_bytes()).map_err(|e| format!("view.toml 写入失败: {e}"))
    }
}

// ---------- global_filter.toml / locks.toml 共用的四维名称集合 ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DimensionSet {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub authors: Vec<String>,
}

impl DimensionSet {
    /// 名称维度（category/tag/author）是否存在
    pub fn named_contains(&self, dimension: &str, name: &str) -> bool {
        let list = match dimension {
            "category" => &self.categories,
            "tag" => &self.tags,
            "author" => &self.authors,
            _ => return false,
        };
        list.iter().any(|n| n == name)
    }

    /// 文件夹路径命中（含子树：条目是路径前缀）
    pub fn folder_hit(&self, path: &str) -> bool {
        self.folders.iter().any(|f| {
            f == path || path.starts_with(&format!("{f}/"))
        })
    }
}

// ---------- global_filter.toml ----------

pub struct GlobalFilter {
    path: String,
    inner: RwLock<DimensionSet>,
}

impl GlobalFilter {
    pub fn load(path: &str) -> GlobalFilter {
        let set = Self::parse_file(path).unwrap_or_default();
        GlobalFilter {
            path: path.to_string(),
            inner: RwLock::new(set),
        }
    }

    fn parse_file(path: &str) -> Option<DimensionSet> {
        let text = std::fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }

    pub fn reload(&self) {
        if let Some(set) = Self::parse_file(&self.path) {
            *self.inner.write().unwrap() = set;
        }
    }

    pub fn snapshot(&self) -> DimensionSet {
        self.inner.read().unwrap().clone()
    }

    /// 标记/取消隐藏（幂等）；kind: folder|category|tag|author
    pub fn set(&self, kind: &str, name: &str, hidden: bool) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        let list = match kind {
            "folder" => &mut guard.folders,
            "category" => &mut guard.categories,
            "tag" => &mut guard.tags,
            "author" => &mut guard.authors,
            _ => return Err(format!("非法维度: {kind}")),
        };
        if hidden {
            if !list.iter().any(|n| n == name) {
                list.push(name.to_string());
            }
        } else {
            list.retain(|n| n != name);
        }
        self.persist(&guard)
    }

    /// 名称维度重命名跟随（目标已隐藏时合并：保留目标即可）
    pub fn migrate_named(&self, dimension: &str, old: &str, new: &str) -> Result<(), String> {
        self.set(dimension, old, false)?;
        // 只有目标不存在时才搬运（合并语义：目标已隐藏则保持隐藏）
        let guard = self.inner.read().unwrap();
        let target_hidden = guard.named_contains(dimension, new);
        drop(guard);
        if !target_hidden {
            self.set(dimension, new, true)?;
        }
        Ok(())
    }

    pub fn remove_named(&self, dimension: &str, name: &str) -> Result<(), String> {
        self.set(dimension, name, false)
    }

    /// 文件夹移动/重命名跟随（含移入回收站 → 原路径隐藏项保留语义由调用方决定）
    pub fn migrate_folder(&self, old: &str, new: &str) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        let had = guard.folders.iter().any(|f| f == old);
        if had {
            guard.folders.retain(|f| f != old);
            if !guard.folders.iter().any(|f| f == new) {
                guard.folders.push(new.to_string());
            }
            guard.folders.sort();
            return self.persist(&guard);
        }
        Ok(())
    }

    /// 前缀移除（文件夹删除：该前缀下全部隐藏项清除）
    pub fn remove_folder_prefix(&self, prefix: &str) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        let before = guard.folders.len();
        guard.folders.retain(|f| f != prefix && !f.starts_with(&format!("{prefix}/")));
        if guard.folders.len() != before {
            return self.persist(&guard);
        }
        Ok(())
    }

    fn persist(&self, set: &DimensionSet) -> Result<(), String> {
        let text = toml::to_string_pretty(set).map_err(|e| e.to_string())?;
        atomic_write(&self.path, text.as_bytes()).map_err(|e| format!("global_filter.toml 写入失败: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sumi-reg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_str().unwrap().to_string()
    }

    #[test]
    fn name_registry_ops() {
        let dir = temp_dir("names");
        let path = format!("{dir}/tags.toml");
        let reg = NameRegistry::load(&path);
        assert!(reg.insert("科幻").unwrap());
        assert!(!reg.insert("科幻").unwrap());
        assert!(reg.contains("科幻"));
        assert!(reg.rename("科幻", "科幻小说").unwrap());
        assert!(reg.contains("科幻小说") && !reg.contains("科幻"));
        // 重载后仍在（持久化生效）
        reg.reload();
        assert!(reg.contains("科幻小说"));
        assert!(reg.remove("科幻小说").unwrap());
        assert!(!reg.remove("科幻小说").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_preferences_scopes() {
        let dir = temp_dir("view");
        let path = format!("{dir}/view.toml");
        let prefs = ViewPreferences::load(&path);
        prefs
            .set("folder:novels", ViewPreference {
                order_by: "title".into(),
                order: "asc".into(),
            })
            .unwrap();
        prefs.set("tag:科幻", ViewPreference::default()).unwrap();
        assert_eq!(prefs.get("folder:novels").unwrap().order_by, "title");
        prefs.migrate_folder_scope("novels", "library/novels").unwrap();
        assert!(prefs.get("folder:novels").is_none());
        assert!(prefs.get("folder:library/novels").is_some());
        prefs.set("folder:library/novels/a", ViewPreference::default()).unwrap();
        prefs.remove_folder_prefix("library/novels").unwrap();
        assert!(prefs.get("folder:library/novels").is_none());
        assert!(prefs.get("folder:library/novels/a").is_none());
        // 重命名合并：目标已有设置保留目标
        prefs.set("tag:new", ViewPreference {
            order_by: "size".into(),
            order: "desc".into(),
        })
        .unwrap();
        prefs.migrate_named_scope("tag", "科幻", "new").unwrap();
        assert_eq!(prefs.get("tag:new").unwrap().order_by, "size");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn global_filter_ops() {
        let dir = temp_dir("gf");
        let path = format!("{dir}/global_filter.toml");
        let gf = GlobalFilter::load(&path);
        gf.set("folder", "private", true).unwrap();
        gf.set("category", "私密", true).unwrap();
        let snap = gf.snapshot();
        assert!(snap.folder_hit("private/日记"));
        assert!(!snap.folder_hit("public"));
        assert!(snap.named_contains("category", "私密"));
        gf.migrate_folder("private", "secret").unwrap();
        assert!(gf.snapshot().folder_hit("secret/x"));
        gf.remove_folder_prefix("secret").unwrap();
        assert!(gf.snapshot().folders.is_empty());
        gf.migrate_named("category", "私密", "机密").unwrap();
        assert!(gf.snapshot().named_contains("category", "机密"));
        gf.remove_named("category", "机密").unwrap();
        assert!(gf.snapshot().categories.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
