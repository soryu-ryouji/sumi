<script setup lang="ts">
// 顶栏（通栏，覆盖内容区与详情侧板上方）：左侧栏开关 ‖ 计数/导入指示 ‖ 搜索、筛选、设置、详情侧板开关。
// 搜索默认为图标，点击展开输入框（Esc 清空/收起，失焦且为空时收起）；阅读状态与排序收进筛选菜单。
// 整条为窗口拖拽区（双击空白切换最大化），交互控件单独 no-drag。
import { computed, nextTick, ref, watch } from 'vue';
import { useBooks, ORDER_LABELS } from '@/stores/books';
import { useUi } from '@/stores/ui';
import { hasShell, isMac, TRAFFIC_INSET, CONTROLS_INSET, dragDoubleclickMaximize } from '@/shared/lib/platform';
import { openContextMenuAt, type CtxItem } from '@/shared/lib/context-menu';
import { importingCount } from '@/shared/lib/importer';
import { formatBytes } from '@/shared/format';

const books = useBooks();
const ui = useUi();

const searchText = ref('');
const searchMode = ref<'meta' | 'content'>('meta');
/** 搜索框展开态（图标 → 输入框）；有搜索文本时常驻 */
const searchOpen = ref(false);
const searchInput = ref<HTMLInputElement | null>(null);

// 输入防抖 → 过滤态
let debounce: ReturnType<typeof setTimeout> | undefined;
watch(searchText, (v) => {
  clearTimeout(debounce);
  debounce = setTimeout(() => {
    if (searchMode.value === 'meta') {
      const kws = v.trim() ? [v.trim()] : [];
      if (kws.join() !== books.filter.keywords.join()) {
        books.filter.keywords = kws;
        books.filter.content = '';
        void books.load();
      }
    } else {
      if (v.trim() !== books.filter.content) {
        books.filter.content = v.trim();
        books.filter.keywords = [];
        void books.load();
      }
    }
  }, 350);
});
watch(searchMode, () => {
  // 模式切换立即按当前输入重查
  const v = searchText.value.trim();
  books.filter.keywords = searchMode.value === 'meta' && v ? [v] : [];
  books.filter.content = searchMode.value === 'content' ? v : '';
  void books.load();
});

async function openSearch(): Promise<void> {
  searchOpen.value = true;
  await nextTick();
  searchInput.value?.focus();
}

/** 搜索开关：再点一次关闭并清空（清空经防抖 watcher 重查，与 Esc 行为一致） */
function toggleSearch(): void {
  if (searchOpen.value || searchText.value) {
    searchText.value = '';
    searchOpen.value = false;
  } else {
    void openSearch();
  }
}

function onSearchBlur(): void {
  if (!searchText.value) {
    searchOpen.value = false;
  }
}

function onSearchEsc(): void {
  if (searchText.value) {
    searchText.value = '';
  } else {
    searchOpen.value = false;
  }
}

const readStatusOptions = [
  { value: '', label: '全部状态' },
  { value: 'unread', label: '未读' },
  { value: 'reading', label: '在读' },
  { value: 'finished', label: '读完' },
  { value: 'abandoned', label: '弃读' },
];
const readStatus = ref('');
watch(readStatus, (v) => {
  books.filter.readStatus = v || null;
  void books.load();
});

/** 筛选菜单：阅读状态（单选）+ 排序字段（单选）+ 排序方向（单选），当前项打勾 */
function openFilterMenu(e: MouseEvent): void {
  const items: CtxItem[] = [
    ...readStatusOptions.map((o) => ({
      label: o.label,
      checked: readStatus.value === o.value,
      action: () => {
        readStatus.value = o.value;
      },
    })),
    { separator: true },
    ...Object.entries(ORDER_LABELS).map(([value, label]) => ({
      label: `按${label}排序`,
      checked: books.filter.orderBy === value,
      action: () => {
        books.filter.orderBy = value;
        void books.load();
      },
    })),
    { separator: true },
    {
      label: '升序',
      checked: books.filter.order === 'asc',
      action: () => {
        books.filter.order = 'asc';
        void books.load();
      },
    },
    {
      label: '降序',
      checked: books.filter.order === 'desc',
      action: () => {
        books.filter.order = 'desc';
        void books.load();
      },
    },
  ];
  // 锚定到按钮下缘，避免菜单盖住按钮本身；超出视口的回翻由宿主测量修正
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  openContextMenuAt(items, rect.left, rect.bottom + 6);
}

const totalLabel = computed(() => `${books.total} 本 · ${formatBytes(books.totalSize)}`);
</script>

<template>
  <div
    class="topbar"
    :style="{
      paddingRight: hasShell() && !isMac() ? CONTROLS_INSET + 'px' : undefined,
      paddingLeft: hasShell() && isMac() && !ui.showSidebar ? TRAFFIC_INSET + 'px' : undefined,
    }"
    @dblclick="dragDoubleclickMaximize"
  >
    <button class="btn icon-btn" :class="{ active: ui.showSidebar }" title="显示 / 隐藏侧栏" @click="ui.toggleSidebar()">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M9 4v16" />
      </svg>
    </button>

    <div class="topbar-spacer" />
    <span class="count-label">{{ totalLabel }}</span>
    <span v-if="importingCount > 0" class="importing">导入中 ×{{ importingCount }}</span>

    <!-- 搜索：默认图标，点击展开输入框（含书目/全文切换）；有文本时常驻 -->
    <div v-if="searchOpen || searchText" class="search-box">
      <input
        ref="searchInput"
        v-model="searchText"
        type="text"
        class="search-input"
        :placeholder="searchMode === 'meta' ? '搜索书名 / 作者 / 备注…' : '全文检索正文内容…'"
        @blur="onSearchBlur"
        @keydown.esc="onSearchEsc"
      />
      <button class="mode-btn" :title="searchMode === 'meta' ? '切换为全文检索' : '切换为书目搜索'" @click="searchMode = searchMode === 'meta' ? 'content' : 'meta'">
        {{ searchMode === 'meta' ? '书目' : '全文' }}
      </button>
    </div>
    <button
      class="btn icon-btn"
      :class="{ active: searchOpen || searchText }"
      :title="searchOpen || searchText ? '关闭搜索' : '搜索'"
      @mousedown.prevent
      @click="toggleSearch"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="11" cy="11" r="8" />
        <path d="M21 21l-4.35-4.35" />
      </svg>
    </button>

    <!-- 筛选：阅读状态 + 排序收进弹层菜单；有非默认条件时高亮 -->
    <button class="btn icon-btn" :class="{ active: readStatus }" title="筛选与排序" @click="openFilterMenu">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 3H2l8 9.46V19l4 2v-8.54L22 3z" />
      </svg>
    </button>

    <button class="btn icon-btn" :class="{ active: ui.settingsOpen }" title="设置" @click="ui.settingsOpen = true">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="3" />
        <path
          d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82.33l.06.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"
        />
      </svg>
    </button>

    <button class="btn icon-btn" :class="{ active: ui.showInspector }" title="显示 / 隐藏详情面板" @click="ui.toggleInspector()">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M15 4v16" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.topbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--panel);
  flex: none;
  /* 整条为窗口拖拽区（双击空白切换最大化），交互控件单独 no-drag */
  -webkit-app-region: drag;
}
/* 交互控件退出拖拽区域 */
.topbar button,
.topbar input {
  -webkit-app-region: no-drag;
}
/* 图标按钮：统一 28px 方形 */
.icon-btn {
  padding: 6px;
  display: grid;
  place-items: center;
  color: var(--muted);
}
.icon-btn:hover {
  color: var(--text);
}
/* 激活态（搜索展开 / 有筛选条件）以 accent 色标识 */
.icon-btn.active {
  color: var(--accent);
  border-color: var(--accent-dim);
}
.search-box {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 320px;
}
.search-input {
  flex: 1;
  min-width: 0;
}
.mode-btn {
  flex: none;
  padding: 6px 10px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--panel-2);
  color: var(--muted);
  font-size: 12px;
  cursor: pointer;
}
.mode-btn:hover {
  color: var(--text);
}
.topbar-spacer {
  flex: 1;
}
.count-label {
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}
.importing {
  font-size: 12px;
  color: var(--accent);
}
</style>
