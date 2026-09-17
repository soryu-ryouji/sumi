<script setup lang="ts">
// 通用顶部拖拽条：全屏视图（引导页/启动屏/回收站/设置/阅读器）的窗口拖拽区。
// macOS 左端避让原生红绿灯（TRAFFIC_INSET），Windows/Linux 右端避让 fixed 自绘控制（CONTROLS_INSET）；
// 纯浏览器无窗口控件，无避让。双击空白切换最大化（macOS 惯例）。
import { hasShell, isMac, TRAFFIC_INSET, CONTROLS_INSET, dragDoubleclickMaximize } from '@/shared/lib/platform';

defineProps<{ title?: string }>();

// 无需避让的一侧不要给内联 padding：内联 0px 会覆盖 CSS 的 padding: 0 12px，标题/内容贴边
const padLeft = hasShell() && isMac() ? `${TRAFFIC_INSET}px` : undefined;
const padRight = hasShell() && !isMac() ? `${CONTROLS_INSET}px` : undefined;
</script>

<template>
  <div class="drag-bar" :style="{ paddingLeft: padLeft, paddingRight: padRight }" @dblclick="dragDoubleclickMaximize">
    <span v-if="title" class="drag-title">{{ title }}</span>
    <slot />
  </div>
</template>

<style scoped>
.drag-bar {
  flex: none;
  height: 44px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px;
  border-bottom: 1px solid var(--border);
  background: var(--panel);
  -webkit-app-region: drag;
}
/* 拖拽区内的交互控件退出拖拽 */
.drag-bar :deep(button),
.drag-bar :deep(input),
.drag-bar :deep(select) {
  -webkit-app-region: no-drag;
}
.drag-title {
  font-size: 14px;
  font-weight: 600;
}
</style>
