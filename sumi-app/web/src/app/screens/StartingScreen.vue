<script setup lang="ts">
// 启动屏：初始索引进度（sync/scan/hash/apply 四阶段，total=0 为不定态）。
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { shell } from '../shell';

const phase = ref('');
const processed = ref(0);
const total = ref(0);
let off: (() => void) | undefined;

onMounted(() => {
  off = shell()?.onServerProgress((p) => {
    phase.value = p.phase;
    processed.value = p.processed;
    total.value = p.total;
  });
});
onUnmounted(() => off?.());

const PHASE_LABEL: Record<string, string> = {
  sync: '读取书目数据',
  scan: '遍历书库目录',
  hash: '计算内容指纹',
  apply: '更新索引',
};

const percent = computed(() => (total.value > 0 ? Math.min(100, Math.round((processed.value / total.value) * 100)) : null));
</script>

<template>
  <div class="starting-screen">
    <img src="/icon.png" alt="sumi" class="starting-logo" />
    <div class="starting-title">正在建立索引</div>
    <div class="starting-phase">{{ PHASE_LABEL[phase] ?? '准备中' }}</div>
    <div class="starting-bar">
      <div class="starting-bar-fill" :class="{ indeterminate: percent === null }" :style="percent !== null ? { width: percent + '%' } : {}" />
    </div>
    <div class="starting-count">{{ percent !== null ? `${percent}%（${processed}/${total}）` : '扫描书库中…' }}</div>
  </div>
</template>

<style scoped>
.starting-screen {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
}
.starting-logo {
  width: 72px;
  height: 72px;
  border-radius: 16px;
  margin-bottom: 8px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}
.starting-title {
  font-size: 16px;
  font-weight: 600;
}
.starting-phase {
  font-size: 13px;
  color: var(--muted);
}
.starting-bar {
  width: 320px;
  max-width: 70vw;
  height: 4px;
  border-radius: 2px;
  background: var(--border);
  overflow: hidden;
  margin-top: 10px;
}
.starting-bar-fill {
  height: 100%;
  background: var(--accent);
  transition: width 0.2s ease;
  border-radius: 2px;
}
.starting-bar-fill.indeterminate {
  width: 36%;
  animation: slide 1.2s ease-in-out infinite alternate;
}
@keyframes slide {
  from {
    transform: translateX(-40px);
  }
  to {
    transform: translateX(320px);
  }
}
.starting-count {
  font-size: 12px;
  color: var(--muted);
}
</style>
