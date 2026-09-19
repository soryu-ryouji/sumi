<script setup lang="ts">
// 详情侧板：选中书籍时展示/编辑元数据（update）、路径操作、文件夹移动、封面管理、移入回收站（viewer 只读时仅展示）；
// 多选时顶部批量条提供批量移动；未选中书籍时显示当前分区概览（文件夹分区 → 文件夹信息，其余 → 书库整体信息）。
import { computed, reactive, ref, watch } from 'vue';
import { useUi } from '@/stores/ui';
import { useBooks } from '@/stores/books';
import { useConnection } from '@/stores/connection';
import { useLibrary } from '@/stores/library';
import { api, directUrl } from '@/shared/api/client';
import { primaryPathOf } from '@/shared/lib/item';
import { formatBytes, formatTime, READ_STATUS_LABEL } from '@/shared/format';
import { shell } from '@/app/shell';
import { moveItemsToFolder } from '@/shared/lib/move';
import FolderPickerDialog from './FolderPickerDialog.vue';
import type { FolderNode, Item } from '@/shared/api/types';

const ui = useUi();
const books = useBooks();
const conn = useConnection();
const library = useLibrary();

const item = computed<Item | null>(() => books.items.find((i) => i.id === ui.selectedId) ?? null);
const show = computed(() => item.value !== null);

// —— 移动入口（单选「文件夹」行 / 多选批量条；回收站视图不提供移动） ——
const moveAllowed = computed(() => conn.writable && !books.filter.inTrash);
const multiSelected = computed(() => moveAllowed.value && show.value && ui.selectedIds.length > 1);
/** 主选中的主路径所在目录（'' = 书库根） */
const primaryFolder = computed(() => {
  const p = item.value ? primaryPathOf(item.value) : '';
  const i = p.lastIndexOf('/');
  return i < 0 ? '' : p.slice(0, i);
});
const folderLabel = computed(() => primaryFolder.value || '（书库根目录）');
const showFolderDialog = ref(false);

/** 对话框确定：多选批量移、单选只移主选中 */
function onMoveConfirm(path: string): void {
  showFolderDialog.value = false;
  if (ui.selectedIds.length > 1) {
    void moveItemsToFolder([...ui.selectedIds], path);
  } else if (item.value) {
    void moveItemsToFolder([item.value.id], path);
  }
}

// 分区概览：在文件夹树中递归定位当前筛选的文件夹（找不到时名称回退为路径最后一段）
function findFolderNode(nodes: FolderNode[], path: string): FolderNode | null {
  for (const n of nodes) {
    if (n.path === path) {
      return n;
    }
    const hit = findFolderNode(n.children, path);
    if (hit) {
      return hit;
    }
  }
  return null;
}

const folderNode = computed<FolderNode | null>(() => {
  const p = books.filter.folder;
  return p ? findFolderNode(library.tree, p) : null;
});

const folderName = computed(() => {
  if (folderNode.value) {
    return folderNode.value.name;
  }
  return books.filter.folder?.split(/[\\/]/).filter(Boolean).pop() ?? '';
});

// 概览的「当前筛选」chips（系列 + 分类/标签/作者；key 加维度前缀避免同名值 key 重复）
const activeFilters = computed<{ key: string; label: string }[]>(() => {
  const f = books.filter;
  return [
    ...(f.series ? [{ key: `s:${f.series}`, label: f.series }] : []),
    ...f.categories.map((v) => ({ key: `c:${v}`, label: v })),
    ...f.tags.map((v) => ({ key: `t:${v}`, label: v })),
    ...f.authors.map((v) => ({ key: `a:${v}`, label: v })),
  ];
});

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

// 上下文菜单「编辑元数据」：选中项已定位，直接进编辑态
watch(
  () => ui.inspectorEdit,
  (v) => {
    if (v) {
      startEdit();
      ui.inspectorEdit = false;
    }
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
  <aside v-if="ui.showInspector" class="inspector">
    <!-- 选中书籍：元数据展示/编辑 -->
    <div v-if="show && item" class="inspector-scroll">
      <!-- 多选批量条：不微批编辑，仅提供批量移动 -->
      <div v-if="multiSelected" class="batch-bar">
        <span class="batch-count">已选 {{ ui.selectedIds.length }} 本</span>
        <button class="btn small" @click="showFolderDialog = true">移动到文件夹…</button>
      </div>
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
          <div
            class="meta-row folder-row"
            :class="{ clickable: moveAllowed }"
            :title="moveAllowed ? '点击移动到其他文件夹' : ''"
            @click="moveAllowed && (showFolderDialog = true)"
          >
            <span>文件夹</span><b>{{ folderLabel }}</b>
          </div>
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
          <div class="edit-actions">
            <button class="btn" :disabled="saving" @click="editing = false">取消</button>
            <button class="btn primary" :disabled="saving" @click="save">保存</button>
          </div>
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

    <!-- 未选中书籍：当前分区概览 -->
    <div v-else class="inspector-scroll">
      <template v-if="books.filter.folder">
        <div class="paths-title">文件夹</div>
        <h2 class="i-title">{{ folderName }}</h2>
        <div class="i-meta">
          <div class="meta-row"><span>路径</span><b :title="books.filter.folder">{{ books.filter.folder }}</b></div>
          <div class="meta-row"><span>书籍</span><b>{{ books.total }} 本 · {{ formatBytes(books.totalSize) }}</b></div>
          <div v-if="folderNode" class="meta-row"><span>修改</span><b>{{ formatTime(folderNode.modification_time) }}</b></div>
          <div v-if="folderNode" class="meta-row"><span>子文件夹</span><b>{{ folderNode.children.length }}</b></div>
        </div>
      </template>
      <template v-else>
        <div class="paths-title">书库</div>
        <h2 class="i-title">{{ library.libraryName || '书库' }}</h2>
        <div class="i-meta">
          <div class="meta-row"><span>书籍</span><b>{{ books.total }} 本 · {{ formatBytes(books.totalSize) }}</b></div>
          <div class="meta-row"><span>分类</span><b>{{ library.categories.length }}</b></div>
          <div class="meta-row"><span>标签</span><b>{{ library.tags.length }}</b></div>
          <div class="meta-row"><span>作者</span><b>{{ library.authors.length }}</b></div>
          <div class="meta-row"><span>系列</span><b>{{ library.series.length }}</b></div>
        </div>
      </template>
      <template v-if="activeFilters.length">
        <div class="paths-title">当前筛选</div>
        <div class="i-chips">
          <span v-for="f in activeFilters" :key="f.key" class="chip">{{ f.label }}</span>
        </div>
      </template>
    </div>
    <FolderPickerDialog
      v-if="showFolderDialog"
      :title="ui.selectedIds.length > 1 ? `移动 ${ui.selectedIds.length} 本书到文件夹` : '移动到文件夹'"
      :current="ui.selectedIds.length > 1 ? null : primaryFolder"
      @confirm="onMoveConfirm"
      @cancel="showFolderDialog = false"
    />
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
/* 多选批量条（面板顶部） */
.batch-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  margin-bottom: 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--panel-2);
}
.batch-count {
  flex: 1;
  font-size: 12px;
  color: var(--muted);
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
/* 「文件夹」行：可写时点击弹移动对话框 */
.folder-row.clickable {
  cursor: pointer;
  border-radius: 4px;
}
.folder-row.clickable:hover b {
  color: var(--accent);
  text-decoration: underline;
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
</style>
