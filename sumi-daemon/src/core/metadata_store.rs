//! 元数据权威存储（双模式统一接口）：
//! - 数据库（默认）：`.sumi/metadata.db`，单文件 SQLite，写入同步落盘
//! - 配置文件：`.sumi/metadata/<hash>.toml`，网盘同步友好（冲突粒度为单本书）
//! 两模式共享同一份内存索引与查询路径；模式探测（storage_mode 标记优先）与切换
//! （写新权威层 → 删旧文件 → 写标记）的契约见 storage.md。

use crate::core::item::ItemCore;
use crate::core::metadata as toml_meta;
use crate::core::paths::LibraryPaths;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StorageMode {
    Database,
    Toml,
}

impl StorageMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            StorageMode::Database => "database",
            StorageMode::Toml => "toml",
        }
    }

    pub fn parse(s: &str) -> Option<StorageMode> {
        match s {
            "database" => Some(StorageMode::Database),
            "toml" => Some(StorageMode::Toml),
            _ => None,
        }
    }
}

/// 模式探测：`storage_mode` 标记文件优先（迁移的最后一步写入）；
/// 无标记按内容探测——metadata.db 存在且非空 → 数据库；有 metadata/*.toml → 配置文件；
/// 都没有 → 新库默认数据库。两种文件皆存（迁移中断窗口）时空 db 视为残留回退配置文件
pub fn detect_storage_mode(paths: &LibraryPaths) -> StorageMode {
    if let Ok(text) = std::fs::read_to_string(&paths.storage_mode_file) {
        if let Some(mode) = StorageMode::parse(text.trim()) {
            return mode;
        }
    }
    let db_nonempty = Path::new(&paths.metadata_db_file).exists()
        && std::fs::metadata(&paths.metadata_db_file).map(|m| m.len() > 0).unwrap_or(false);
    let has_toml = metadata_dir_entries(paths).map(|e| !e.is_empty()).unwrap_or(false);
    match (db_nonempty, has_toml) {
        (true, _) => StorageMode::Database,
        (false, true) => StorageMode::Toml,
        (false, false) => StorageMode::Database,
    }
}

/// metadata/ 目录内的合法元数据文件（id, 绝对路径）；冲突副本忽略
fn metadata_dir_entries(paths: &LibraryPaths) -> std::io::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&paths.metadata_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if toml_meta::is_metadata_file_name(&name) {
            let id = name.trim_end_matches(".toml").to_string();
            out.push((id, entry.path().to_string_lossy().into_owned()));
        }
    }
    Ok(out)
}

/// 元数据权威存储。数据库模式持连接（Mutex 串行化——写路径一律经流水线单写者，
/// 读路径为注水与迁移，无并发热点）；配置文件模式直接文件 IO
pub struct MetadataStore {
    paths: LibraryPaths,
    mode: std::sync::RwLock<StorageMode>,
    db: Mutex<Option<Connection>>,
}

impl MetadataStore {
    pub fn open(paths: LibraryPaths) -> MetadataStore {
        let mode = detect_storage_mode(&paths);
        let store = MetadataStore {
            paths,
            mode: std::sync::RwLock::new(mode),
            db: Mutex::new(None),
        };
        if mode == StorageMode::Database {
            let conn = store.open_db();
            *store.db.lock().unwrap() = Some(conn);
        }
        store
    }

    pub fn mode(&self) -> StorageMode {
        *self.mode.read().unwrap()
    }

    fn open_db(&self) -> Connection {
        let conn = Connection::open(&self.paths.metadata_db_file)
            .unwrap_or_else(|e| panic!("metadata.db 打开失败: {e}"));
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                authors TEXT NOT NULL DEFAULT '[]',
                publisher TEXT NOT NULL DEFAULT '',
                pubdate TEXT NOT NULL DEFAULT '',
                isbn TEXT NOT NULL DEFAULT '',
                language TEXT NOT NULL DEFAULT '',
                series TEXT NOT NULL DEFAULT '',
                series_index REAL NOT NULL DEFAULT 0,
                description TEXT NOT NULL DEFAULT '',
                overridden_fields TEXT NOT NULL DEFAULT '[]',
                read_status TEXT NOT NULL DEFAULT 'unread',
                progress REAL NOT NULL DEFAULT 0,
                progress_loc TEXT NOT NULL DEFAULT '',
                last_read_time INTEGER NOT NULL DEFAULT 0,
                tags TEXT NOT NULL DEFAULT '[]',
                categories TEXT NOT NULL DEFAULT '[]',
                star INTEGER NOT NULL DEFAULT 0,
                annotation TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL DEFAULT '',
                added_time INTEGER NOT NULL DEFAULT 0,
                cover_width INTEGER NOT NULL DEFAULT 0,
                cover_height INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS paths (
                path TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                size INTEGER NOT NULL DEFAULT 0,
                modification_time INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_paths_item ON paths(item_id);",
        )
        .expect("metadata.db schema 初始化失败");
        conn
    }

    /// 全量注水（启动/对账重建）：按当前模式读全部条目
    pub fn load_all(&self) -> Vec<ItemCore> {
        match self.mode() {
            StorageMode::Toml => self.load_all_toml(),
            StorageMode::Database => self.load_all_db(),
        }
    }

    fn load_all_toml(&self) -> Vec<ItemCore> {
        let entries = match metadata_dir_entries(&self.paths) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::with_capacity(entries.len());
        for (id, path) in entries {
            match toml_meta::read_metadata_file(&path, &id) {
                Ok(Some(item)) => out.push(item),
                Ok(None) => {}
                Err(e) => tracing::warn!("跳过损坏的元数据文件: {e}"),
            }
        }
        out
    }

    fn load_all_db(&self) -> Vec<ItemCore> {
        let guard = self.db.lock().unwrap();
        let Some(conn) = guard.as_ref() else {
            return Vec::new();
        };
        let mut by_id: HashMap<String, ItemCore> = HashMap::new();
        let mut stmt = match conn.prepare(
            "SELECT id, title, authors, publisher, pubdate, isbn, language, series, series_index,
                    description, overridden_fields, read_status, progress, progress_loc, last_read_time,
                    tags, categories, star, annotation, url, added_time, cover_width, cover_height
             FROM items",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, f64>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, i64>(14)?,
                row.get::<_, String>(15)?,
                row.get::<_, String>(16)?,
                row.get::<_, i64>(17)?,
                row.get::<_, String>(18)?,
                row.get::<_, String>(19)?,
                row.get::<_, i64>(20)?,
                row.get::<_, i64>(21)?,
                row.get::<_, i64>(22)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (
                    id, title, authors, publisher, pubdate, isbn, language, series, series_index,
                    description, overridden_fields, read_status, progress, progress_loc,
                    last_read_time, tags, categories, star, annotation, url, added_time,
                    cover_width, cover_height,
                ) = row;
                by_id.insert(
                    id.clone(),
                    ItemCore {
                        id,
                        title,
                        authors: parse_json_array(&authors),
                        publisher,
                        pubdate,
                        isbn,
                        language,
                        series,
                        series_index,
                        description,
                        overridden_fields: parse_json_array(&overridden_fields),
                        read_status,
                        progress,
                        progress_loc,
                        last_read_time,
                        tags: parse_json_array(&tags),
                        categories: parse_json_array(&categories),
                        star,
                        annotation,
                        url,
                        added_time,
                        cover_width: cover_width.max(0) as u32,
                        cover_height: cover_height.max(0) as u32,
                        paths: Vec::new(),
                    },
                );
            }
        }
        let mut stmt = match conn.prepare("SELECT path, item_id, size, modification_time FROM paths")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                crate::core::item::PathRecord {
                    path: row.get(0)?,
                    size: row.get::<_, i64>(2)?.max(0) as u64,
                    modification_time: row.get(3)?,
                },
            ))
        });
        if let Ok(rows) = rows {
            for (item_id, record) in rows.flatten() {
                if let Some(item) = by_id.get_mut(&item_id) {
                    item.paths.push(record);
                }
            }
        }
        by_id.into_values().collect()
    }

    /// 写入/更新单条（原子）：先写权威层，成功即视为持久
    pub fn upsert(&self, item: &ItemCore) -> Result<(), String> {
        match self.mode() {
            StorageMode::Toml => {
                let path = format!("{}/{}.toml", self.paths.metadata_dir, item.id);
                toml_meta::write_metadata_file(&path, item)
                    .map_err(|e| format!("元数据写入失败: {e}"))
            }
            StorageMode::Database => {
                let guard = self.db.lock().unwrap();
                let conn = guard.as_ref().unwrap();
                let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                upsert_db(&tx, item)?;
                tx.commit().map_err(|e| e.to_string())
            }
        }
    }

    /// 批量写入（数据库模式单事务；配置文件模式逐文件原子写）
    pub fn upsert_batch(&self, items: &[ItemCore]) -> Result<(), String> {
        match self.mode() {
            StorageMode::Toml => {
                for item in items {
                    self.upsert(item)?;
                }
                Ok(())
            }
            StorageMode::Database => {
                let guard = self.db.lock().unwrap();
                let conn = guard.as_ref().unwrap();
                let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                for item in items {
                    upsert_db(&tx, item)?;
                }
                tx.commit().map_err(|e| e.to_string())
            }
        }
    }

    /// 删除单条（迁移/清空回收站用）
    pub fn delete(&self, id: &str) -> Result<(), String> {
        match self.mode() {
            StorageMode::Toml => {
                let path = format!("{}/{id}.toml", self.paths.metadata_dir);
                match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(format!("元数据删除失败: {e}")),
                }
            }
            StorageMode::Database => {
                let guard = self.db.lock().unwrap();
                let conn = guard.as_ref().unwrap();
                conn.execute("DELETE FROM paths WHERE item_id = ?1", [id])
                    .map_err(|e| e.to_string())?;
                conn.execute("DELETE FROM items WHERE id = ?1", [id])
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
        }
    }

    /// 存储模式切换：单写者内全量互转（写新权威层 → 删旧文件 → 写标记 → 切内存态）。
    /// 内存态切换后立即使新权威层生效（不等进程重启，消除「迁移后到重启前的写入落旧层」
    /// 的丢失窗口）；旧文件被占用删不掉时（Windows）由下次启动清理兜底
    pub fn migrate(&self, target: StorageMode, all_items: &[ItemCore]) -> Result<(), String> {
        if target == self.mode() {
            return Ok(());
        }
        match target {
            StorageMode::Database => {
                let conn = self.open_db();
                let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                for item in all_items {
                    upsert_db(&tx, item)?;
                }
                tx.commit().map_err(|e| e.to_string())?;
                drop(conn);
                // 删旧 TOML（逐文件；失败记录但不阻断——标记文件才是权威判定）
                if let Ok(entries) = metadata_dir_entries(&self.paths) {
                    for (_, path) in entries {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            StorageMode::Toml => {
                for item in all_items {
                    let path = format!("{}/{}.toml", self.paths.metadata_dir, item.id);
                    toml_meta::write_metadata_file(&path, item)
                        .map_err(|e| format!("迁移写入失败 {path}: {e}"))?;
                }
                // 先关 db 连接再删文件：Windows 删除打开中的 SQLite 文件会失败（Unix 允许）
                *self.db.lock().unwrap() = None;
                // 删旧 db（WAL/SHM 伴随文件一并）
                for suffix in ["", "-wal", "-shm"] {
                    let p = format!("{}{suffix}", self.paths.metadata_db_file);
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        crate::core::config::atomic_write(&self.paths.storage_mode_file, target.as_str().as_bytes())
            .map_err(|e| format!("storage_mode 标记写入失败: {e}"))?;
        self.activate(target);
        Ok(())
    }

    /// 迁移完成后的内存态切换：重置 db 连接并翻转 mode（写路径立即走新权威层）
    fn activate(&self, target: StorageMode) {
        let mut guard = self.db.lock().unwrap();
        // 先关旧连接（Windows 上句柄不释放会占用文件）
        *guard = None;
        if target == StorageMode::Database {
            *guard = Some(self.open_db());
        }
        *self.mode.write().unwrap() = target;
    }
}

fn upsert_db(conn: &Connection, item: &ItemCore) -> Result<(), String> {
    conn.execute(
        "INSERT INTO items (id, title, authors, publisher, pubdate, isbn, language, series, series_index,
                            description, overridden_fields, read_status, progress, progress_loc, last_read_time,
                            tags, categories, star, annotation, url, added_time, cover_width, cover_height)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
         ON CONFLICT(id) DO UPDATE SET
            title=excluded.title, authors=excluded.authors, publisher=excluded.publisher,
            pubdate=excluded.pubdate, isbn=excluded.isbn, language=excluded.language,
            series=excluded.series, series_index=excluded.series_index, description=excluded.description,
            overridden_fields=excluded.overridden_fields, read_status=excluded.read_status,
            progress=excluded.progress, progress_loc=excluded.progress_loc, last_read_time=excluded.last_read_time,
            tags=excluded.tags, categories=excluded.categories, star=excluded.star,
            annotation=excluded.annotation, url=excluded.url, added_time=excluded.added_time,
            cover_width=excluded.cover_width, cover_height=excluded.cover_height",
        rusqlite::params![
            item.id,
            item.title,
            serde_json::to_string(&item.authors).unwrap_or_else(|_| "[]".into()),
            item.publisher,
            item.pubdate,
            item.isbn,
            item.language,
            item.series,
            item.series_index,
            item.description,
            serde_json::to_string(&item.overridden_fields).unwrap_or_else(|_| "[]".into()),
            item.read_status,
            item.progress,
            item.progress_loc,
            item.last_read_time,
            serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&item.categories).unwrap_or_else(|_| "[]".into()),
            item.star,
            item.annotation,
            item.url,
            item.added_time,
            item.cover_width as i64,
            item.cover_height as i64,
        ],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM paths WHERE item_id = ?1", [&item.id])
        .map_err(|e| e.to_string())?;
    for p in &item.paths {
        conn.execute(
            "INSERT OR REPLACE INTO paths (path, item_id, size, modification_time) VALUES (?1,?2,?3,?4)",
            rusqlite::params![p.path, item.id, p.size as i64, p.modification_time],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn parse_json_array(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::item::PathRecord;

    fn temp_library(tag: &str) -> LibraryPaths {
        let dir = std::env::temp_dir().join(format!("sumi-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        LibraryPaths::new(dir.to_str().unwrap(), Some("/tmp/sumi-store-cache".into()))
    }

    fn sample(id: &str) -> ItemCore {
        let mut item = ItemCore::new(id, vec![PathRecord::new("novels/三体.epub", 100, 42)], 7);
        item.title = "三体".into();
        item.tags = vec!["科幻".into()];
        item.star = 5;
        item
    }

    #[test]
    fn detect_defaults_and_roundtrip_both_modes() {
        let paths = temp_library("detect");
        paths.ensure_layout();
        // 新库默认数据库
        assert_eq!(detect_storage_mode(&paths), StorageMode::Database);

        // 数据库模式 roundtrip
        let store = MetadataStore::open(paths.clone());
        assert_eq!(store.mode(), StorageMode::Database);
        store.upsert(&sample(&"a".repeat(64))).unwrap();
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "三体");
        assert_eq!(loaded[0].paths[0].path, "novels/三体.epub");

        // 迁移到 toml：写新层删旧层写标记
        store.migrate(StorageMode::Toml, &loaded).unwrap();
        assert_eq!(detect_storage_mode(&paths), StorageMode::Toml);
        assert!(!Path::new(&paths.metadata_db_file).exists());

        // 内存态立即切换：不重启，后续写入直接落新层（toml 文件可见）
        let mut edited = loaded[0].clone();
        edited.star = 3;
        store.upsert(&edited).unwrap();
        let reloaded = MetadataStore::open(paths.clone()).load_all();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].star, 3);

        // toml 模式 roundtrip
        let store = MetadataStore::open(paths.clone());
        assert_eq!(store.mode(), StorageMode::Toml);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].star, 3);

        // 迁回数据库
        store.migrate(StorageMode::Database, &loaded).unwrap();
        assert_eq!(detect_storage_mode(&paths), StorageMode::Database);
        let store = MetadataStore::open(paths.clone());
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "三体");

        // 删除
        store.delete(&"a".repeat(64)).unwrap();
        assert!(store.load_all().is_empty());

        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn conflict_copies_ignored() {
        let paths = temp_library("conflict");
        paths.ensure_layout();
        let id = "b".repeat(64);
        std::fs::write(format!("{}/{id}.toml", paths.metadata_dir), toml_meta::to_toml(&sample(&id))).unwrap();
        std::fs::write(
            format!("{}/{id}.sync-conflict-20250101.toml", paths.metadata_dir),
            "title = '冲突副本'",
        )
        .unwrap();
        assert_eq!(detect_storage_mode(&paths), StorageMode::Toml);
        let store = MetadataStore::open(paths.clone());
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "b".repeat(64));
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
