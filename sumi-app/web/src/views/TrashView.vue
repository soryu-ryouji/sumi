<script setup lang="ts">
// 回收站列表：内嵌主界面内容区（由侧栏「回收站」导航置 books.filter.inTrash 后渲染，非独立视图）。
// 顶部工具行提供计数与清空（不可恢复操作有确认）；单本恢复走行内按钮。
import { ref } from 'vue';
import { useBooks } from '@/stores/books';
import { useUi } from '@/stores/ui';
import { useLibrary } from '@/stores/library';
import { useConnection } from '@/stores/connection';
import { api, directUrl } from '@/shared/api/client';
import { formatTime } from '@/shared/format';
import { primaryPathOf } from '@/shared/lib/item';
import type { Item } from '@/shared/api/types';

const books = useBooks();
const ui = useUi();
const library = useLibrary();
const conn = useConnection();
const clearing = ref(false);

async function restore(item: Item): Promise<void> {
  try {
    await api('/item/restore', { method: 'POST', body: { id: item.id } });
    books.dropItem(item.id);
  } catch (e) {
    ui.toastError(e);
  }
}

async function clearAll(): Promise<void> {
  if (!window.confirm('彻底删除回收站内的全部内容？此操作不可恢复（原文件将被删除）。')) {
    return;
  }
  clearing.value = true;
  try {
    await api('/trash/clear', { method: 'POST' });
    books.items = [];
    books.total = 0;
    library.refreshAll().catch(() => {});
    ui.toast('回收站已清空');
  } catch (e) {
    ui.toastError(e);
  } finally {
    clearing.value = false;
  }
}
</script>

<template>
  <div class="trash-view">
    <!-- 内嵌工具行（非拖拽区）：返回书库靠侧栏导航（「全部书籍」/任意筛选退出回收站） -->
    <div class="trash-toolbar">
      <span class="trash-count">共 {{ books.total }} 本</span>
      <div class="spacer" />
      <button v-if="conn.writable && books.total > 0" class="btn danger" :disabled="clearing" @click="clearAll">清空回收站</button>
    </div>
    <div class="trash-list">
      <div v-for="item in books.items" :key="item.id" class="trash-item">
        <img class="trash-cover" :src="directUrl('/item/cover', { id: item.id })" :alt="item.title" loading="lazy" />
        <div class="trash-info">
          <div class="trash-name">{{ item.title || item.name }}</div>
          <div class="trash-path">{{ primaryPathOf(item) }}</div>
          <div class="trash-time">移入于 {{ formatTime(item.added_time) }}</div>
        </div>
        <button v-if="conn.writable" class="btn" @click="restore(item)">恢复</button>
      </div>
      <div v-if="!books.items.length && !books.loading" class="trash-empty">回收站是空的</div>
    </div>
  </div>
</template>

<style scoped>
.trash-view {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.trash-toolbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
}
.trash-count {
  color: var(--muted);
  font-size: 12px;
}
.spacer {
  flex: 1;
}
.trash-list {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.trash-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--panel);
}
.trash-cover {
  width: 40px;
  height: 56px;
  object-fit: contain;
  border-radius: 4px;
  background: #2c2c30;
  flex: none;
}
.trash-info {
  flex: 1;
  min-width: 0;
}
.trash-name {
  font-size: 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.trash-path {
  font-size: 11px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.trash-time {
  font-size: 11px;
  color: var(--muted);
}
.trash-empty {
  padding: 60px;
  text-align: center;
  color: var(--muted);
}
</style>
