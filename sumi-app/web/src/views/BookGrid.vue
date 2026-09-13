<script setup lang="ts">
// 封面网格：skeleton 布局驱动（cover 宽高自适应占位）+ 滚动加载 + 空态/错误态。
import { onMounted, onUnmounted, ref } from 'vue';
import { useBooks } from '@/stores/books';
import { useUi } from '@/stores/ui';
import { useLibrary } from '@/stores/library';
import BookCard from './BookCard.vue';

const books = useBooks();
const ui = useUi();
const library = useLibrary();

// 冷启动：boot 已触发首载；此处兜底（直连/SSE 恢复等场景 store 仍空时）
if (!books.items.length && !books.loading && books.total === 0) {
  void books.load().catch(() => {});
}

const scroller = ref<HTMLElement | null>(null);
let observer: IntersectionObserver | null = null;

onMounted(() => {
  observer = new IntersectionObserver(
    (entries) => {
      if (entries[0]?.isIntersecting) {
        void books.loadMore();
      }
    },
    { root: scroller.value, rootMargin: '600px' },
  );
  const sentinel = scroller.value?.querySelector('.grid-sentinel');
  if (sentinel) {
    observer.observe(sentinel);
  }
});
onUnmounted(() => observer?.disconnect());

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
</script>

<template>
  <div ref="scroller" class="grid-scroller">
    <div v-if="books.error" class="grid-state error">
      <div>加载失败：{{ errorText(books.error) }}</div>
      <button class="btn" @click="books.load()">重试</button>
    </div>
    <div v-else-if="!books.loading && !books.items.length" class="grid-state">
      <div class="grid-empty-title">{{ library.libraryName || '书库' }} 还是空的</div>
      <div class="grid-empty-sub">把书籍文件拖进窗口，或点击右上角「添加书籍」</div>
    </div>
    <div v-else class="book-grid">
      <BookCard v-for="item in books.items" :key="item.id" :item="item" @open="ui.readerItemId = item.id" />
    </div>
    <div v-if="books.loading" class="grid-loading">加载中…</div>
    <div class="grid-sentinel" />
  </div>
</template>

<style scoped>
.grid-scroller {
  flex: 1;
  overflow-y: auto;
  padding: 14px;
  min-height: 0;
}
.book-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(148px, 1fr));
  gap: 14px;
}
.grid-state {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--muted);
}
.grid-state.error {
  color: var(--danger);
}
.grid-empty-title {
  font-size: 15px;
  color: var(--text);
}
.grid-empty-sub {
  font-size: 13px;
}
.grid-loading {
  text-align: center;
  padding: 14px;
  color: var(--muted);
  font-size: 12px;
}
</style>
