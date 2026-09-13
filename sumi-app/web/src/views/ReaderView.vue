<script setup lang="ts">
// 阅读器：txt/md 按 toc 章节切分渲染（char 锚点对位）；epub/docx/mobi 走归一化 HTML（srcdoc，
// data-href 对位 toc）；pdf/cbz 提示走系统打开。进度/状态随手保存。
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useUi } from '@/stores/ui';
import { useBooks } from '@/stores/books';
import { useConnection } from '@/stores/connection';
import { api, directUrl } from '@/shared/api/client';
import type { Item, TocEntry } from '@/shared/api/types';

const ui = useUi();
const books = useBooks();
const conn = useConnection();

const item = computed<Item | null>(() => books.items.find((i) => i.id === ui.readerItemId) ?? null);
const toc = ref<TocEntry[]>([]);
const textContent = ref<string | null>(null); // txt/md
const htmlContent = ref<string | null>(null); // epub/docx/mobi
const loadError = ref('');
const iframe = ref<HTMLIFrameElement | null>(null);

const mode = computed<'text' | 'html' | 'unsupported' | null>(() => {
  if (!item.value) {
    return null;
  }
  const ext = item.value.ext;
  if (ext === 'txt' || ext === 'md') {
    return 'text';
  }
  if (ext === 'epub' || ext === 'docx' || ext === 'mobi' || ext === 'azw3') {
    return 'html';
  }
  return 'unsupported';
});

let saveTimer: ReturnType<typeof setTimeout> | undefined;

onMounted(async () => {
  if (!item.value) {
    return;
  }
  try {
    toc.value = await api<TocEntry[]>(`/item/toc?id=${encodeURIComponent(item.value.id)}`);
    if (mode.value === 'text') {
      const res = await fetch(directUrl(`/item/content`, { id: item.value.id }));
      if (!res.ok) {
        throw new Error(`正文获取失败（HTTP ${res.status}）`);
      }
      textContent.value = await res.text();
    } else if (mode.value === 'html') {
      const res = await fetch(directUrl(`/item/content`, { id: item.value.id }));
      if (!res.ok) {
        const body = await res.json().catch(() => null);
        throw new Error(body?.error?.message ?? `正文获取失败（HTTP ${res.status}）`);
      }
      let html = await res.text();
      // 注入阅读器样式与暗色适配（srcdoc 内无外部样式）
      html = html.replace(
        '</head>',
        `<style>
          body { margin: 0 auto; padding: 40px 24px; max-width: 720px; background: #1a1a1c; color: #d8d8dc;
                font: 16px/1.9 -apple-system, 'PingFang SC', 'Microsoft YaHei', serif; }
          img { max-width: 100%; }
          section { scroll-margin-top: 16px; }
        </style></head>`,
      );
      htmlContent.value = html;
    }
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e);
  }

  // 阅读即更新状态（在读）与最近阅读时间
  if (conn.writable && item.value && item.value.read_status === 'unread') {
    void api('/item/update', { method: 'POST', body: { id: item.value.id, read_status: 'reading' } }).catch(() => {});
  }
});

// txt/md 章节切分：toc 的 char-N 锚点把全文切成段，每段可定位
interface Segment {
  offset: number;
  title: string;
  text: string;
}
const segments = computed<Segment[]>(() => {
  const text = textContent.value;
  if (text === null) {
    return [];
  }
  const flat: { offset: number; title: string }[] = [];
  const walk = (entries: TocEntry[]) => {
    for (const e of entries) {
      const m = e.anchor.match(/^anchor:char-(\d+)$/);
      if (m) {
        flat.push({ offset: Number(m[1]), title: e.title });
      }
      walk(e.children);
    }
  };
  walk(toc.value);
  flat.sort((a, b) => a.offset - b.offset);
  const out: Segment[] = [];
  let prev = 0;
  let prevTitle = '';
  for (const { offset, title } of flat) {
    if (offset > prev) {
      out.push({ offset: prev, title: prevTitle, text: text.slice(prev, offset) });
    }
    prev = offset;
    prevTitle = title;
  }
  out.push({ offset: prev, title: prevTitle, text: text.slice(prev) });
  return out.filter((s) => s.text.trim().length > 0 || s.title);
});

function scrollTextSegment(offset: number): void {
  document.getElementById(`seg-${offset}`)?.scrollIntoView({ behavior: 'smooth' });
}

function scrollHtmlAnchor(anchor: string): void {
  // toc 锚点 anchor:href:<zip 路径> → section[data-href] 对位
  const href = anchor.replace(/^anchor:href:/, '');
  const doc = iframe.value?.contentDocument;
  if (!doc) {
    return;
  }
  const target = doc.querySelector(`section[data-href="${CSS.escape(href)}"]`) ?? doc.getElementById(href);
  target?.scrollIntoView({ behavior: 'smooth' });
}

function jump(entry: TocEntry): void {
  if (mode.value === 'text') {
    const m = entry.anchor.match(/^anchor:char-(\d+)$/);
    if (m) {
      scrollTextSegment(Number(m[1]));
    }
  } else if (mode.value === 'html') {
    scrollHtmlAnchor(entry.anchor);
  }
}

// 进度随手保存
const progress = ref(item.value?.progress ?? 0);
watch(
  () => item.value?.id,
  () => {
    progress.value = item.value?.progress ?? 0;
  },
);
watch(progress, (v) => {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    if (conn.writable && item.value) {
      void api('/item/update', { method: 'POST', body: { id: item.value.id, progress: v } }).catch(() => {});
    }
  }, 600);
});
onUnmounted(() => clearTimeout(saveTimer));

async function finish(): Promise<void> {
  if (!conn.writable || !item.value) {
    return;
  }
  try {
    await api('/item/update', { method: 'POST', body: { id: item.value.id, read_status: 'finished', progress: 100 } });
    progress.value = 100;
  } catch (e) {
    ui.toastError(e);
  }
}

function openExternal(): void {
  if (item.value) {
    void api('/item/open', { method: 'POST', body: { id: item.value.id } }).catch((e) => ui.toastError(e));
  }
}

const showToc = computed(() => toc.value.length > 0);
</script>

<template>
  <div v-if="item" class="reader">
    <div class="reader-top">
      <button class="btn" @click="ui.readerItemId = null">← 返回</button>
      <div class="reader-title">{{ item.title || item.name }}</div>
      <div class="reader-progress">
        <input v-model.number="progress" type="range" min="0" max="100" :disabled="!conn.writable" />
        <span>{{ progress }}%</span>
      </div>
      <button v-if="conn.writable && item.read_status !== 'finished'" class="btn" @click="finish">读完</button>
      <button v-if="mode === 'unsupported'" class="btn" @click="openExternal">系统打开</button>
    </div>

    <div class="reader-body">
      <aside v-if="showToc" class="reader-toc">
        <template v-for="entry in toc" :key="entry.anchor">
          <button class="toc-item" @click="jump(entry)">{{ entry.title || '（无题）' }}</button>
          <button v-for="child in entry.children" :key="child.anchor" class="toc-item sub" @click="jump(child)">{{ child.title }}</button>
        </template>
      </aside>

      <div class="reader-content">
        <div v-if="loadError" class="reader-error">{{ loadError }}</div>
        <template v-else-if="mode === 'text'">
          <div class="text-body">
            <section v-for="seg in segments" :id="`seg-${seg.offset}`" :key="seg.offset">
              <h3 v-if="seg.title">{{ seg.title }}</h3>
              <pre class="seg-text">{{ seg.text }}</pre>
            </section>
          </div>
        </template>
        <iframe v-else-if="mode === 'html' && htmlContent !== null" ref="iframe" class="html-body" :srcdoc="htmlContent" sandbox="allow-same-origin" />
        <div v-else-if="mode === 'unsupported'" class="reader-error">
          {{ item.ext.toUpperCase() }} 格式在 2.0 内置阅读器支持；当前可「系统打开」用默认应用阅读。
        </div>
        <div v-else class="reader-error">正文加载中…</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.reader {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.reader-top {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--panel);
  flex: none;
}
.reader-title {
  font-size: 14px;
  font-weight: 600;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.reader-progress {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--muted);
}
.reader-body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.reader-toc {
  width: 240px;
  flex: none;
  overflow-y: auto;
  border-right: 1px solid var(--border);
  background: var(--panel);
  padding: 8px;
}
.toc-item {
  display: block;
  width: 100%;
  text-align: left;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  padding: 5px 8px;
  border-radius: 6px;
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.toc-item:hover {
  background: var(--hover);
}
.toc-item.sub {
  padding-left: 22px;
  font-size: 12px;
  color: var(--muted);
}
.reader-content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
}
.reader-error {
  padding: 60px 40px;
  text-align: center;
  color: var(--muted);
}
.text-body {
  max-width: 760px;
  margin: 0 auto;
  padding: 32px 24px 80px;
}
.text-body section {
  scroll-margin-top: 20px;
}
.text-body h3 {
  font-size: 17px;
  margin: 28px 0 10px;
  color: var(--accent);
}
.seg-text {
  font: 16px/1.9 -apple-system, 'PingFang SC', 'Microsoft YaHei', serif;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
}
.html-body {
  width: 100%;
  height: 100%;
  border: none;
  background: var(--bg);
}
</style>
