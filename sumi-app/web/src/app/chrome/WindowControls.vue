<script setup lang="ts">
// Windows/Linux 自绘窗口控制钮：fixed 窗口右上角（覆盖所有视图，内容区按 CONTROLS_INSET 避让）。
// macOS 用系统原生红绿灯（titleBarStyle: 'hidden'，压在侧栏顶部拖拽条左侧），本组件不渲染；
// 纯浏览器（无 Electron 壳）无窗口控制，同样不渲染。
import { onMounted, onUnmounted, ref } from 'vue';
import { shell } from '@/app/shell';
import { CONTROLS_HEIGHT } from '@/shared/lib/platform';

const visible = shell() !== null && shell()?.platform !== 'darwin';
const maximized = ref(false);
let off: (() => void) | undefined;
onMounted(() => {
  off = shell()?.onWindowMaximized((v) => {
    maximized.value = v;
  });
});
onUnmounted(() => off?.());

function toggleMaximize(): void {
  void shell()?.toggleMaximizeWindow().then((v) => {
    maximized.value = v;
  });
}
</script>

<template>
  <div v-if="visible" class="win-controls" :style="{ height: CONTROLS_HEIGHT + 'px' }">
    <button class="win-btn" title="最小化" @click="shell()?.minimizeWindow()">
      <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
    </button>
    <button class="win-btn" :title="maximized ? '还原' : '最大化'" @click="toggleMaximize">
      <svg v-if="!maximized" width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" /></svg>
      <svg v-else width="10" height="10" viewBox="0 0 10 10">
        <rect x="0.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" />
        <path d="M2.5 2.5v-2h7v7h-2" fill="none" stroke="currentColor" />
      </svg>
    </button>
    <button class="win-btn close" title="关闭" @click="shell()?.closeWindow()">
      <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg>
    </button>
  </div>
</template>

<style scoped>
.win-controls {
  position: fixed;
  top: 0;
  right: 0;
  z-index: 500;
  display: flex;
  -webkit-app-region: no-drag;
}
.win-btn {
  width: 44px;
  height: 100%;
  display: grid;
  place-items: center;
  border: none;
  background: var(--panel);
  color: var(--muted);
  cursor: default;
}
.win-btn:hover {
  background: var(--hover);
  color: var(--text);
}
.win-btn.close:hover {
  background: #e81123;
  color: #fff;
}
</style>
