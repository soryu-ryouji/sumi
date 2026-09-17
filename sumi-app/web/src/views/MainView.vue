<script setup lang="ts">
// 主界面：侧栏（导航/筛选，通高）+ 右半（顶栏通栏 + 内容行：网格/回收站 + 详情侧板）。
// 顶栏通栏覆盖详情侧板上方，窗口控制避让由顶栏统一处理，详情侧板不再自带顶部占位条。
// 左右侧栏显隐由顶栏开关控制（ui.showSidebar / ui.showInspector，持久化）。
// 回收站不整页跳转：内容区随 books.filter.inTrash 在网格与回收站列表间切换（侧栏导航置位）。
// 整个主界面为拖拽导入区（拖文件落入即导入）。
import { useUi } from '@/stores/ui';
import { useBooks } from '@/stores/books';
import { importBooks } from '@/shared/lib/importer';
import Sidebar from './Sidebar.vue';
import TopBar from './TopBar.vue';
import BookGrid from './BookGrid.vue';
import TrashView from './TrashView.vue';
import Inspector from './Inspector.vue';

const ui = useUi();
const books = useBooks();

// 主界面整窗拖拽导入
function onDrop(e: DragEvent): void {
  const files = Array.from(e.dataTransfer?.files ?? []);
  if (files.length) {
    e.preventDefault();
    importBooks(files);
  }
}
</script>

<template>
  <div class="main-view" @drop="onDrop" @dragover.prevent>
    <Sidebar v-if="ui.showSidebar" />
    <div class="main-body">
      <TopBar />
      <div class="main-center">
        <TrashView v-if="books.filter.inTrash" />
        <BookGrid v-else />
        <Inspector />
      </div>
    </div>
  </div>
</template>

<style scoped>
.main-view {
  flex: 1;
  display: flex;
  min-height: 0;
}
.main-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.main-center {
  flex: 1;
  display: flex;
  min-height: 0;
}
</style>
