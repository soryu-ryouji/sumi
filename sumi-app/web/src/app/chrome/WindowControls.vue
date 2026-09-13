<script setup lang="ts">
// Windows/Linux 自绘窗口控制钮（macOS 用系统红绿灯，不渲染本组件）。
defineProps<{ maximized: boolean }>();
defineEmits<{ minimize: []; maximize: []; close: [] }>();
</script>

<template>
  <div class="win-controls">
    <button class="win-btn" title="最小化" @click="$emit('minimize')">
      <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
    </button>
    <button class="win-btn" :title="maximized ? '还原' : '最大化'" @click="$emit('maximize')">
      <svg v-if="!maximized" width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" /></svg>
      <svg v-else width="10" height="10" viewBox="0 0 10 10">
        <rect x="0.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" />
        <path d="M2.5 2.5v-2h7v7h-2" fill="none" stroke="currentColor" />
      </svg>
    </button>
    <button class="win-btn close" title="关闭" @click="$emit('close')">
      <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg>
    </button>
  </div>
</template>

<style scoped>
.win-controls {
  display: flex;
  -webkit-app-region: no-drag;
}
.win-btn {
  width: 44px;
  height: 100%;
  display: grid;
  place-items: center;
  border: none;
  background: transparent;
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
