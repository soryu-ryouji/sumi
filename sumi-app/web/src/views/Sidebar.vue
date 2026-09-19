<script setup lang="ts">
// 侧栏：顶部拖拽条（macOS 红绿灯压左端，内容只放右端）+ 书库导航（全部书籍/回收站，回收站由 books.filter.inTrash 驱动内容区切换）
// + 文件夹树 + 分类/标签/作者/系列聚合。区块标题行可折叠，显隐可在设置「界面」中配置（偏好持久化在 ui store）。
import { computed, ref } from 'vue';
import { useLibrary } from '@/stores/library';
import { useBooks } from '@/stores/books';
import { useUi } from '@/stores/ui';
import { useConnection } from '@/stores/connection';
import { hasShell, isMac, TRAFFIC_INSET, dragDoubleclickMaximize } from '@/shared/lib/platform';
import { createFolder } from '@/shared/lib/move';
import { shell } from '@/app/shell';
import type { CountEntry } from '@/shared/api/types';
import FolderTreeNode from './FolderTreeNode.vue';
import InputDialog from './InputDialog.vue';

const library = useLibrary();
const books = useBooks();
const ui = useUi();
const conn = useConnection();
const showBrand = !isMac();

/** 根下新建文件夹对话框（文件夹区块标题行「＋」入口） */
const showNewFolder = ref(false);

const dimensions = computed(() =>
  [
    { key: 'category', title: '分类', field: 'categories' as const, entries: library.categories },
    { key: 'tag', title: '标签', field: 'tags' as const, entries: library.tags },
    { key: 'author', title: '作者', field: 'authors' as const, entries: library.authors },
    { key: 'series', title: '系列', field: 'series' as const, entries: library.series },
  ].filter((group) => ui.sidebarSections[group.key] !== false),
);

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
  // 点任何筛选都退出回收站（回收站只是 inTrash 筛选态，非独立视图）
  books.filter.inTrash = false;
  const list = books.filter[field];
  books.filter[field] = list.includes(name) ? list.filter((x) => x !== name) : [...list, name];
  void books.load();
}

function selectFolder(path: string): void {
  books.filter.inTrash = false;
  books.filter.folder = books.filter.folder === path ? null : path;
  void books.load();
}

function selectSeries(name: string): void {
  books.filter.inTrash = false;
  books.filter.series = books.filter.series === name ? null : name;
  void books.load();
}

function clearAllFilters(): void {
  books.filter.inTrash = false;
  books.filter.folder = null;
  books.filter.series = null;
  books.filter.tags = [];
  books.filter.categories = [];
  books.filter.authors = [];
  void books.load();
}

/** 进入回收站（内容区随 inTrash 切换为回收站列表） */
function enterTrash(): void {
  books.filter.inTrash = true;
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
    <!-- 顶部拖拽条：侧栏色块通高到窗口上沿；macOS 原生红绿灯压在本条左侧（左端留空），
         右端为换库入口；Windows/Linux 左端为品牌标识（窗口控制在 fixed 右上角，不在此列） -->
    <div class="sidebar-head" :style="isMac() ? { paddingLeft: TRAFFIC_INSET + 'px' } : {}" @dblclick="dragDoubleclickMaximize">
      <template v-if="showBrand">
        <img src="/icon.png" alt="" class="head-logo" />
        <span class="head-name">sumi</span>
      </template>
      <button v-if="hasShell()" class="head-btn" title="切换书库" @click="shell()?.selectLibrary()" @dblclick.stop>⇄</button>
    </div>
    <div class="sidebar-scroll">
      <!-- 书库导航块：固定显示不可配置；标题行可折叠 -->
      <div class="sidebar-section">
        <button class="sidebar-head sec-toggle" @click="ui.toggleSidebarSection('library')">
          <span class="sec-caret" :class="{ open: !ui.sidebarCollapsed.library }">▸</span>
          <span class="sidebar-title">{{ library.libraryName || '书库' }}</span>
        </button>
        <template v-if="!ui.sidebarCollapsed.library">
          <button class="link-btn" :class="{ active: !books.filter.folder && !books.filter.inTrash }" @click="clearAllFilters">全部书籍</button>
          <button class="link-btn trash" :class="{ active: books.filter.inTrash }" @click="enterTrash">回收站</button>
        </template>
      </div>

      <!-- 区块显隐只看用户设置；标题行（含＋新建入口）不依赖树非空——空库也要能建第一个文件夹 -->
      <div v-if="ui.sidebarSections.folders" class="sidebar-section">
        <!-- 标题行右侧「＋」= 根下新建文件夹（可写时显示；行内嵌套按钮非法，改为同排双控件） -->
        <div class="sec-head-row">
          <button class="sidebar-head sec-toggle" @click="ui.toggleSidebarSection('folders')">
            <span class="sec-caret" :class="{ open: !ui.sidebarCollapsed.folders }">▸</span>
            <span class="sidebar-title">文件夹</span>
          </button>
          <button v-if="conn.writable" class="sec-add" title="新建文件夹（书库根目录）" @click="showNewFolder = true">＋</button>
        </div>
        <template v-if="!ui.sidebarCollapsed.folders">
          <FolderTreeNode v-for="node in library.tree" :key="node.path" :node="node" :active="books.filter.folder" @select="selectFolder" />
        </template>
      </div>

      <div v-for="group in dimensions" :key="group.key" class="sidebar-section">
        <button class="sidebar-head sec-toggle" @click="ui.toggleSidebarSection(group.key)">
          <span class="sec-caret" :class="{ open: !ui.sidebarCollapsed[group.key] }">▸</span>
          <span class="sidebar-title">{{ group.title }}</span>
        </button>
        <template v-if="!ui.sidebarCollapsed[group.key]">
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
        </template>
      </div>
    </div>
    <InputDialog
      v-if="showNewFolder"
      title="新建文件夹"
      placeholder="文件夹名（建在书库根目录）"
      confirm-label="创建"
      @confirm="(v) => { showNewFolder = false; void createFolder(v); }"
      @cancel="showNewFolder = false"
    />
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
.sidebar-head {
  flex: none;
  height: 40px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 8px;
  /* macOS：左端留出原生红绿灯区域（红绿灯压在本条上），经动态 style 注入；其余平台从左展示品牌 */
  -webkit-app-region: drag;
  border-bottom: 1px solid var(--border);
}
/* 条在拖拽区内，按钮须退出拖拽 */
.head-btn {
  -webkit-app-region: no-drag;
  margin-left: auto;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--muted);
  font-size: 14px;
  cursor: pointer;
}
.head-btn:hover {
  background: var(--hover);
  color: var(--text);
}
.head-logo {
  width: 18px;
  height: 18px;
  border-radius: 4px;
}
.head-name {
  font-size: 12px;
  color: var(--muted);
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
/* 区块标题行：可点击折叠/展开。原生 button 无全局 reset，须自清默认样式；
   同时退出 .sidebar-head 的窗口拖拽区（drag 会吞掉点击） */
.sec-toggle {
  width: 100%;
  height: auto;
  padding: 0 4px 4px;
  border: none;
  border-bottom: none;
  background: transparent;
  font: inherit;
  text-align: left;
  cursor: pointer;
  -webkit-app-region: no-drag;
}
/* 文件夹区块标题行：折叠钮 + 右侧新建钮同排（其余区块保持原单一标题钮） */
.sec-head-row {
  display: flex;
  align-items: center;
}
.sec-head-row .sec-toggle {
  flex: 1;
  width: auto;
}
.sec-add {
  flex: none;
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--muted);
  font-size: 13px;
  line-height: 1;
  cursor: pointer;
}
.sec-add:hover {
  background: var(--hover);
  color: var(--text);
}
/* caret 展开态旋转 90°（与文件夹树 tree-caret 同款） */
.sec-caret {
  width: 12px;
  flex: none;
  color: var(--muted);
  font-size: 10px;
  transition: transform 0.12s;
  text-align: center;
}
.sec-caret.open {
  transform: rotate(90deg);
}
.sidebar-title {
  font-size: 11px;
  color: var(--muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
/* 区块内容缩进对齐标题文字（标题 = 4 左内边距 + 12 caret + 8 gap = 24px）；
   文件夹树节点自带 caret 占位已自然对齐，不在此列 */
.sidebar-section .link-btn,
.sidebar-section .dim-item {
  padding-left: 24px;
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
/* trash 的 muted 会压过 active 的白字（同优先级后写胜出），补一条恢复高亮 */
.link-btn.trash.active {
  color: #fff;
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
</style>
