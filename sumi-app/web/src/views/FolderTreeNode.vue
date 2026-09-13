<script setup lang="ts">
// 文件夹树节点（递归；SFC 按文件名隐式自引用）。
import { ref } from 'vue';
import type { FolderNode } from '@/shared/api/types';

const props = defineProps<{ node: FolderNode; active: string | null }>();
const emit = defineEmits<{ select: [path: string] }>();
const expanded = ref(true);
const hasChildren = () => props.node.children.length > 0;
</script>

<template>
  <div class="tree-node">
    <div class="tree-row" :class="{ active: active === node.path }" @click="emit('select', node.path)">
      <span v-if="hasChildren()" class="tree-caret" :class="{ open: expanded }" @click.stop="expanded = !expanded">▸</span>
      <span v-else class="tree-caret placeholder" />
      <span class="tree-name" :title="node.path">{{ node.name }}</span>
    </div>
    <div v-if="expanded && hasChildren()" class="tree-children">
      <FolderTreeNode v-for="child in node.children" :key="child.path" :node="child" :active="active" @select="(p) => emit('select', p)" />
    </div>
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
