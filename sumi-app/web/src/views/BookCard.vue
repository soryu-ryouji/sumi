<script setup lang="ts">
// 书籍卡片：封面（懒加载）+ 书名 + 阅读状态角标 + 选中态。
// 双击 = 系统默认应用打开；右键 = 上下文菜单（打开/定位/复制路径/移动/重命名/编辑元数据/移入回收站）。
// 点击多选：普通点击单选，Ctrl/Cmd 追加/移除，Shift 区间选；可拖拽到侧栏文件夹树移动（书内拖拽 MIME）。
import { computed, ref } from 'vue';
import { api, ApiError, directUrl } from '@/shared/api/client';
import { useUi } from '@/stores/ui';
import { useBooks } from '@/stores/books';
import { useConnection } from '@/stores/connection';
import { useLibrary } from '@/stores/library';
import { shell } from '@/app/shell';
import { openContextMenu, type CtxItem } from '@/shared/lib/context-menu';
import { startItemsDrag } from '@/shared/lib/dnd';
import { primaryPathOf } from '@/shared/lib/item';
import { moveItemsToFolder, renameItem } from '@/shared/lib/move';
import FolderPickerDialog from './FolderPickerDialog.vue';
import InputDialog from './InputDialog.vue';
import type { Item } from '@/shared/api/types';

const props = defineProps<{ item: Item }>();

const ui = useUi();
const books = useBooks();
const conn = useConnection();
const library = useLibrary();

const loaded = ref(false);
const failed = ref(false);

const selected = computed(() => ui.selectedIds.includes(props.item.id));
/** 移动/重命名入口仅在可写且非回收站视图出现 */
const movable = computed(() => conn.writable && !books.filter.inTrash);
/** 拖拽源条件与移动入口一致 */
const draggable = computed(() => movable.value);
/** 移动对话框当前所在文件夹（多选位置混合时不标记，传 null） */
const currentFolder = computed(() => {
  const p = primaryPathOf(props.item);
  const i = p.lastIndexOf('/');
  return ui.selectedIds.length > 1 ? null : i < 0 ? '' : p.slice(0, i);
});
/** 当前文件名（去扩展名，重命名对话框初始值；扩展名由后端沿用） */
const nameWithoutExt = computed(() => {
  const full = primaryPathOf(props.item).split('/').pop() ?? '';
  const i = full.lastIndexOf('.');
  return i > 0 ? full.slice(0, i) : full;
});

const showMoveDialog = ref(false);
const showRenameDialog = ref(false);
const coverUrl = computed(() => {
  const v = books.coverBust[props.item.id] ?? 0;
  return directUrl(`/item/cover`, { id: props.item.id, v: String(v) });
});

const STATUS_BADGE: Record<string, string> = { reading: '读', finished: '完', abandoned: '弃' };
const badge = computed(() => STATUS_BADGE[props.item.read_status] ?? '');

const primary = computed(() => primaryPathOf(props.item));

/** 刷新元数据（重建封面 + 重解析内嵌元数据，用户编辑字段受 overridden 保护） */
async function refreshMetadata(): Promise<void> {
  try {
    await api('/item/refresh_metadata', { method: 'POST', body: { id: props.item.id } });
    books.bumpCover(props.item.id);
    ui.toast('已重建封面与元数据');
  } catch (e) {
    ui.toastError(e);
  }
}

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
    // 从选择集与主选中同步移除，避免选择集残留已删除书籍
    ui.selectedIds = ui.selectedIds.filter((x) => x !== props.item.id);
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
  // 右键的书不在选择集 → 重置为单选该本；已在多选集内则保留整组
  if (!ui.selectedIds.includes(props.item.id)) {
    ui.selectOnly(props.item.id);
  }
  const items: CtxItem[] = [];
  if (conn.isAdmin) {
    items.push({ label: '打开（系统默认应用）', action: () => void openExternal() });
  }
  if (shell()) {
    items.push({ label: '在文件夹中显示', action: () => void shell()?.showInFolder(primary.value) });
    items.push({ label: '复制文件路径', action: copyPrimaryPath });
  }
  if (movable.value) {
    items.push({
      label: ui.selectedIds.length > 1 ? `移动 ${ui.selectedIds.length} 本书到文件夹…` : '移动到文件夹…',
      action: () => {
        showMoveDialog.value = true;
      },
    });
    if (ui.selectedIds.length === 1) {
      items.push({
        label: '重命名…',
        action: () => {
          showRenameDialog.value = true;
        },
      });
    }
  }
  if (conn.writable) {
    items.push({
      label: '刷新元数据',
      action: () => void refreshMetadata(),
    });
    items.push({
      label: '编辑元数据',
      action: () => {
        ui.openInspector(); // 面板隐藏时主动展开（用户明确的查看意图）
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

/** 点击选中：Shift 区间选 > Ctrl/Cmd 追加/移除 > 普通单选 */
function onClick(e: MouseEvent): void {
  if (e.shiftKey) {
    ui.rangeSelectTo(props.item.id);
  } else if (e.ctrlKey || e.metaKey) {
    ui.toggleSelect(props.item.id);
  } else {
    ui.selectOnly(props.item.id);
  }
}

/** 拖拽源：拖的书不在选择集则先单选，随后带整组 id 走书内拖拽 MIME */
function onDragStart(e: DragEvent): void {
  if (!draggable.value) {
    e.preventDefault();
    return;
  }
  if (!ui.selectedIds.includes(props.item.id)) {
    ui.selectOnly(props.item.id);
  }
  startItemsDrag(e, [...ui.selectedIds]);
}

/** 重命名：后端会自动追加扩展名，用户带扩展名输入时先剥离（避免双重后缀） */
function doRename(value: string): void {
  showRenameDialog.value = false;
  const ext = props.item.ext.toLowerCase();
  const name = ext && value.toLowerCase().endsWith(`.${ext}`) ? value.slice(0, -(ext.length + 1)) : value;
  void renameItem(props.item.id, name);
}
</script>

<template>
  <div
    class="book-card"
    :class="{ selected }"
    :draggable="draggable"
    @click.stop="onClick"
    @dblclick="conn.isAdmin && openExternal()"
    @contextmenu="onContextMenu"
    @dragstart="onDragStart"
  >
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
  <FolderPickerDialog
    v-if="showMoveDialog"
    :title="ui.selectedIds.length > 1 ? `移动 ${ui.selectedIds.length} 本书到文件夹` : '移动到文件夹'"
    :current="currentFolder"
    @confirm="(p) => { showMoveDialog = false; void moveItemsToFolder([...ui.selectedIds], p); }"
    @cancel="showMoveDialog = false"
  />
  <InputDialog
    v-if="showRenameDialog"
    title="重命名"
    :initial="nameWithoutExt"
    placeholder="文件名（扩展名自动保留）"
    confirm-label="重命名"
    @confirm="doRename"
    @cancel="showRenameDialog = false"
  />
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
