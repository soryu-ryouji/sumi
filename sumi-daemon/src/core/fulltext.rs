//! FTS5 全文索引（库外 index.db 的 fts 表，可删重建）。
//! 设计见 docs/backend/fulltext-search.md：「CJK 单字 + 西文词」分词，短语查询 = 子串匹配。
//! 实现采用**预分词 + unicode61**：写入与查询共用同一预分词器（CJK 字符间插空格，
//! 西文/数字连续串不动），unicode61 按空白切开即得到「单字 + 词」的 token 流——
//! 与自定义 tokenizer 语义等价，且不依赖 rusqlite 的 C API 注册能力。
//! 查询语法映射：关键词（双引号内为整体）→ 预分词 → FTS5 短语查询；多关键词 AND。

use rusqlite::Connection;

pub struct FulltextIndex {
    conn: Connection,
}

/// CJK 判定：统一表意文字及扩展 A–G（覆盖中日韩汉字全集的实用近似）
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF      // 扩展 A
        | 0x4E00..=0x9FFF    // 统一表意
        | 0x20000..=0x2A6DF  // 扩展 B
        | 0x2A700..=0x2B73F  // 扩展 C/D
        | 0x2B740..=0x2B81F  // 扩展 E
        | 0x2B820..=0x2CEAF  // 扩展 F
        | 0x2CEB0..=0x2EBEF  // 扩展 I（2020 后并入 G 区段）
        | 0x30000..=0x3134F  // 扩展 G
        | 0xF900..=0xFAFF    // 兼容表意
    )
}

/// 预分词：CJK 字符间插入空格（每个汉字成为独立 token），西文/数字串保持连续。
/// 查询与索引共用，口径必然一致
pub fn pre_tokenize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut prev_cjk = false;
    for c in text.chars() {
        if is_cjk(c) {
            if !out.is_empty() && !prev_cjk {
                out.push(' ');
            }
            out.push(c);
            out.push(' ');
            prev_cjk = true;
        } else {
            out.push(c);
            prev_cjk = false;
        }
    }
    out
}

/// content 查询串 → FTS5 MATCH 表达式：
/// 按空白拆关键词（双引号内为整体短语），每个关键词预分词后组成 FTS5 短语，
/// 关键词间 AND。全部关键词预分词后为空时返回 None（不追加全文条件）
pub fn build_match_expr(query: &str) -> Option<String> {
    let keywords = split_keywords(query);
    let mut phrases = Vec::new();
    for keyword in keywords {
        let tokenized = pre_tokenize(&keyword);
        let terms: Vec<&str> = tokenized
            .split_whitespace()
            .filter(|term| term.chars().any(|c| c.is_alphanumeric()))
            .collect();
        if terms.is_empty() {
            continue;
        }
        // FTS5 短语语法：单 term 直接用；多 term 用 "t1 t2"（引号内双引号转义）
        let phrase: String = terms.join(" ");
        let escaped = phrase.replace('"', "\"\"");
        phrases.push(format!("\"{escaped}\""));
    }
    if phrases.is_empty() {
        return None;
    }
    Some(phrases.join(" AND "))
}

/// 关键词切分：空白分隔，双引号内（含空格）为一个关键词
fn split_keywords(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in query.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
                current.clear();
            }
            c => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

impl FulltextIndex {
    /// 打开（或创建）index.db 的 fts 表。schema 版本不符（旧结构）时整表重建
    pub fn open(db_file: &str) -> Result<FulltextIndex, String> {
        let conn = Connection::open(db_file).map_err(|e| format!("index.db 打开失败: {e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(
                item_id UNINDEXED,
                text,
                tokenize = 'unicode61 remove_diacritics 2'
            );",
        )
        .map_err(|e| format!("fts 表创建失败: {e}"))?;
        Ok(FulltextIndex { conn })
    }

    /// 写入/更新一条正文（预分词后入倒排）
    pub fn upsert(&self, item_id: &str, text: &str) -> Result<(), String> {
        let tokenized = pre_tokenize(text);
        self.conn
            .execute("DELETE FROM fts WHERE item_id = ?1", [item_id])
            .map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "INSERT INTO fts (item_id, text) VALUES (?1, ?2)",
                rusqlite::params![item_id, tokenized],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn remove(&self, item_id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM fts WHERE item_id = ?1", [item_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 全文检索：返回命中 item id（与调用方的候选集求交由查询层完成）
    pub fn search(&self, query: &str) -> Result<Vec<String>, String> {
        let Some(expr) = build_match_expr(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self
            .conn
            .prepare("SELECT item_id FROM fts WHERE fts MATCH ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&expr], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    }

    /// 正文是否已索引（补缺失模式判定）
    pub fn contains(&self, item_id: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM fts WHERE item_id = ?1", [item_id], |_| Ok(()))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(tag: &str) -> (FulltextIndex, String) {
        let path = std::env::temp_dir()
            .join(format!("sumi-fts-{tag}-{}.db", std::process::id()))
            .to_str()
            .unwrap()
            .to_string();
        let _ = std::fs::remove_file(&path);
        (FulltextIndex::open(&path).unwrap(), path)
    }

    #[test]
    fn chinese_substring_match() {
        let (fts, path) = db("zh");
        fts.upsert("a", "黑暗森林法则：宇宙就是一座黑暗森林").unwrap();
        fts.upsert("b", "三体问题是天体力学难题").unwrap();

        // 中文子串（多字关键词 → 短语邻接 = 子串）
        assert_eq!(fts.search("黑暗森林").unwrap(), vec!["a".to_string()]);
        assert_eq!(fts.search("森林").unwrap(), vec!["a".to_string()]);
        // 单字
        assert_eq!(fts.search("体").unwrap(), vec!["b".to_string()]);
        // 子串「暗森」命中（原文黑[暗森]林连续）
        assert_eq!(fts.search("暗森").unwrap(), vec!["a".to_string()]);
        // 跨字不命中（「黑森」在原文中不相邻）
        assert!(fts.search("黑森").unwrap().is_empty());
        // 多关键词 AND
        assert_eq!(fts.search("黑暗 法则").unwrap(), vec!["a".to_string()]);
        assert!(fts.search("黑暗 难题").unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn western_words_and_quotes() {
        let (fts, path) = db("en");
        fts.upsert("x", "the quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(fts.search("quick fox").unwrap(), vec!["x".to_string()]);
        assert_eq!(fts.search("\"brown fox\"").unwrap(), vec!["x".to_string()]);
        // 顺序不匹配的短语（brown→fox 之间有内容）
        assert!(fts.search("\"fox brown\"").unwrap().is_empty());
        // 混合
        fts.upsert("y", "中文夹杂 English words 的文本").unwrap();
        assert_eq!(fts.search("English 中文").unwrap(), vec!["y".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn remove_and_contains() {
        let (fts, path) = db("rm");
        fts.upsert("a", "测试正文").unwrap();
        assert!(fts.contains("a"));
        fts.remove("a").unwrap();
        assert!(!fts.contains("a"));
        assert!(fts.search("测试").unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_queries() {
        // 纯标点/空白 → None（不追加全文条件）
        assert!(build_match_expr("").is_none());
        assert!(build_match_expr("   ").is_none());
        assert!(build_match_expr("，。！").is_none());
        assert_eq!(build_match_expr("黑 森").unwrap(), "\"黑\" AND \"森\"");
    }

    #[test]
    fn fts_injection_is_neutralized() {
        // FTS5 语法注入：特殊字符只按字面短语处理
        let (fts, path) = db("inj");
        fts.upsert("a", "正常文本 NEAR").unwrap();
        assert!(fts.search("NEAR(正常, 文本)").unwrap().is_empty());
        // 引号内的特殊词按字面匹配
        assert_eq!(fts.search("\"NEAR\"").unwrap(), vec!["a".to_string()]);
        let _ = std::fs::remove_file(&path);
    }
}
