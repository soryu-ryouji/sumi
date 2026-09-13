<script setup lang="ts">
// 侧栏：文件夹树 + 分类/标签/作者/系列聚合（点击筛选；锁名目提示；入口组）。
import { computed } from 'vue';
import { useLibrary } from '@/stores/library';
import { useBooks } from '@/stores/books';
import { useUi } from '@/stores/ui';
import type { CountEntry } from '@/shared/api/types';
import FolderTreeNode from './FolderTreeNode.vue';

const library = useLibrary();
const books = useBooks();
const ui = useUi();

const dimensions = computed(() => [
  { key: 'category', title: '分类', field: 'categories' as const, entries: library.categories },
  { key: 'tag', title: '标签', field: 'tags' as const, entries: library.tags },
  { key: 'author', title: '作者', field: 'authors' as const, entries: library.authors },
  { key: 'series', title: '系列', field: 'series' as const, entries: library.series },
]);

/** 维度是否已上锁（未解锁的名称在列表服务端仍可见——锁保护的是条目内容与主动筛选） */
function isLocked(field: string, name: string): boolean {
  const locks = library.locks;
  if (!locks) {
    return false;
  }
  const map: Record<string, keyof typeof locks> = { categories: 'categories', tags: 'tags', authors: 'authors' };
  const key = map[field];
  return key ? locks[key].includes(name) : false;
}

function toggleArrayFilter(field: 'tags' | 'categories' | 'authors', name: string): void {
  const list = books.filter[field];
  books.filter[field] = list.includes(name) ? list.filter((x) => x !== name) : [...list, name];
  void books.load();
}

function selectFolder(path: string): void {
  books.filter.folder = books.filter.folder === path ? null : path;
  void books.load();
}

function selectSeries(name: string): void {
  books.filter.series = books.filter.series === name ? null : name;
  void books.load();
}

function clearAllFilters(): void {
  books.filter.folder = null;
  books.filter.series = null;
  books.filter.tags = [];
  books.filter.categories = [];
  books.filter.authors = [];
  void books.load();
}

function dimActive(group: (typeof dimensions.value)[number], name: string): boolean {
  return group.field === 'series' ? books.filter.series === name : (books.filter[group.field] as string[]).includes(name);
}

function dimClick(group: (typeof dimensions.value)[number], name: string): void {
  if (group.field === 'series') {
    selectSeries(name);
  } else {
    toggleArrayFilter(group.field, name);
  }
}

const entryCount = (e: CountEntry) => e.count;
</script>

<template>
  <aside class="sidebar">
    <div class="sidebar-scroll">
      <div class="sidebar-section">
        <div class="sidebar-head"><span class="sidebar-title">{{ library.libraryName || '书库' }}</span></div>
        <button class="link-btn" :class="{ active: !books.filter.folder && !books.filter.inTrash }" @click="clearAllFilters">全部书籍</button>
        <button class="link-btn trash" @click="ui.view = 'trash'">回收站</button>
      </div>

      <div v-if="library.tree.length" class="sidebar-section">
        <div class="sidebar-head"><span class="sidebar-title">文件夹</span></div>
        <FolderTreeNode v-for="node in library.tree" :key="node.path" :node="node" :active="books.filter.folder" @select="selectFolder" />
      </div>

      <div v-for="group in dimensions" :key="group.key" class="sidebar-section">
        <div class="sidebar-head"><span class="sidebar-title">{{ group.title }}</span></div>
        <button
          v-for="entry in group.entries"
          :key="entry.name"
          class="dim-item"
          :class="{ active: dimActive(group, entry.name) }"
          @click="dimClick(group, entry.name)"
        >
          <span class="dim-name">{{ entry.name }}</span>
          <span v-if="group.field !== 'series' && isLocked(group.field, entry.name)" class="dim-lock" title="已锁定（在设置中解锁）">🔒</span>
          <span class="dim-count">{{ entryCount(entry) }}</span>
        </button>
        <div v-if="!group.entries.length" class="dim-empty">暂无</div>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: 220px;
  flex: none;
  background: var(--panel);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.sidebar-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 10px 8px 20px;
}
.sidebar-section {
  margin-bottom: 14px;
}
.sidebar-head {
  padding: 0 8px 4px;
}
.sidebar-title {
  font-size: 11px;
  color: var(--muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.link-btn {
  display: block;
  width: 100%;
  text-align: left;
  padding: 5px 8px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
}
.link-btn:hover {
  background: var(--hover);
}
.link-btn.active {
  background: var(--accent-dim);
  color: #fff;
}
.link-btn.trash {
  color: var(--muted);
}

.dim-item {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 4px 8px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
  min-width: 0;
}
.dim-item:hover {
  background: var(--hover);
}
.dim-item.active {
  background: var(--accent-dim);
  color: #fff;
}
.dim-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
}
.dim-count {
  font-size: 11px;
  color: var(--muted);
}
.dim-item.active .dim-count {
  color: rgba(255, 255, 255, 0.8);
}
.dim-lock {
  font-size: 11px;
}
.dim-empty {
  padding: 4px 8px;
  font-size: 12px;
  color: var(--muted);
}
</style>
