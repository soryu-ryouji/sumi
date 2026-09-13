<script setup lang="ts">
// 引导页：无书库配置时的入口（选择目录 / 打开历史书库）。
import { onMounted, ref } from 'vue';
import { shell } from '../shell';
import DragBar from '../chrome/DragBar.vue';
import type { LibraryList } from '@/shared/api/types';

const list = ref<LibraryList | null>(null);
const busy = ref(false);

onMounted(refresh);

async function refresh(): Promise<void> {
  list.value = (await shell()?.listLibraries()) ?? null;
}

async function select(): Promise<void> {
  busy.value = true;
  try {
    // 就绪经 onServerStarted 通知（boot 已订阅），这里只管发起
    await shell()?.selectLibrary();
  } finally {
    busy.value = false;
  }
}

async function open(path: string): Promise<void> {
  busy.value = true;
  try {
    await shell()?.openLibrary(path);
  } finally {
    busy.value = false;
  }
}

async function remove(path: string): Promise<void> {
  list.value = (await shell()?.removeLibrary(path)) ?? list.value;
}
</script>

<template>
  <div class="connect-screen">
    <DragBar />
    <div class="connect-center">
    <img src="/icon.png" alt="sumi" class="connect-logo" />
    <h1>sumi</h1>
    <p class="connect-sub">选择一个文件夹作为书库——书籍文件留在原地，sumi 的数据全部收在 .sumi/ 隐藏目录里。</p>
    <button class="btn primary" :disabled="busy" @click="select">选择书库目录…</button>

    <div v-if="list?.libraries.length" class="connect-history">
      <div class="connect-history-title">最近打开</div>
      <div v-for="lib in list.libraries" :key="lib.path" class="connect-history-item" :class="{ disabled: !lib.exists }">
        <button class="history-open" :disabled="!lib.exists || busy" @click="open(lib.path)">
          <span class="history-name">{{ lib.name }}</span>
          <span class="history-path">{{ lib.path }}</span>
        </button>
        <button class="history-remove" title="从历史移除" @click="remove(lib.path)">✕</button>
      </div>
    </div>
    </div>
  </div>
</template>

<style scoped>
.connect-screen {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.connect-center {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  padding: 40px;
}
.connect-logo {
  width: 88px;
  height: 88px;
  border-radius: 20px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}
h1 {
  margin: 0;
  font-size: 28px;
  font-weight: 600;
}
.connect-sub {
  margin: 0;
  max-width: 420px;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.7;
}
.connect-history {
  margin-top: 18px;
  width: 480px;
  max-width: 90vw;
}
.connect-history-title {
  font-size: 12px;
  color: var(--muted);
  margin-bottom: 6px;
}
.connect-history-item {
  display: flex;
  align-items: center;
  border: 1px solid var(--border);
  border-radius: 8px;
  margin-bottom: 6px;
  overflow: hidden;
}
.connect-history-item.disabled {
  opacity: 0.45;
}
.history-open {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: 8px 12px;
  background: transparent;
  border: none;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.history-open:hover {
  background: var(--hover);
}
.history-name {
  font-size: 13px;
}
.history-path {
  font-size: 11px;
  color: var(--muted);
}
.history-remove {
  padding: 8px 12px;
  border: none;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
}
.history-remove:hover {
  color: var(--danger);
  background: var(--hover);
}
</style>
