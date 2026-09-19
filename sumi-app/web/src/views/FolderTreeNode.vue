<script setup lang="ts">
// 文件夹树节点（递归；SFC 按文件名隐式自引用）。
// 可写时是移动目标：书籍卡片拖入（书内拖拽 MIME）即移动到本文件夹；右键 = 新建子文件夹/重命名/删除。
import { ref } from 'vue';
import type { FolderNode } from '@/shared/api/types';
import { useConnection } from '@/stores/connection';
import { hasItemsDrag, readItemsDrop } from '@/shared/lib/dnd';
import { createFolder, deleteFolder, moveItemsToFolder, renameFolder } from '@/shared/lib/move';
import { openContextMenu } from '@/shared/lib/context-menu';
import InputDialog from './InputDialog.vue';
import ConfirmDialog from './ConfirmDialog.vue';

const props = defineProps<{ node: FolderNode; active: string | null }>();
const emit = defineEmits<{ select: [path: string] }>();

const conn = useConnection();
const expanded = ref(true);
const hasChildren = () => props.node.children.length > 0;

/** 书内拖拽悬停高亮（仅可写时成为放置目标） */
const dragOver = ref(false);

function onDragOver(e: DragEvent): void {
  // 非书内拖拽不拦截：让外部文件拖入冒泡到整窗导入
  if (!conn.writable || !hasItemsDrag(e)) {
    return;
  }
  e.preventDefault();
  e.dataTransfer && (e.dataTransfer.dropEffect = 'move');
  dragOver.value = true;
}

function onDrop(e: DragEvent): void {
  dragOver.value = false;
  const ids = readItemsDrop(e);
  if (!ids || !conn.writable) {
    return;
  }
  // 只让最内层节点处理，避免冒泡触发外层（.tree-row 非嵌套，此处主要拦住冒到整窗的路径）
  e.stopPropagation();
  e.preventDefault();
  void moveItemsToFolder(ids, props.node.path);
}

// 右键文件夹管理（对话框状态放节点局部：递归组件各自持有，右键谁显示谁的）
const showNewSub = ref(false);
const showRename = ref(false);
const showDelete = ref(false);

function onContextMenu(e: MouseEvent): void {
  if (!conn.writable) {
    return;
  }
  openContextMenu(
    [
      {
        label: '新建子文件夹…',
        action: () => {
          showNewSub.value = true;
        },
      },
      {
        label: '重命名…',
        action: () => {
          showRename.value = true;
        },
      },
      { separator: true },
      {
        label: '删除文件夹…',
        danger: true,
        action: () => {
          showDelete.value = true;
        },
      },
    ],
    e,
  );
}
</script>

<template>
  <div class="tree-node">
    <div
      class="tree-row"
      :class="{ active: active === node.path, 'drop-target': dragOver }"
      @click="emit('select', node.path)"
      @contextmenu="onContextMenu"
      @dragover="onDragOver"
      @dragleave="dragOver = false"
      @drop="onDrop"
    >
      <span v-if="hasChildren()" class="tree-caret" :class="{ open: expanded }" @click.stop="expanded = !expanded">▸</span>
      <span v-else class="tree-caret placeholder" />
      <span class="tree-name" :title="node.path">{{ node.name }}</span>
    </div>
    <div v-if="expanded && hasChildren()" class="tree-children">
      <FolderTreeNode v-for="child in node.children" :key="child.path" :node="child" :active="active" @select="(p) => emit('select', p)" />
    </div>
    <InputDialog
      v-if="showNewSub"
      title="新建子文件夹"
      placeholder="文件夹名"
      confirm-label="创建"
      @confirm="(v) => { showNewSub = false; void createFolder(`${node.path}/${v}`); }"
      @cancel="showNewSub = false"
    />
    <InputDialog
      v-if="showRename"
      title="重命名文件夹"
      :initial="node.name"
      placeholder="文件夹名"
      confirm-label="重命名"
      @confirm="(v) => { showRename = false; void renameFolder(node.path, v); }"
      @cancel="showRename = false"
    />
    <ConfirmDialog
      v-if="showDelete"
      title="删除文件夹"
      :message="`将文件夹「${node.name}」连同其中全部书籍移入回收站？`"
      danger
      confirm-label="移入回收站"
      @confirm="() => { showDelete = false; void deleteFolder(node.path); }"
      @cancel="showDelete = false"
    />
  </div>
</template>

<style scoped>
.tree-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
  min-width: 0;
}
.tree-row:hover {
  background: var(--hover);
}
.tree-row.active {
  background: var(--accent-dim);
  color: #fff;
}
/* 书内拖拽悬停：整行高亮为放置目标（优先级压过 hover/active） */
.tree-row.drop-target,
.tree-row.drop-target:hover {
  background: var(--accent-dim);
  outline: 1px dashed var(--accent-dim);
  color: #fff;
}
.tree-caret {
  width: 12px;
  flex: none;
  color: var(--muted);
  font-size: 10px;
  transition: transform 0.12s;
  text-align: center;
}
.tree-caret.open {
  transform: rotate(90deg);
}
.tree-caret.placeholder {
  visibility: hidden;
}
.tree-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tree-children {
  padding-left: 14px;
}
</style>
