<script setup lang="ts">
// 详情侧板：元数据展示/编辑（update）、路径操作、封面管理、移入回收站。
// viewer 只读时仅展示。
import { computed, reactive, ref, watch } from 'vue';
import { useUi } from '@/stores/ui';
import { useBooks } from '@/stores/books';
import { useConnection } from '@/stores/connection';
import { useLibrary } from '@/stores/library';
import { api, directUrl } from '@/shared/api/client';
import { formatBytes, formatTime, READ_STATUS_LABEL } from '@/shared/format';
import { shell } from '@/app/shell';
import type { Item } from '@/shared/api/types';

const ui = useUi();
const books = useBooks();
const conn = useConnection();
const library = useLibrary();

const item = computed<Item | null>(() => books.items.find((i) => i.id === ui.selectedId) ?? null);
const show = computed(() => item.value !== null);

// 编辑态：从 item 快照构建表单
const editing = ref(false);
const form = reactive({
  title: '',
  authors: '',
  publisher: '',
  pubdate: '',
  isbn: '',
  language: '',
  series: '',
  series_index: '',
  description: '',
  tags: '',
  categories: '',
  star: 0,
  read_status: 'unread',
  progress: 0,
  annotation: '',
  url: '',
});
const saving = ref(false);

watch(
  () => item.value?.id,
  () => {
    editing.value = false;
  },
);

function startEdit(): void {
  const it = item.value;
  if (!it) {
    return;
  }
  form.title = it.title;
  form.authors = it.authors.join(', ');
  form.publisher = it.publisher;
  form.pubdate = it.pubdate;
  form.isbn = it.isbn;
  form.language = it.language;
  form.series = it.series;
  form.series_index = it.series_index ? String(it.series_index) : '';
  form.description = it.description;
  form.tags = it.tags.join(', ');
  form.categories = it.categories.join(', ');
  form.star = it.star;
  form.read_status = it.read_status;
  form.progress = it.progress;
  form.annotation = it.annotation;
  form.url = it.url;
  editing.value = true;
}

async function save(): Promise<void> {
  const it = item.value;
  if (!it) {
    return;
  }
  saving.value = true;
  try {
    const split = (s: string) => s.split(/[,，]/).map((x) => x.trim()).filter(Boolean);
    await api('/item/update', {
      method: 'POST',
      body: {
        id: it.id,
        title: form.title,
        authors: split(form.authors),
        publisher: form.publisher,
        pubdate: form.pubdate,
        isbn: form.isbn,
        language: form.language,
        series: form.series,
        series_index: form.series_index ? Number(form.series_index) : 0,
        description: form.description,
        tags: split(form.tags),
        categories: split(form.categories),
        star: form.star,
        read_status: form.read_status,
        progress: form.progress,
        annotation: form.annotation,
        url: form.url,
      },
    });
    editing.value = false;
    // SSE item.updated 会补丁列表；此处主动刷新维度（标签/分类集合可能变化）
    library.refreshDimensions().catch(() => {});
  } catch (e) {
    ui.toastError(e);
  } finally {
    saving.value = false;
  }
}

async function trash(): Promise<void> {
  const it = item.value;
  if (!it) {
    return;
  }
  try {
    await api('/item/delete', { method: 'POST', body: { id: it.id } });
    ui.selectedId = null;
    // SSE item.trashed 会摘除条目；主动兜底
    books.dropItem(it.id);
    library.refreshAll().catch(() => {});
  } catch (e) {
    ui.toastError(e);
  }
}

async function openFile(): Promise<void> {
  const it = item.value;
  if (!it) {
    return;
  }
  try {
    await api('/item/open', { method: 'POST', body: { id: it.id } });
  } catch (e) {
    ui.toastError(e);
  }
}

function showInFolder(path: string): void {
  void shell()?.showInFolder(path);
}

function copyPath(path: string): void {
  void shell()?.copyPath(path).then(() => ui.toast('路径已复制'));
}

// 封面替换
const coverInput = ref<HTMLInputElement | null>(null);
async function onCoverPick(e: Event): Promise<void> {
  const it = item.value;
  const file = (e.target as HTMLInputElement).files?.[0];
  (e.target as HTMLInputElement).value = '';
  if (!it || !file) {
    return;
  }
  const b64 = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
  try {
    await api('/item/cover', { method: 'PUT', body: { id: it.id, img_base64: b64 } });
  } catch (err) {
    ui.toastError(err);
  }
}

async function removeCover(): Promise<void> {
  const it = item.value;
  if (!it) {
    return;
  }
  try {
    await api(`/item/cover?id=${encodeURIComponent(it.id)}`, { method: 'DELETE' });
  } catch (e) {
    ui.toastError(e);
  }
}
</script>

<template>
  <aside v-if="show && item" class="inspector">
    <div class="inspector-scroll">
      <div class="inspector-cover">
        <img :src="directUrl('/item/cover', { id: item.id })" :alt="item.title" />
        <div v-if="conn.writable" class="cover-actions">
          <input ref="coverInput" type="file" hidden accept="image/*" @change="onCoverPick" />
          <button class="btn small" @click="coverInput?.click()">换封面</button>
          <button v-if="item.custom_cover" class="btn small" @click="removeCover">还原封面</button>
        </div>
      </div>

      <template v-if="!editing">
        <h2 class="i-title">{{ item.title || item.name }}</h2>
        <div class="i-authors">{{ item.authors.join(', ') || '佚名' }}</div>

        <div class="i-meta">
          <div class="meta-row"><span>评分</span><b>{{ item.star > 0 ? '★'.repeat(item.star) : '—' }}</b></div>
          <div class="meta-row"><span>状态</span><b>{{ READ_STATUS_LABEL[item.read_status] ?? item.read_status }}</b></div>
          <div class="meta-row"><span>进度</span><b>{{ item.progress }}%</b></div>
          <div class="meta-row"><span>格式</span><b>{{ item.ext.toUpperCase() }} · {{ formatBytes(item.size) }}</b></div>
          <div class="meta-row"><span>入库</span><b>{{ formatTime(item.added_time) }}</b></div>
          <div v-if="item.publisher" class="meta-row"><span>出版社</span><b>{{ item.publisher }}</b></div>
          <div v-if="item.pubdate" class="meta-row"><span>出版</span><b>{{ item.pubdate }}</b></div>
          <div v-if="item.isbn" class="meta-row"><span>ISBN</span><b>{{ item.isbn }}</b></div>
          <div v-if="item.series" class="meta-row"><span>系列</span><b>{{ item.series }} #{{ item.series_index }}</b></div>
          <div v-if="item.language" class="meta-row"><span>语言</span><b>{{ item.language }}</b></div>
        </div>

        <div v-if="item.tags.length || item.categories.length" class="i-chips">
          <span v-for="c in item.categories" :key="'c' + c" class="chip cat">{{ c }}</span>
          <span v-for="t in item.tags" :key="'t' + t" class="chip">{{ t }}</span>
        </div>

        <p v-if="item.description" class="i-desc">{{ item.description }}</p>
        <p v-if="item.annotation" class="i-annotation">📝 {{ item.annotation }}</p>
      </template>

      <template v-else>
        <div class="edit-form">
          <div class="form-row"><span class="form-label">书名</span><input v-model="form.title" type="text" /></div>
          <div class="form-row"><span class="form-label">作者</span><input v-model="form.authors" type="text" placeholder="逗号分隔" /></div>
          <div class="form-row"><span class="form-label">出版社</span><input v-model="form.publisher" type="text" /></div>
          <div class="form-row"><span class="form-label">出版日期</span><input v-model="form.pubdate" type="text" /></div>
          <div class="form-row"><span class="form-label">ISBN</span><input v-model="form.isbn" type="text" /></div>
          <div class="form-row"><span class="form-label">系列</span><input v-model="form.series" type="text" class="series-input" /><input v-model="form.series_index" type="number" step="0.1" class="series-idx" /></div>
          <div class="form-row"><span class="form-label">语言</span><input v-model="form.language" type="text" /></div>
          <div class="form-row"><span class="form-label">分类</span><input v-model="form.categories" type="text" placeholder="逗号分隔" /></div>
          <div class="form-row"><span class="form-label">标签</span><input v-model="form.tags" type="text" placeholder="逗号分隔" /></div>
          <div class="form-row"><span class="form-label">评分</span>
            <select v-model.number="form.star"><option :value="0">未评</option><option v-for="n in 5" :key="n" :value="n">{{ '★'.repeat(n) }}</option></select>
          </div>
          <div class="form-row"><span class="form-label">状态</span>
            <select v-model="form.read_status"><option v-for="(label, value) in READ_STATUS_LABEL" :key="value" :value="value">{{ label }}</option></select>
          </div>
          <div class="form-row"><span class="form-label">进度 %</span><input v-model.number="form.progress" type="number" min="0" max="100" /></div>
          <div class="form-row"><span class="form-label">备注</span><textarea v-model="form.annotation" rows="2" /></div>
          <div class="form-row"><span class="form-label">链接</span><input v-model="form.url" type="text" /></div>
          <div class="form-row"><span class="form-label">简介</span><textarea v-model="form.description" rows="4" /></div>
        </div>
      </template>

      <div class="i-paths">
        <div class="paths-title">文件位置</div>
        <div v-for="p in item.paths" :key="p" class="path-row">
          <span class="path-text" :title="p">{{ p }}</span>
          <button class="path-btn" title="在文件管理器中显示" @click="showInFolder(p)">📁</button>
          <button class="path-btn" title="复制路径" @click="copyPath(p)">⧉</button>
        </div>
      </div>
    </div>

    <div class="inspector-actions">
      <template v-if="conn.writable">
        <button v-if="!editing" class="btn primary" :disabled="saving" @click="startEdit">编辑</button>
        <template v-else>
          <button class="btn" :disabled="saving" @click="editing = false">取消</button>
          <button class="btn primary" :disabled="saving" @click="save">保存</button>
        </template>
        <button class="btn" @click="ui.readerItemId = item.id; ui.view = 'library'">阅读</button>
        <button v-if="conn.isAdmin" class="btn" @click="openFile">系统打开</button>
        <button class="btn danger" @click="trash">移入回收站</button>
      </template>
      <template v-else>
        <button class="btn" @click="ui.readerItemId = item.id; ui.view = 'library'">阅读</button>
      </template>
      <button class="btn ghost" @click="ui.selectedId = null">关闭</button>
    </div>
  </aside>
</template>

<style scoped>
.inspector {
  width: 320px;
  flex: none;
  background: var(--panel);
  border-left: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.inspector-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
}
.inspector-cover {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}
.inspector-cover img {
  max-width: 170px;
  max-height: 240px;
  border-radius: 6px;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.35);
}
.cover-actions {
  display: flex;
  gap: 8px;
}
.i-title {
  margin: 0 0 4px;
  font-size: 16px;
  line-height: 1.4;
}
.i-authors {
  font-size: 13px;
  color: var(--muted);
  margin-bottom: 12px;
}
.i-meta {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-bottom: 12px;
}
.meta-row {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  font-size: 12px;
}
.meta-row span {
  color: var(--muted);
  flex: none;
}
.meta-row b {
  font-weight: 400;
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.i-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-bottom: 12px;
}
.chip {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--panel-2);
  border: 1px solid var(--border);
}
.chip.cat {
  border-color: var(--accent-dim);
}
.i-desc {
  font-size: 12px;
  line-height: 1.7;
  color: var(--muted);
  margin: 0 0 8px;
  white-space: pre-wrap;
}
.i-annotation {
  font-size: 12px;
  line-height: 1.7;
  margin: 0;
  white-space: pre-wrap;
}
.edit-form .form-row {
  margin-bottom: 8px;
}
.series-input {
  flex: 1;
}
.series-idx {
  width: 64px;
  flex: none;
}
.i-paths {
  margin-top: 14px;
}
.paths-title {
  font-size: 11px;
  color: var(--muted);
  margin-bottom: 4px;
}
.path-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 0;
}
.path-text {
  flex: 1;
  font-size: 11px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  direction: rtl;
  text-align: left;
}
.path-btn {
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 13px;
  padding: 2px 4px;
  border-radius: 4px;
}
.path-btn:hover {
  background: var(--hover);
}
.inspector-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding: 12px 16px;
  border-top: 1px solid var(--border);
}
.btn.ghost {
  border-color: transparent;
  background: transparent;
  color: var(--muted);
}
</style>
