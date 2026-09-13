<script setup lang="ts">
// 设置面板：书库（改名/存储模式/扫描）、LAN、缓存目录、锁管理、关于。
// 只读 viewer 仅可浏览（写操作按钮隐藏）。
import { computed, onMounted, reactive, ref, type Ref } from 'vue';
import { useUi } from '@/stores/ui';
import { useLibrary } from '@/stores/library';
import { useConnection } from '@/stores/connection';
import { api } from '@/shared/api/client';
import { shell } from '@/app/shell';
import DragBar from '@/app/chrome/DragBar.vue';
import type { DimensionNames, LanConfig } from '@/shared/api/types';

const ui = useUi();
const library = useLibrary();
const conn = useConnection();

const TABS = [
  { key: 'library', label: '书库' },
  { key: 'scan', label: '扫描' },
  { key: 'lan', label: '局域网' },
  { key: 'cache', label: '缓存' },
  { key: 'locks', label: '锁' },
  { key: 'about', label: '关于' },
] as const;

// ---- 书库 ----
const libName = ref('');
const storageMode = ref('database');
const switching = ref(false);
onMounted(async () => {
  const info = await api<{ name: string; storage_mode: string }>('/library/info');
  libName.value = info.name;
  storageMode.value = info.storage_mode;
});

async function saveName(): Promise<void> {
  try {
    await api('/library/info', { method: 'PATCH', body: { name: libName.value.trim() } });
    await library.refreshMeta();
    ui.toast('已保存');
  } catch (e) {
    ui.toastError(e);
  }
}

// ---- 打开方式 ----
const OPEN_EXTS = ['epub', 'pdf', 'txt', 'md', 'mobi', 'azw3', 'docx', 'cbz'] as const;
const openers = ref<Record<string, string>>({});
const openersSaving = ref(false);
onMounted(async () => {
  const info = await api<{ openers: Record<string, string> }>('/library/info');
  openers.value = { ...info.openers };
});

async function pickOpener(ext: string): Promise<void> {
  const picked = await shell()?.pickApp();
  if (picked) {
    openers.value = { ...openers.value, [ext]: picked };
  }
}

async function saveOpeners(): Promise<void> {
  openersSaving.value = true;
  try {
    // 清掉空值后整体替换（config.toml 的 [openers]）
    const clean = Object.fromEntries(Object.entries(openers.value).filter(([, v]) => v.trim() !== ''));
    await api('/library/info', { method: 'PATCH', body: { openers: clean } });
    openers.value = clean;
    ui.toast('已保存（立即生效）');
  } catch (e) {
    ui.toastError(e);
  } finally {
    openersSaving.value = false;
  }
}

async function switchStorage(mode: string): Promise<void> {
  if (mode === storageMode.value) {
    return;
  }
  if (!window.confirm(`切换元数据存储方案到「${mode === 'database' ? '数据库' : '配置文件'}」？切换会全量迁移数据。`)) {
    return;
  }
  switching.value = true;
  try {
    await api('/library/storage_mode', { method: 'POST', body: { mode } });
    storageMode.value = mode;
    ui.toast('存储方案已切换');
  } catch (e) {
    ui.toastError(e);
  } finally {
    switching.value = false;
  }
}

// ---- 扫描 ----
const scan = reactive({ periodic: true, interval: 900 });
onMounted(async () => {
  const info = await api<{ scan: { periodic: boolean; interval: number } }>('/library/info');
  scan.periodic = info.scan.periodic;
  scan.interval = info.scan.interval;
});
async function saveScan(): Promise<void> {
  try {
    await api('/library/scan', { method: 'PUT', body: { periodic: scan.periodic, interval: scan.interval } });
    ui.toast('已保存');
  } catch (e) {
    ui.toastError(e);
  }
}

// ---- LAN ----
const lan: Ref<LanConfig | null> = ref(null);
const lanUrls = computed(() => (lan.value ? lanAddresses.value.map((a) => `http://${a}:${lan.value!.port}`).join('　') : ''));
const lanAddresses = ref<string[]>([]);
onMounted(async () => {
  if (conn.isAdmin) {
    lan.value = await api<LanConfig>('/app/lan');
    lanAddresses.value = (await shell()?.lanAddresses()) ?? [];
  }
});
async function saveLan(): Promise<void> {
  if (!lan.value) {
    return;
  }
  try {
    const { active: _active, ...body } = lan.value;
    lan.value = await api<LanConfig>('/app/lan', { method: 'PUT', body });
    ui.toast('已保存（监听能力随桌面端打包接线）');
  } catch (e) {
    ui.toastError(e);
  }
}

// ---- 缓存 ----
const cache = ref<{ current: string; isDefault: boolean } | null>(null);
onMounted(async () => {
  cache.value = (await shell()?.getCacheDir()) ?? null;
});
async function pickCache(): Promise<void> {
  const picked = await shell()?.pickCacheDir();
  if (!picked) {
    return;
  }
  await applyCache(picked);
}
async function resetCache(): Promise<void> {
  await applyCache(null);
}
async function applyCache(p: string | null): Promise<void> {
  try {
    await shell()?.setCacheDir(p); // server 重启，就绪经 onServerStarted 自动接管
  } catch (e) {
    ui.toastError(e);
  }
}

// ---- 锁 ----
const locks = ref<DimensionNames | null>(null);
onMounted(async () => {
  locks.value = await api<DimensionNames>('/lock/list');
});
const lockForm = reactive({ dimension: 'tag', name: '', password: '' });
const unlockForm = reactive({ dimension: 'tag', name: '', password: '' });

async function setLock(): Promise<void> {
  try {
    await api('/lock/set', { method: 'POST', body: { ...lockForm } });
    locks.value = await api<DimensionNames>('/lock/list');
    lockForm.name = '';
    lockForm.password = '';
    ui.toast('已上锁');
  } catch (e) {
    ui.toastError(e);
  }
}
async function unlock(): Promise<void> {
  try {
    const res = await api<{ unlock_token: string }>('/lock/unlock', { method: 'POST', body: { ...unlockForm } });
    conn.addUnlockTicket(res.unlock_token);
    unlockForm.name = '';
    unlockForm.password = '';
    ui.toast('已解锁（本次会话有效）');
  } catch (e) {
    ui.toastError(e);
  }
}
async function removeLock(dim: string, name: string): Promise<void> {
  const password = window.prompt(`解除「${name}」的锁，请输入密码：`);
  if (password === null) {
    return;
  }
  try {
    await api('/lock/remove', { method: 'POST', body: { dimension: dim, name, password } });
    locks.value = await api<DimensionNames>('/lock/list');
  } catch (e) {
    ui.toastError(e);
  }
}
const DIM_LABEL: Record<string, string> = { folders: '文件夹', categories: '分类', tags: '标签', authors: '作者' };
const lockEntries = computed(() => {
  const l = locks.value;
  if (!l) {
    return [];
  }
  return (['folders', 'categories', 'tags', 'authors'] as const).flatMap((key) => l[key].map((name) => ({ dim: key, name })));
});
</script>

<template>
  <div class="settings">
    <DragBar title="设置" />
    <div class="settings-body">
      <nav class="settings-nav">
        <button v-for="tab in TABS" :key="tab.key" class="nav-item" :class="{ active: ui.settingsTab === tab.key }" @click="ui.settingsTab = tab.key; ui.view = 'settings'">
          {{ tab.label }}
        </button>
        <button class="nav-item back" @click="ui.view = 'library'">← 返回书库</button>
      </nav>

      <div class="settings-panel">
        <!-- 书库 -->
        <template v-if="ui.settingsTab === 'library'">
          <h3>书库信息</h3>
          <div class="form-row"><span class="form-label">书库名</span><input v-model="libName" type="text" :disabled="!conn.writable" /><button v-if="conn.writable" class="btn small" @click="saveName">保存</button></div>
          <div v-if="conn.isAdmin" class="form-row">
            <span class="form-label">元数据存储</span>
            <select :value="storageMode" :disabled="switching" @change="switchStorage(($event.target as HTMLSelectElement).value)">
              <option value="database">数据库（本地专用，性能好）</option>
              <option value="toml">配置文件（网盘同步友好）</option>
            </select>
          </div>
        </template>

        <!-- 打开方式 -->
        <template v-else-if="ui.settingsTab === 'openers'">
          <h3>打开方式</h3>
          <p class="hint">按格式指定打开书籍的应用（未指定 = 系统默认应用）。
          macOS 支持 .app 路径 / 应用名 / 可执行文件路径；Windows/Linux 填可执行文件路径。</p>
          <div v-for="ext in OPEN_EXTS" :key="ext" class="form-row">
            <span class="form-label">.{{ ext }}</span>
            <input v-model="openers[ext]" type="text" placeholder="系统默认" :disabled="!conn.writable" class="opener-input" />
            <button v-if="shell() && conn.writable" class="btn small" @click="pickOpener(ext)">选择…</button>
            <button v-if="conn.writable && openers[ext]" class="btn small" @click="openers[ext] = ''">清除</button>
          </div>
          <button v-if="conn.writable" class="btn" :disabled="openersSaving" @click="saveOpeners">保存</button>
        </template>

        <!-- 扫描 -->
        <template v-else-if="ui.settingsTab === 'scan'">
          <h3>周期扫描</h3>
          <p class="hint">文件监听漏事件时的兜底：周期性重扫书库收敛索引（默认 15 分钟）。</p>
          <div class="form-row"><span class="form-label">启用</span><input v-model="scan.periodic" type="checkbox" :disabled="!conn.writable" /></div>
          <div class="form-row"><span class="form-label">间隔（秒）</span><input v-model.number="scan.interval" type="number" min="60" :disabled="!conn.writable" /></div>
          <button v-if="conn.writable" class="btn" @click="saveScan">保存</button>
        </template>

        <!-- LAN -->
        <template v-else-if="ui.settingsTab === 'lan'">
          <h3>局域网访问</h3>
          <p v-if="!conn.isAdmin" class="hint">需要管理员权限。</p>
          <template v-if="lan && conn.isAdmin">
            <p class="hint">同一书库经浏览器访问（viewer token 鉴权，权限三档）。监听能力随桌面端打包接线。</p>
            <div class="form-row"><span class="form-label">启用</span><input v-model="lan.enabled" type="checkbox" /></div>
            <div class="form-row"><span class="form-label">端口</span><input v-model.number="lan.port" type="number" min="1" max="65535" /></div>
            <div class="form-row"><span class="form-label">viewer token</span><input v-model="lan.token" type="text" /></div>
            <div class="form-row"><span class="form-label">允许写</span><input v-model="lan.writable" type="checkbox" /></div>
            <div class="form-row"><span class="form-label">拆分写 token</span><input v-model="lan.separate_write_token" type="checkbox" /></div>
            <div v-if="lan.separate_write_token" class="form-row"><span class="form-label">write token</span><input v-model="lan.write_token" type="text" /></div>
            <button class="btn" @click="saveLan">保存</button>
            <div v-if="lanUrls" class="lan-addr">本机地址：{{ lanUrls }}</div>
          </template>
        </template>

        <!-- 缓存 -->
        <template v-else-if="ui.settingsTab === 'cache'">
          <h3>缓存目录</h3>
          <p class="hint">派生缓存（自动封面 / 全文索引 / 归一化正文）存放位置，可随时删除、重启自动重建，不参与同步。切换后服务重启。</p>
          <div class="form-row"><span class="form-label">当前位置</span><code class="cache-path">{{ cache?.current ?? '—' }}</code></div>
          <button class="btn" @click="pickCache">选择新目录…</button>
          <button v-if="cache && !cache.isDefault" class="btn" @click="resetCache">恢复默认</button>
        </template>

        <!-- 锁 -->
        <template v-else-if="ui.settingsTab === 'locks'">
          <h3>锁</h3>
          <p class="hint">四维锁（文件夹/分类/标签/作者），服务端强制：未解锁时对应条目从列表与内容直连中排除。</p>
          <div v-if="lockEntries.length" class="lock-list">
            <div v-for="entry in lockEntries" :key="entry.dim + entry.name" class="lock-row">
              <span class="chip">{{ DIM_LABEL[entry.dim] }}</span>
              <span class="lock-name">{{ entry.name }}</span>
              <span class="lock-unlocked" v-if="false" />
              <button v-if="conn.isAdmin" class="btn small" @click="removeLock(entry.dim === 'folders' ? 'folder' : entry.dim.slice(0, -1), entry.name)">解除</button>
            </div>
          </div>
          <div v-else class="hint">当前没有上锁的条目。</div>

          <template v-if="conn.isAdmin">
            <h4>上锁</h4>
            <div class="form-row">
              <select v-model="lockForm.dimension"><option value="folder">文件夹</option><option value="category">分类</option><option value="tag">标签</option><option value="author">作者</option></select>
              <input v-model="lockForm.name" type="text" placeholder="名称" />
              <input v-model="lockForm.password" type="password" placeholder="密码" />
              <button class="btn small" @click="setLock">上锁</button>
            </div>
          </template>

          <h4>解锁（本次会话）</h4>
          <div class="form-row">
            <select v-model="unlockForm.dimension"><option value="folder">文件夹</option><option value="category">分类</option><option value="tag">标签</option><option value="author">作者</option></select>
            <input v-model="unlockForm.name" type="text" placeholder="名称" />
            <input v-model="unlockForm.password" type="password" placeholder="密码" />
            <button class="btn small" @click="unlock">解锁</button>
          </div>
          <p class="hint">已解锁 {{ conn.unlockTickets.length }} 项（票据随服务重启失效）。</p>
        </template>

        <!-- 关于 -->
        <template v-else>
          <div class="about">
            <img src="/icon.png" alt="sumi" />
            <h3>sumi</h3>
            <p>对标 Calibre 的开源书籍/文本资源管理工具</p>
            <p class="hint">daemon {{ conn.appInfo?.version }} · app 0.1.0 · {{ conn.appInfo?.platform }}</p>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.settings-body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.settings-nav {
  width: 160px;
  flex: none;
  border-right: 1px solid var(--border);
  background: var(--panel);
  padding: 10px 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.nav-item {
  text-align: left;
  padding: 7px 12px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
}
.nav-item:hover {
  background: var(--hover);
}
.nav-item.active {
  background: var(--accent-dim);
  color: #fff;
}
.nav-item.back {
  margin-top: 12px;
  color: var(--muted);
}
.settings-panel {
  flex: 1;
  overflow-y: auto;
  padding: 24px 28px;
  max-width: 720px;
}
h3 {
  margin: 0 0 16px;
  font-size: 15px;
}
h4 {
  margin: 20px 0 10px;
  font-size: 13px;
  color: var(--muted);
}
.hint {
  font-size: 12px;
  color: var(--muted);
  line-height: 1.7;
  margin: 0 0 14px;
}
.cache-path {
  font-size: 12px;
  color: var(--muted);
  word-break: break-all;
}
.lan-addr {
  margin-top: 14px;
  font-size: 12px;
  color: var(--ok);
}
.opener-input {
  flex: 1;
  min-width: 0;
}
.lock-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 8px;
}
.lock-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.lock-name {
  flex: 1;
  font-size: 13px;
}
.chip {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--panel-2);
  border: 1px solid var(--border);
}
.about {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding-top: 40px;
}
.about img {
  width: 88px;
  height: 88px;
  border-radius: 20px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}
.about h3 {
  margin: 8px 0 0;
}
.about p {
  margin: 0;
  font-size: 13px;
  color: var(--muted);
}
</style>
