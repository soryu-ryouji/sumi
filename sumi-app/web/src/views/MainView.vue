<script setup lang="ts">
// 主界面三栏布局：侧栏（导航/筛选）+ 顶栏（搜索/操作）+ 内容区（网格 + 详情侧板）。
import Sidebar from './Sidebar.vue';
import TopBar from './TopBar.vue';
import BookGrid from './BookGrid.vue';
import Inspector from './Inspector.vue';
</script>

<template>
  <div class="main-view">
    <Sidebar />
    <div class="main-center">
      <TopBar />
      <BookGrid />
    </div>
    <Inspector />
  </div>
</template>

<style scoped>
.main-view {
  flex: 1;
  display: flex;
  min-height: 0;
}
.main-center {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}
</style>
