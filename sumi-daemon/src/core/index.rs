//! 内存索引：全部 Item 的权威内存态（`.sumi/` 中不存在任何索引文件）。
//! 读路径（item/list/skeleton/聚合）在轻量键上排序后只投影需要的字段（大库下降低持锁时间）；
//! 写路径一律经索引流水线的单写者（本结构的 &mut 入口只被流水线消费循环调用）。

use crate::core::item::ItemCore;
use std::collections::HashMap;

pub struct ItemIndex {
    /// id → item（含回收站位置；可见性由查询投影按位置派生）
    items: HashMap<String, ItemCore>,
    /// 文件位置（库内相对路径，含 .sumi/trash/ 前缀）→ item id
    path_owner: HashMap<String, String>,
}

impl ItemIndex {
    pub fn new() -> ItemIndex {
        ItemIndex {
            items: HashMap::new(),
            path_owner: HashMap::new(),
        }
    }

    /// 注水（启动/对账重建）：整批替换
    pub fn hydrate(&mut self, items: Vec<ItemCore>) {
        self.items.clear();
        self.path_owner.clear();
        for item in items {
            self.insert_inner(item);
        }
    }

    fn insert_inner(&mut self, item: ItemCore) {
        for p in &item.paths {
            self.path_owner.insert(p.path.clone(), item.id.clone());
        }
        self.items.insert(item.id.clone(), item);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&ItemCore> {
        self.items.get(id)
    }

    /// 路径 → item id（文件监听/迁移匹配用）
    pub fn owner_of(&self, path: &str) -> Option<&str> {
        self.path_owner.get(path).map(|s| s.as_str())
    }

    /// 全部条目引用（查询与聚合遍历）
    pub fn iter(&self) -> impl Iterator<Item = &ItemCore> {
        self.items.values()
    }

    /// 插入/替换条目（流水线单写者调用）
    pub fn upsert(&mut self, item: ItemCore) {
        // 清理旧位置映射（id 已存在时位置可能变化）
        if let Some(old) = self.items.get(&item.id) {
            for p in &old.paths {
                if self.path_owner.get(&p.path) == Some(&item.id) {
                    self.path_owner.remove(&p.path);
                }
            }
        }
        self.insert_inner(item);
    }

    /// 删除条目（清空回收站/彻底删除）
    pub fn remove(&mut self, id: &str) -> Option<ItemCore> {
        let removed = self.items.remove(id);
        if let Some(item) = &removed {
            for p in &item.paths {
                if self.path_owner.get(&p.path) == Some(&id.to_string()) {
                    self.path_owner.remove(&p.path);
                }
            }
        }
        removed
    }

    /// 名称维度聚合（category/tag/author/series 的 list 数据源；不含回收站条目）
    pub fn dimension_counts(&self, dimension: &str) -> Vec<(String, u64)> {
        let mut counts: HashMap<String, u64> = HashMap::new();
        for item in self.items.values() {
            if !item.has_library_path() {
                continue;
            }
            match dimension {
                "category" => {
                    for c in &item.categories {
                        *counts.entry(c.clone()).or_insert(0) += 1;
                    }
                }
                "tag" => {
                    for t in &item.tags {
                        *counts.entry(t.clone()).or_insert(0) += 1;
                    }
                }
                "author" => {
                    for a in &item.authors {
                        *counts.entry(a.clone()).or_insert(0) += 1;
                    }
                }
                "series" => {
                    if !item.series.is_empty() {
                        *counts.entry(item.series.clone()).or_insert(0) += 1;
                    }
                }
                _ => {}
            }
        }
        let mut list: Vec<(String, u64)> = counts.into_iter().collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }

    /// 维度重命名/删除的命中 id 集（级联迁移的输入）
    pub fn ids_with_named(&self, dimension: &str, name: &str) -> Vec<String> {
        self.items
            .values()
            .filter(|item| match dimension {
                "category" => item.categories.iter().any(|c| c == name),
                "tag" => item.tags.iter().any(|t| t == name),
                "author" => item.authors.iter().any(|a| a == name),
                "series" => item.series == name,
                _ => false,
            })
            .map(|item| item.id.clone())
            .collect()
    }

    /// 库内 item 总数（不含回收站）
    pub fn library_count(&self) -> usize {
        self.items.values().filter(|i| i.has_library_path()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::item::PathRecord;

    fn item(id: &str, paths: &[&str], tags: &[&str]) -> ItemCore {
        let mut it = ItemCore::new(
            id,
            paths.iter().map(|p| PathRecord::new(*p, 1, 1)).collect(),
            1,
        );
        it.tags = tags.iter().map(|s| s.to_string()).collect();
        it
    }

    #[test]
    fn upsert_and_query() {
        let mut index = ItemIndex::new();
        index.upsert(item("a", &["novels/x.epub"], &["科幻"]));
        index.upsert(item("b", &["tech/y.pdf"], &["科幻", "AI"]));
        assert_eq!(index.len(), 2);
        assert_eq!(index.owner_of("novels/x.epub"), Some("a"));
        assert_eq!(index.library_count(), 2);

        // 同内容多位置：位置并入既有条目
        let mut multi = item("a", &["novels/x.epub", "backup/x.epub"], &["科幻"]);
        multi.tags.push("经典".into());
        index.upsert(multi);
        assert_eq!(index.len(), 2);
        assert_eq!(index.owner_of("backup/x.epub"), Some("a"));
        assert_eq!(index.get("a").unwrap().paths.len(), 2);

        // 回收站：全部位置进 trash 后不计入库内
        index.upsert(item("b", &[".sumi/trash/tech/y.pdf"], &[]));
        assert_eq!(index.library_count(), 1);

        // 维度聚合：不含回收站条目
        let counts = index.dimension_counts("tag");
        assert_eq!(counts, vec![("科幻".to_string(), 1), ("经典".to_string(), 1)]);

        // 删除
        index.remove("a");
        assert!(index.owner_of("novels/x.epub").is_none());
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn ids_with_named() {
        let mut index = ItemIndex::new();
        index.upsert(item("a", &["x.epub"], &["科幻"]));
        index.upsert(item("b", &["y.epub"], &["技术"]));
        let mut ids = index.ids_with_named("tag", "科幻");
        ids.sort();
        assert_eq!(ids, vec!["a".to_string()]);
    }
}
