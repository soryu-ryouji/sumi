<script setup lang="ts">
// 文件夹选择对话框的树节点（递归；SFC 按文件名隐式自引用）。
// 与侧栏 FolderTreeNode 的区别：单击行 = 选中（不发筛选），当前所在文件夹带标记。
import { ref } from 'vue';
import type { FolderNode } from '@/shared/api/types';

const props = defineProps<{ node: FolderNode; selectedPath: string; current: string | null }>();
const emit = defineEmits<{ select: [path: string] }>();
const expanded = ref(true);
const hasChildren = () => props.node.children.length > 0;
</script>

<template>
  <div class="picker-node">
    <div class="picker-row" :class="{ selected: selectedPath === node.path }" @click="emit('select', node.path)">
      <span v-if="hasChildren()" class="picker-caret" :class="{ open: expanded }" @click.stop="expanded = !expanded">▸</span>
      <span v-else class="picker-caret placeholder" />
      <span class="picker-name" :title="node.path">{{ node.name }}</span>
      <span v-if="current === node.path" class="picker-current">当前</span>
    </div>
    <div v-if="expanded && hasChildren()" class="picker-children">
      <FolderPickerNode v-for="child in node.children" :key="child.path" :node="child" :selected-path="selectedPath" :current="current" @select="(p) => emit('select', p)" />
    </div>
  </div>
</template>

<style scoped>
.picker-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
  min-width: 0;
}
.picker-row:hover {
  background: var(--hover);
}
.picker-row.selected {
  background: var(--accent-dim);
  color: #fff;
}
.picker-caret {
  width: 12px;
  flex: none;
  color: var(--muted);
  font-size: 10px;
  transition: transform 0.12s;
  text-align: center;
}
.picker-caret.open {
  transform: rotate(90deg);
}
.picker-caret.placeholder {
  visibility: hidden;
}
.picker-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.picker-current {
  flex: none;
  margin-left: auto;
  font-size: 10px;
  color: var(--muted);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0 4px;
}
.picker-children {
  padding-left: 14px;
}
</style>
