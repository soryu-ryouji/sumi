<script setup lang="ts">
// 自绘标题栏：mac 只承担拖拽区（红绿灯为系统原生）；Windows/Linux 左侧品牌、右侧窗口控制。
import { onMounted, onUnmounted, ref } from 'vue';
import { shell } from '../shell';
import WindowControls from './WindowControls.vue';

const isMac = shell()?.platform === 'darwin';
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
  <header class="titlebar" :class="{ mac: isMac }">
    <div class="titlebar-drag">
      <template v-if="!isMac">
        <img src="/icon.png" alt="" class="titlebar-logo" />
        <span class="titlebar-title">sumi</span>
      </template>
    </div>
    <WindowControls v-if="!isMac" :maximized="maximized" @minimize="shell()?.minimizeWindow()" @maximize="toggleMaximize" @close="shell()?.closeWindow()" />
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  align-items: stretch;
  height: 38px;
  flex: none;
  background: var(--panel);
  border-bottom: 1px solid var(--border);
  user-select: none;
}
.titlebar.mac {
  background: transparent;
  border-bottom: none;
  position: absolute;
  inset: 0 0 auto 0;
  z-index: 10;
  pointer-events: none;
}
.titlebar.mac :deep(.titlebar-drag) {
  pointer-events: none;
}
.titlebar-drag {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  -webkit-app-region: drag;
  min-width: 0;
}
.titlebar-logo {
  width: 18px;
  height: 18px;
  border-radius: 4px;
}
.titlebar-title {
  font-size: 12px;
  color: var(--muted);
}
</style>
