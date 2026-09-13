<script setup lang="ts">
// 书籍卡片：封面（懒加载）+ 书名 + 阅读状态角标 + 选中态。
// 双击 = 系统默认应用打开；右键 = 上下文菜单（打开/定位/复制路径/编辑元数据/移入回收站）。
import { computed, ref } from 'vue';
import { api, ApiError, directUrl } from '@/shared/api/client';
import { useUi } from '@/stores/ui';
import { useBooks } from '@/stores/books';
import { useConnection } from '@/stores/connection';
import { useLibrary } from '@/stores/library';
import { shell } from '@/app/shell';
import { openContextMenu, type CtxItem } from '@/shared/lib/context-menu';
import type { Item } from '@/shared/api/types';

const props = defineProps<{ item: Item }>();

const ui = useUi();
const books = useBooks();
const conn = useConnection();
const library = useLibrary();

const loaded = ref(false);
const failed = ref(false);

const selected = computed(() => ui.selectedId === props.item.id);
const coverUrl = computed(() => directUrl(`/item/cover`, { id: props.item.id }));

const STATUS_BADGE: Record<string, string> = { reading: '读', finished: '完', abandoned: '弃' };
const badge = computed(() => STATUS_BADGE[props.item.read_status] ?? '');

const primary = computed(() => props.item.paths[0] ?? '');

/** 系统默认应用打开（item/open，admin 限定） */
async function openExternal(): Promise<void> {
  try {
    await api('/item/open', { method: 'POST', body: { id: props.item.id } });
  } catch (e) {
    ui.toastError(e instanceof ApiError ? `打开失败：${e.message}` : e);
  }
}

async function trash(): Promise<void> {
  try {
    await api('/item/delete', { method: 'POST', body: { id: props.item.id } });
    books.dropItem(props.item.id);
    if (ui.selectedId === props.item.id) {
      ui.selectedId = null;
    }
    library.refreshAll().catch(() => {});
  } catch (e) {
    ui.toastError(e);
  }
}

function copyPrimaryPath(): void {
  if (shell()) {
    void shell()?.copyPath(primary.value).then(() => ui.toast('路径已复制'));
  }
}

function onContextMenu(e: MouseEvent): void {
  ui.selectedId = props.item.id;
  const items: CtxItem[] = [];
  if (conn.isAdmin) {
    items.push({ label: '打开（系统默认应用）', action: () => void openExternal() });
  }
  if (shell()) {
    items.push({ label: '在文件夹中显示', action: () => void shell()?.showInFolder(primary.value) });
    items.push({ label: '复制文件路径', action: copyPrimaryPath });
  }
  if (conn.writable) {
    items.push({
      label: '编辑元数据',
      action: () => {
        ui.inspectorEdit = true;
      },
    });
    items.push({ separator: true });
    items.push({ label: '移入回收站', danger: true, action: () => void trash() });
  }
  if (items.length) {
    openContextMenu(items, e);
  }
}
</script>

<template>
  <div class="book-card" :class="{ selected }" @click="ui.selectedId = item.id" @dblclick="conn.isAdmin && openExternal()" @contextmenu="onContextMenu">
    <div class="cover-wrap">
      <img
        v-if="!failed"
        :src="coverUrl"
        :alt="item.title"
        loading="lazy"
        :class="{ loaded: loaded }"
        @load="loaded = true"
        @error="failed = true"
      />
      <div v-if="failed" class="cover-fallback">
        <span>{{ item.ext.toUpperCase() }}</span>
      </div>
      <span v-if="badge" class="status-badge" :class="item.read_status">{{ badge }}</span>
    </div>
    <div class="card-title" :title="item.title">{{ item.title || item.name }}</div>
    <div class="card-sub" :title="item.authors.join(', ')">{{ item.authors.join(', ') || '—' }}</div>
    <div v-if="item.star > 0" class="card-star">{{ '★'.repeat(item.star) }}</div>
  </div>
</template>

<style scoped>
.book-card {
  cursor: pointer;
  border-radius: 10px;
  padding: 8px;
  transition: background 0.12s;
  min-width: 0;
}
.book-card:hover {
  background: var(--hover);
}
.book-card.selected {
  background: var(--hover);
  outline: 2px solid var(--accent-dim);
}
.cover-wrap {
  position: relative;
  aspect-ratio: 2 / 3;
  border-radius: 6px;
  overflow: hidden;
  background: #2c2c30;
  display: grid;
  place-items: center;
}
img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  opacity: 0;
  transition: opacity 0.2s;
}
img.loaded {
  opacity: 1;
}
.cover-fallback {
  font-size: 22px;
  color: var(--muted);
  font-weight: 600;
}
.status-badge {
  position: absolute;
  top: 6px;
  right: 6px;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 11px;
  color: #fff;
}
.status-badge.reading {
  background: var(--accent-dim);
}
.status-badge.finished {
  background: var(--ok);
}
.status-badge.abandoned {
  background: #777;
}
.card-title {
  margin-top: 6px;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.card-sub {
  font-size: 11px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.card-star {
  font-size: 11px;
  color: #e0b45e;
}
</style>
