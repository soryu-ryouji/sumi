<script setup lang="ts">
// 顶栏：搜索（书名/作者 + 全文切换）、排序、阅读状态筛选、添加（按钮 + 拖拽入口）、换库、设置。
import { computed, ref, watch } from 'vue';
import { useBooks, ORDER_LABELS } from '@/stores/books';
import { useUi } from '@/stores/ui';
import { useConnection } from '@/stores/connection';
import { api, ApiError } from '@/shared/api/client';
import { shell } from '@/app/shell';
import { formatBytes } from '@/shared/format';

const books = useBooks();
const ui = useUi();
const conn = useConnection();

const searchText = ref('');
const searchMode = ref<'meta' | 'content'>('meta');
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

const importing = ref(0);
const fileInput = ref<HTMLInputElement | null>(null);

async function addFile(file: File): Promise<void> {
  importing.value++;
  try {
    // Electron 拖拽/选择：优先绝对路径导入（保留原文件时间，无 base64 拷贝）；
    // 浏览器形态回退 base64
    const path = (await shell()?.getPathForFile(file)) ?? '';
    const folder = books.filter.folder ?? '';
    if (path) {
      await api('/item/add', { method: 'POST', body: { path, folder_path: folder || undefined } });
    } else {
      const b64 = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(file);
      });
      await api('/item/add', { method: 'POST', body: { file_base64: b64, name: file.name, folder_path: folder || undefined } });
    }
  } catch (e) {
    ui.toastError(e instanceof ApiError ? `${e.message}（${file.name}）` : e);
  } finally {
    importing.value--;
  }
}

function onPick(e: Event): void {
  const files = Array.from((e.target as HTMLInputElement).files ?? []);
  void Promise.all(files.map(addFile));
  (e.target as HTMLInputElement).value = '';
}

// 主界面整窗拖拽导入
function onDrop(e: DragEvent): void {
  const files = Array.from(e.dataTransfer?.files ?? []);
  if (files.length) {
    e.preventDefault();
    void Promise.all(files.map(addFile));
  }
}

const totalLabel = computed(() => `${books.total} 本 · ${formatBytes(books.totalSize)}`);

function toggleOrder(): void {
  books.filter.order = books.filter.order === 'desc' ? 'asc' : 'desc';
  void books.load();
}
</script>

<template>
  <div class="topbar" @drop="onDrop" @dragover.prevent>
    <div class="search-box">
      <input v-model="searchText" type="text" class="search-input" :placeholder="searchMode === 'meta' ? '搜索书名 / 作者 / 备注…' : '全文检索正文内容…'" />
      <button class="mode-btn" :title="searchMode === 'meta' ? '切换为全文检索' : '切换为书目搜索'" @click="searchMode = searchMode === 'meta' ? 'content' : 'meta'">
        {{ searchMode === 'meta' ? '书目' : '全文' }}
      </button>
    </div>

    <select v-model="readStatus" class="status-select" title="阅读状态">
      <option v-for="o in readStatusOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
    </select>

    <select v-model="books.filter.orderBy" class="order-select" title="排序" @change="books.load()">
      <option v-for="(label, value) in ORDER_LABELS" :key="value" :value="value">{{ label }}</option>
    </select>
    <button class="order-dir" :title="books.filter.order === 'desc' ? '降序' : '升序'" @click="toggleOrder">
      {{ books.filter.order === 'desc' ? '↓' : '↑' }}
    </button>

    <div class="topbar-spacer" />
    <span class="count-label">{{ totalLabel }}</span>
    <span v-if="importing > 0" class="importing">导入中 ×{{ importing }}</span>

    <template v-if="conn.writable">
      <input ref="fileInput" type="file" multiple hidden accept=".epub,.pdf,.txt,.md,.mobi,.azw3,.docx,.cbz" @change="onPick" />
      <button class="btn primary" @click="fileInput?.click()">添加书籍</button>
    </template>
    <button class="btn" title="切换书库" @click="shell()?.selectLibrary()">换库</button>
    <button class="btn" title="设置" @click="ui.view = 'settings'">设置</button>
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
}
.search-box {
  display: flex;
  align-items: center;
  flex: 1;
  max-width: 420px;
  gap: 6px;
}
.search-input {
  flex: 1;
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
.status-select,
.order-select {
  max-width: 110px;
}
.order-dir {
  width: 30px;
  padding: 6px 0;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--panel-2);
  color: var(--text);
  cursor: pointer;
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
