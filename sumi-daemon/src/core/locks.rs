//! 四维度锁（`.sumi/locks.toml`，Argon2id 密码哈希，服务端强制）。
//! 解锁票据为 daemon 内存态（重启失效），各客户端独立持有；验证失败全局节流
//! （连续 5 次冷却 60s，429 THROTTLED）。判定口径见 API 文档 lock 节。

use crate::core::config::atomic_write;
use crate::core::registry_file::DimensionSet;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LockFile {
    #[serde(default)]
    pub folders: Vec<LockEntry>,
    #[serde(default)]
    pub categories: Vec<NamedLock>,
    #[serde(default)]
    pub tags: Vec<NamedLock>,
    #[serde(default)]
    pub authors: Vec<NamedLock>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LockEntry {
    pub path: String,
    pub password: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NamedLock {
    pub name: String,
    pub password: String,
}

impl LockFile {
    /// 名称快照（不含密码哈希；lock/list 响应与 locks.changed 负载）
    pub fn names(&self) -> DimensionSet {
        DimensionSet {
            folders: self.folders.iter().map(|l| l.path.clone()).collect(),
            categories: self.categories.iter().map(|l| l.name.clone()).collect(),
            tags: self.tags.iter().map(|l| l.name.clone()).collect(),
            authors: self.authors.iter().map(|l| l.name.clone()).collect(),
        }
    }
}

/// 解锁票据集合：请求头 X-Sumi-Unlock / 查询参数 ?unlock= 携带的票据在此校验。
/// 票据 → 锁条目的反查由 Locks 持有
struct TicketStore {
    /// 票据 → (dimension, name)
    issued: HashMap<String, (String, String)>,
}

pub struct Locks {
    path: String,
    inner: std::sync::RwLock<LockFile>,
    tickets: Mutex<TicketStore>,
    /// 验证失败全局节流：连续失败计数与冷却起点
    throttle: Mutex<(u32, Option<Instant>)>,
}

const MAX_FAILED_ATTEMPTS: u32 = 5;
const COOLDOWN_SECS: u64 = 60;

fn hash_password(password: &str) -> Result<String, String> {
    // getrandom 产 16 字节盐（b64 后即 SaltString 合法形态）
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).expect("生成盐失败");
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| format!("盐编码失败: {e}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("密码哈希失败: {e}"))
}

fn verify_password(hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

impl Locks {
    pub fn load(path: &str) -> Locks {
        let file = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str::<LockFile>(&t).ok())
            .unwrap_or_default();
        Locks {
            path: path.to_string(),
            inner: std::sync::RwLock::new(file),
            tickets: Mutex::new(TicketStore { issued: HashMap::new() }),
            throttle: Mutex::new((0, None)),
        }
    }

    pub fn reload(&self) {
        if let Some(file) = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| toml::from_str::<LockFile>(&t).ok())
        {
            *self.inner.write().unwrap() = file;
            // 外部同步重载后，已发票据按目标条目是否仍存在收敛（不存在的条目票据作废）
            let names = self.inner.read().unwrap().names();
            self.tickets.lock().unwrap().issued.retain(|_, (dim, name)| {
                names.named_contains(dim, name) || (dim == "folder" && names.folders.iter().any(|f| f == name))
            });
        }
    }

    pub fn snapshot_names(&self) -> DimensionSet {
        self.inner.read().unwrap().names()
    }

    /// 节流检查：Some(剩余冷却秒) 表示当前被节流
    fn throttle_check(&self) -> Option<u64> {
        let mut guard = self.throttle.lock().unwrap();
        let (fails, since) = *guard;
        if fails >= MAX_FAILED_ATTEMPTS {
            if let Some(start) = since {
                let elapsed = start.elapsed().as_secs();
                if elapsed < COOLDOWN_SECS {
                    return Some(COOLDOWN_SECS - elapsed);
                }
                // 冷却结束，复位
                *guard = (0, None);
            }
        }
        None
    }

    fn record_failure(&self) {
        let mut guard = self.throttle.lock().unwrap();
        let (fails, _) = *guard;
        let new_fails = fails + 1;
        *guard = (new_fails, if new_fails >= MAX_FAILED_ATTEMPTS { Some(Instant::now()) } else { None });
    }

    fn record_success(&self) {
        *self.throttle.lock().unwrap() = (0, None);
    }

    /// 设锁/改密：已锁条目改密必须带正确旧密码（否则 OLD_PASSWORD_REQUIRED）
    pub fn set(
        &self,
        dimension: &str,
        name: &str,
        password: &str,
        old_password: Option<&str>,
    ) -> Result<(), SetLockError> {
        if let Some(remain) = self.throttle_check() {
            return Err(SetLockError::Throttled(remain));
        }
        let existing_hash = {
            let guard = self.inner.read().unwrap();
            find_hash(&guard, dimension, name).map(|h| h.to_string())
        };
        if let Some(hash) = existing_hash {
            let old = old_password.unwrap_or("");
            if !verify_password(&hash, old) {
                // 旧密码校验失败：计为一次验证失败（节流口径）
                self.record_failure();
                return Err(if old.is_empty() {
                    SetLockError::OldPasswordRequired
                } else {
                    SetLockError::WrongPassword
                });
            }
        }
        self.record_success();
        let hashed = hash_password(password).map_err(SetLockError::Internal)?;
        let mut guard = self.inner.write().unwrap();
        set_hash(&mut guard, dimension, name, hashed);
        self.persist(&guard).map_err(SetLockError::Internal)
    }

    /// 解除锁：需密码
    pub fn remove(&self, dimension: &str, name: &str, password: &str) -> Result<bool, SetLockError> {
        if let Some(remain) = self.throttle_check() {
            return Err(SetLockError::Throttled(remain));
        }
        let mut guard = self.inner.write().unwrap();
        let Some(hash) = find_hash(&guard, dimension, name).map(|h| h.to_string()) else {
            return Ok(false); // 未上锁（LOCK_NOT_FOUND 由 API 层判定）
        };
        if !verify_password(&hash, password) {
            self.record_failure();
            return Err(SetLockError::WrongPassword);
        }
        self.record_success();
        remove_entry(&mut guard, dimension, name);
        self.persist(&guard).map_err(SetLockError::Internal)?;
        Ok(true)
    }

    /// 解锁：校验密码发放随机票据（daemon 内存，重启失效）
    pub fn unlock(&self, dimension: &str, name: &str, password: &str) -> Result<Option<String>, UnlockError> {
        if let Some(remain) = self.throttle_check() {
            return Err(UnlockError::Throttled(remain));
        }
        let guard = self.inner.read().unwrap();
        let Some(hash) = find_hash(&guard, dimension, name) else {
            return Err(UnlockError::NotFound);
        };
        if !verify_password(hash, password) {
            self.record_failure();
            return Err(UnlockError::WrongPassword);
        }
        self.record_success();
        let mut bytes = [0u8; 24];
        getrandom::fill(&mut bytes).expect("生成解锁票据失败");
        let ticket: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        self.tickets
            .lock()
            .unwrap()
            .issued
            .insert(ticket.clone(), (dimension.to_string(), name.to_string()));
        Ok(Some(ticket))
    }

    /// 请求携带的票据集合对应的「已解锁锁集合」（名称维度）
    pub fn unlocked_dimensions(&self, tickets: &[String]) -> DimensionSet {
        let mut out = DimensionSet::default();
        let store = self.tickets.lock().unwrap();
        for t in tickets {
            if let Some((dim, name)) = store.issued.get(t) {
                match dim.as_str() {
                    "category" => out.categories.push(name.clone()),
                    "tag" => out.tags.push(name.clone()),
                    "author" => out.authors.push(name.clone()),
                    "folder" => out.folders.push(name.clone()),
                    _ => {}
                }
            }
        }
        out
    }

    /// 级联：名称维度重命名跟随（目标已锁时合并，保留目标密码）
    pub fn migrate_named(&self, dimension: &str, old: &str, new: &str) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        let Some(hash) = find_hash(&guard, dimension, old).map(|h| h.to_string()) else {
            return Ok(());
        };
        remove_entry(&mut guard, dimension, old);
        if find_hash(&guard, dimension, new).is_none() {
            set_hash(&mut guard, dimension, new, hash);
        }
        self.persist(&guard)
    }

    /// 级联：名称维度删除清除
    pub fn remove_named(&self, dimension: &str, name: &str) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        let removed = remove_entry(&mut guard, dimension, name);
        if removed {
            return self.persist(&guard);
        }
        Ok(())
    }

    /// 级联：文件夹移动/重命名跟随
    pub fn migrate_folder(&self, old: &str, new: &str) -> Result<(), String> {
        self.migrate_named("folder", old, new)
    }

    /// 级联：文件夹前缀移除（删除/清空回收站）
    pub fn remove_folder_prefix(&self, prefix: &str) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        let before = guard.folders.len();
        guard
            .folders
            .retain(|l| l.path != prefix && !l.path.starts_with(&format!("{prefix}/")));
        if guard.folders.len() != before {
            return self.persist(&guard);
        }
        Ok(())
    }

    fn persist(&self, file: &LockFile) -> Result<(), String> {
        let text = toml::to_string_pretty(file).map_err(|e| e.to_string())?;
        atomic_write(&self.path, text.as_bytes()).map_err(|e| format!("locks.toml 写入失败: {e}"))
    }
}

#[derive(Debug)]
pub enum SetLockError {
    OldPasswordRequired,
    WrongPassword,
    Throttled(u64),
    Internal(String),
}

#[derive(Debug)]
pub enum UnlockError {
    NotFound,
    WrongPassword,
    Throttled(u64),
}

fn find_hash<'a>(file: &'a LockFile, dimension: &str, name: &str) -> Option<&'a str> {
    match dimension {
        "folder" => file.folders.iter().find(|l| l.path == name).map(|l| l.password.as_str()),
        "category" => file.categories.iter().find(|l| l.name == name).map(|l| l.password.as_str()),
        "tag" => file.tags.iter().find(|l| l.name == name).map(|l| l.password.as_str()),
        "author" => file.authors.iter().find(|l| l.name == name).map(|l| l.password.as_str()),
        _ => None,
    }
}

fn set_hash(file: &mut LockFile, dimension: &str, name: &str, hash: String) {
    match dimension {
        "folder" => {
            if let Some(l) = file.folders.iter_mut().find(|l| l.path == name) {
                l.password = hash;
            } else {
                file.folders.push(LockEntry { path: name.to_string(), password: hash });
            }
        }
        "category" => {
            if let Some(l) = file.categories.iter_mut().find(|l| l.name == name) {
                l.password = hash;
            } else {
                file.categories.push(NamedLock { name: name.to_string(), password: hash });
            }
        }
        "tag" => {
            if let Some(l) = file.tags.iter_mut().find(|l| l.name == name) {
                l.password = hash;
            } else {
                file.tags.push(NamedLock { name: name.to_string(), password: hash });
            }
        }
        "author" => {
            if let Some(l) = file.authors.iter_mut().find(|l| l.name == name) {
                l.password = hash;
            } else {
                file.authors.push(NamedLock { name: name.to_string(), password: hash });
            }
        }
        _ => {}
    }
}

fn remove_entry(file: &mut LockFile, dimension: &str, name: &str) -> bool {
    match dimension {
        "folder" => {
            let before = file.folders.len();
            file.folders.retain(|l| l.path != name);
            before != file.folders.len()
        }
        "category" => {
            let before = file.categories.len();
            file.categories.retain(|l| l.name != name);
            before != file.categories.len()
        }
        "tag" => {
            let before = file.tags.len();
            file.tags.retain(|l| l.name != name);
            before != file.tags.len()
        }
        "author" => {
            let before = file.authors.len();
            file.authors.retain(|l| l.name != name);
            before != file.authors.len()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sumi-locks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_str().unwrap().to_string()
    }

    #[test]
    fn lock_lifecycle_and_tickets() {
        let dir = temp_dir("lifecycle");
        let path = format!("{dir}/locks.toml");
        let locks = Locks::load(&path);

        // 设锁
        locks.set("category", "私密", "pass123", None).unwrap();
        let names = locks.snapshot_names();
        assert!(names.named_contains("category", "私密"));

        // 改密必须带旧密码
        match locks.set("category", "私密", "new", None) {
            Err(SetLockError::OldPasswordRequired) => {}
            other => panic!("期望 OldPasswordRequired，得到 {other:?}"),
        }
        locks.set("category", "私密", "newpass", Some("pass123")).unwrap();

        // 解锁：错误密码 → WrongPassword；正确 → 票据
        match locks.unlock("category", "私密", "bad") {
            Err(UnlockError::WrongPassword) => {}
            other => panic!("期望 WrongPassword，得到 {other:?}"),
        }
        let ticket = locks.unlock("category", "私密", "newpass").unwrap().unwrap();
        let unlocked = locks.unlocked_dimensions(&[ticket.clone()]);
        assert!(unlocked.named_contains("category", "私密"));

        // 解除锁
        locks.remove("category", "私密", "newpass").unwrap();
        assert!(!locks.snapshot_names().named_contains("category", "私密"));

        // 重载后票据仍在（条目没了才作废）
        locks.reload();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_migration() {
        let dir = temp_dir("migrate");
        let path = format!("{dir}/locks.toml");
        let locks = Locks::load(&path);
        locks.set("folder", "private", "pw", None).unwrap();
        locks.migrate_folder("private", "moved").unwrap();
        let names = locks.snapshot_names();
        assert!(names.folders.iter().any(|f| f == "moved"));
        locks.set("folder", "moved/sub", "pw", None).unwrap();
        locks.remove_folder_prefix("moved").unwrap();
        assert!(locks.snapshot_names().folders.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
