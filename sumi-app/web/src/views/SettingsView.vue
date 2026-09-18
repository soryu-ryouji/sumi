<script setup lang="ts">
// 设置面板：浮动对话框（Teleport 到 body 遮罩居中，书库界面保留在背景），
// 书库（改名/存储模式/扫描）、LAN、缓存目录、锁管理、关于。只读 viewer 仅可浏览（写操作按钮隐藏）。
// 交互要点（参考 hawk SettingsDialog）：
// - 遮罩「按下与抬起都落在遮罩上」才关闭：面板内拖选文本滑出松开不误关。
// - Esc 关闭（捕获阶段拦截并阻断冒泡，避免顶栏搜索等全局 Esc 处理跟着触发）。
// - 打开期间挂 body.dialog-open 挂起窗口拖拽区（styles.css 全局降为 no-drag）：Electron 的
//   -webkit-app-region: drag 由 OS 命中测试优先消费，不禁用的话点遮罩盖住的顶栏会变成拖动窗口。
import { computed, onMounted, onUnmounted, reactive, ref, type Ref } from 'vue';
import { useEventListener } from '@vueuse/core';
import { useUi } from '@/stores/ui';
import { useLibrary } from '@/stores/library';
import { useConnection } from '@/stores/connection';
import { api } from '@/shared/api/client';
import { shell } from '@/app/shell';
import { hasShell } from '@/shared/lib/platform';
import { useUpdater } from '@/shared/lib/updater';
import { formatBytes } from '@/shared/format';
import type { DimensionNames, LanConfig } from '@/shared/api/types';

const ui = useUi();
const library = useLibrary();
const conn = useConnection();

const TABS = [
  { key: 'interface', label: '界面' },
  { key: 'library', label: '书库' },
  { key: 'openers', label: '打开方式' },
  { key: 'scan', label: '扫描' },
  { key: 'lan', label: '局域网' },
  { key: 'cache', label: '缓存' },
  { key: 'locks', label: '锁' },
  { key: 'update', label: '更新' },
  { key: 'about', label: '关于' },
] as const;

// ---- 界面：侧栏区块显隐（纯前端偏好，随 ui store 持久化，无服务端交互） ----
const SIDEBAR_SECTIONS = [
  { key: 'folders', label: '文件夹' },
  { key: 'category', label: '分类' },
  { key: 'tag', label: '标签' },
  { key: 'author', label: '作者' },
  { key: 'series', label: '系列' },
] as const;

// ---- 书库 ----
const libName = ref('');
const storageMode = ref('database');
const switching = ref(false);
/** 保存时回写 EPUB/PDF 文件本身（daemon 持久化；网盘同步场景建议关闭） */
const embedMetadata = ref(true);
onMounted(async () => {
  const info = await api<{ name: string; storage_mode: string; embed_metadata?: boolean }>('/library/info');
  libName.value = info.name;
  storageMode.value = info.storage_mode;
  // 向后兼容：旧 daemon 无此字段时保持默认开
  if (typeof info.embed_metadata === 'boolean') {
    embedMetadata.value = info.embed_metadata;
  }
});

/** 勾选即保存（无需显式保存按钮；失败回滚勾选态）。从 DOM 读新值——单向 :checked 绑定下 ref 不随点击变化 */
async function toggleEmbed(next: boolean): Promise<void> {
  const prev = embedMetadata.value;
  embedMetadata.value = next;
  try {
    await api('/library/info', { method: 'PATCH', body: { embed_metadata: next } });
    ui.toast('已保存');
  } catch (e) {
    embedMetadata.value = prev;
    ui.toastError(e);
  }
}

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
const refreshingCache = ref(false);
onMounted(async () => {
  cache.value = (await shell()?.getCacheDir()) ?? null;
});
/** 重建派生缓存（补缺失模式：封面缺失重生成、全文索引补齐；已存在的不动） */
async function refreshCache(): Promise<void> {
  refreshingCache.value = true;
  try {
    const res = await api<{ dispatched: number }>('/library/refresh_cache', { method: 'POST', body: { type: 'library' } });
    ui.toast(`已派发 ${res.dispatched} 本重建（后台进行）`);
  } catch (e) {
    ui.toastError(e);
  } finally {
    refreshingCache.value = false;
  }
}
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
// lock/* API 的 dimension 用单数（folder/category/tag/author），显式映射（slice(0,-1) 会把 categories 截成 categorie）
const DIM_SINGULAR: Record<string, string> = { folders: 'folder', categories: 'category', tags: 'tag', authors: 'author' };
const lockEntries = computed(() => {
  const l = locks.value;
  if (!l) {
    return [];
  }
  return (['folders', 'categories', 'tags', 'authors'] as const).flatMap((key) => l[key].map((name) => ({ dim: key, name })));
});

// ---- 更新（状态机见 shared/lib/updater.ts） ----
const updater = useUpdater();
const appVersion = ref('');
const buildSha = ref('');
onMounted(() => {
  void shell()?.getAppVersion().then((v) => {
    appVersion.value = v.version;
    buildSha.value = v.sha;
  });
});
/** 下载百分比（total 未知时为 null，UI 显示已下载字节数） */
const downloadPct = computed(() => {
  const p = updater.progress.value;
  return p && p.total > 0 ? Math.floor((p.received / p.total) * 100) : null;
});
/** 发现的新版本标签（nightly 显示短 sha，稳定版带 v 前缀） */
const updateLabel = computed(() => {
  const info = updater.update.value;
  if (!info) {
    return '';
  }
  return info.channel === 'nightly' ? `nightly ${info.version}` : `v${info.version}`;
});

// ---- 浮动对话框生命周期 ----
onMounted(() => {
  document.body.classList.add('dialog-open');
});
onUnmounted(() => {
  document.body.classList.remove('dialog-open');
});

// 遮罩关闭：按下与抬起都落在遮罩上才关（面板内拖选文本滑出松开不误关）
let downOnMask = false;

function onMaskDown(e: PointerEvent): void {
  downOnMask = e.target === e.currentTarget;
}

function onMaskUp(e: PointerEvent): void {
  if (downOnMask && e.target === e.currentTarget) {
    ui.settingsOpen = false;
  }
  downOnMask = false;
}

// Esc 关闭：捕获阶段处理并阻断冒泡，避免顶栏搜索等全局 Esc 处理跟着触发
useEventListener(
  window,
  'keydown',
  (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      ui.settingsOpen = false;
    }
  },
  { capture: true },
);
</script>

<template>
  <Teleport to="body">
    <div class="mask" @pointerdown="onMaskDown" @pointerup="onMaskUp">
      <div class="dialog" role="dialog" aria-modal="true" aria-label="设置">
        <header class="dialog-head">
          <span class="dialog-title">设置</span>
          <button class="dialog-close" title="关闭 (Esc)" @click="ui.settingsOpen = false">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </header>

        <div class="settings-body">
          <nav class="settings-nav">
            <button v-for="tab in TABS" :key="tab.key" class="nav-item" :class="{ active: ui.settingsTab === tab.key }" @click="ui.settingsTab = tab.key">
              {{ tab.label }}
            </button>
          </nav>

          <div class="settings-panel">
            <!-- 界面 -->
            <template v-if="ui.settingsTab === 'interface'">
              <h3>侧栏区块</h3>
              <p class="hint">勾选后显示在左侧栏；折叠状态在侧栏内点击区块标题切换。</p>
              <div v-for="sec in SIDEBAR_SECTIONS" :key="sec.key" class="form-row">
                <span class="form-label">{{ sec.label }}</span>
                <input
                  type="checkbox"
                  :checked="ui.sidebarSections[sec.key]"
                  @change="ui.setSidebarSectionVisible(sec.key, ($event.target as HTMLInputElement).checked)"
                />
              </div>
            </template>

            <!-- 书库 -->
            <template v-else-if="ui.settingsTab === 'library'">
              <h3>书库信息</h3>
              <div class="form-row"><span class="form-label">书库名</span><input v-model="libName" type="text" :disabled="!conn.writable" /><button v-if="conn.writable" class="btn small" @click="saveName">保存</button></div>
              <div v-if="conn.isAdmin" class="form-row">
                <span class="form-label">元数据存储</span>
                <select :value="storageMode" :disabled="switching" @change="switchStorage(($event.target as HTMLSelectElement).value)">
                  <option value="database">数据库（本地专用，性能好）</option>
                  <option value="toml">配置文件（网盘同步友好）</option>
                </select>
              </div>
              <div class="form-row"><span class="form-label">保存时回写文件</span><input type="checkbox" :checked="embedMetadata" :disabled="!conn.writable" @change="toggleEmbed(($event.target as HTMLInputElement).checked)" /></div>
              <p class="hint">EPUB/PDF 元数据保存时写回文件本身；关闭后改动只存书库。网盘同步场景建议关闭（回写会导致整文件重传）。</p>
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
              <div style="height: 8px" />
              <button v-if="conn.writable" class="btn" :disabled="refreshingCache" @click="refreshCache">重建缺失的封面与全文索引</button>
              <p class="hint">逐本重建走右键菜单「刷新元数据」；强制全部重建可先删除缓存目录内容再重启。</p>
            </template>

            <!-- 锁 -->
            <template v-else-if="ui.settingsTab === 'locks'">
              <h3>锁</h3>
              <p class="hint">四维锁（文件夹/分类/标签/作者），服务端强制：未解锁时对应条目从列表与内容直连中排除。</p>
              <div v-if="lockEntries.length" class="lock-list">
                <div v-for="entry in lockEntries" :key="entry.dim + entry.name" class="lock-row">
                  <span class="chip">{{ DIM_LABEL[entry.dim] }}</span>
                  <span class="lock-name">{{ entry.name }}</span>
                  <button v-if="conn.isAdmin" class="btn small" @click="removeLock(DIM_SINGULAR[entry.dim], entry.name)">解除</button>
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

            <!-- 更新（仅桌面端；状态机见 shared/lib/updater.ts） -->
            <template v-else-if="ui.settingsTab === 'update'">
              <template v-if="hasShell()">
                <h3>应用更新</h3>
                <div class="form-row"><span class="form-label">当前版本</span><span>{{ !buildSha ? '…' : buildSha === 'dev' ? '开发版' : `v${appVersion} · ${buildSha.slice(0, 7)}` }}</span></div>

                <div class="form-row"><span class="form-label">更新通道</span>
                  <label class="channel-radio"><input type="radio" value="stable" :checked="updater.channel.value === 'stable'" @change="updater.setChannel('stable')" /> 稳定版</label>
                  <label class="channel-radio"><input type="radio" value="nightly" :checked="updater.channel.value === 'nightly'" @change="updater.setChannel('nightly')" /> 滚动版（nightly）</label>
                </div>
                <p class="hint">滚动版包含最新改动，稳定性不作保证；切换通道后需重新检查更新。</p>

                <div class="form-row"><span class="form-label">检查更新</span>
                  <span class="update-status">
                    <template v-if="updater.phase.value === 'checking'">正在检查…</template>
                    <template v-else-if="updater.phase.value === 'uptodate'">已是最新（{{ updater.channel.value === 'nightly' ? 'nightly' : '稳定版' }}）</template>
                    <template v-else-if="updater.update.value">
                      发现新版本 {{ updateLabel }}
                      <a :href="updater.update.value.url" target="_blank" rel="noreferrer">发布说明</a>
                    </template>
                    <template v-else-if="updater.phase.value === 'error'">检查失败</template>
                    <template v-else>未检查</template>
                  </span>
                </div>
                <p v-if="updater.error.value" class="update-error">{{ updater.error.value }}</p>

                <div v-if="updater.phase.value === 'downloading'" class="update-progress">
                  <div class="update-progress-bar">
                    <div class="update-progress-fill" :style="{ width: downloadPct !== null ? downloadPct + '%' : '100%' }" :class="{ indeterminate: downloadPct === null }" />
                  </div>
                  <span class="update-progress-text">
                    {{
                      updater.verifying.value
                        ? '校验中…'
                        : updater.progress.value
                          ? `${formatBytes(updater.progress.value.received)} / ${updater.progress.value.total > 0 ? formatBytes(updater.progress.value.total) : '…'}`
                          : '准备下载…'
                    }}
                  </span>
                </div>

                <div class="update-actions">
                  <button class="btn" :disabled="updater.phase.value === 'checking' || updater.phase.value === 'downloading' || updater.phase.value === 'ready'" @click="void updater.check()">检查更新</button>
                  <button v-if="updater.phase.value === 'available'" class="btn primary" @click="void updater.downloadAndInstall()">下载并安装</button>
                  <button v-if="updater.phase.value === 'downloading'" class="btn" @click="void updater.cancel()">取消下载</button>
                  <button v-if="updater.phase.value === 'ready'" class="btn primary" @click="void updater.install()">重启并安装</button>
                </div>
                <p v-if="updater.phase.value === 'ready'" class="hint">安装包已下载并校验。重启后自动完成安装。</p>
                <p v-if="updater.phase.value === 'error'" class="hint">检查/下载失败后可重试；网络受限时 GitHub API 有访问限速（每小时 60 次）。</p>
              </template>
              <template v-else>
                <h3>应用更新</h3>
                <p class="hint">仅桌面端支持自动更新；浏览器形态请直接访问 Release 页获取新版本。</p>
              </template>
            </template>

            <!-- 关于 -->
            <template v-else>
              <div class="about">
                <img src="/icon.png" alt="sumi" />
                <h3>sumi</h3>
                <p>对标 Calibre 的开源书籍/文本资源管理工具</p>
                <p class="hint">daemon {{ conn.appInfo?.version }} · app {{ appVersion || (hasShell() ? '…' : 'web') }} · {{ conn.appInfo?.platform }}</p>
              </div>
            </template>
          </div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.mask {
  position: fixed;
  inset: 0;
  /* 深于主界面；浅于右键菜单（900），低于窗口控制（500）使右上三钮在对话框打开时仍可点 */
  z-index: 450;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.5);
}
/* 对话框尺寸固定（钳制视口）：分区切换/提示出现只改内容区滚动，面板尺寸不变，避免界面跳跃 */
.dialog {
  width: min(720px, calc(100vw - 32px));
  height: min(600px, 88vh);
  display: flex;
  flex-direction: column;
  border-radius: 10px;
  background: var(--panel);
  border: 1px solid var(--border);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
  overflow: hidden;
}
.dialog-head {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px 10px 16px;
  border-bottom: 1px solid var(--border);
}
.dialog-title {
  font-weight: 600;
}
/* 头部关闭钮：28px 方形，风格对齐 TopBar 的 icon-btn */
.dialog-close {
  width: 28px;
  height: 28px;
  display: grid;
  place-items: center;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
}
.dialog-close:hover {
  color: var(--text);
  background: var(--hover);
}
.settings-body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.settings-nav {
  width: 120px;
  flex: none;
  border-right: 1px solid var(--border);
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
.settings-panel {
  flex: 1;
  overflow-y: auto;
  padding: 24px 28px;
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

/* ---- 更新分区 ---- */
.channel-radio {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  margin-right: 16px;
  cursor: pointer;
}
.update-status {
  font-size: 13px;
}
.update-status a {
  color: var(--accent);
  text-decoration: none;
}
.update-error {
  font-size: 12px;
  color: var(--danger);
  margin: -8px 0 12px;
}
.update-progress {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 12px;
}
.update-progress-bar {
  height: 6px;
  border-radius: 3px;
  background: var(--panel-2);
  overflow: hidden;
}
.update-progress-fill {
  height: 100%;
  border-radius: 3px;
  background: var(--accent);
}
/* total 未知时不定态进度条 */
.update-progress-fill.indeterminate {
  width: 40%;
  animation: update-indeterminate 1.1s ease-in-out infinite;
}
@keyframes update-indeterminate {
  from {
    transform: translateX(-100%);
  }
  to {
    transform: translateX(250%);
  }
}
.update-progress-text {
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}
.update-actions {
  display: flex;
  gap: 8px;
  margin-bottom: 8px;
}
</style>
