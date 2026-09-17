// 应用更新渲染层编排（Electron 桌面端；检查/下载/安装语义在主进程，见 electron/src/updater.ts）。
// - 模块级共享状态：设置「更新」分区与启动静默检查共用
// - phase 状态机：idle → checking →（uptodate | available | error）→ downloading → ready
// - 通道偏好存 localStorage；切换通道后旧检查结果作废，需重新检查
// - 静默检查（主界面就绪后延迟一次，App.vue 触发）：发现新版本 toast 一次，按 通道@版本 去重
import { ref } from 'vue';
import { shell } from '@/app/shell';
import { useUi } from '@/stores/ui';
import { UPDATE_CANCELLED, type UpdateInfo, type UpdateProgress } from '@/shared/api/types';

export type UpdaterPhase = 'idle' | 'checking' | 'uptodate' | 'available' | 'downloading' | 'ready' | 'error';

const CHANNEL_KEY = 'sumi.updateChannel';
const NOTICE_KEY = 'sumi.lastUpdateNotice';

function readLocal(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeLocal(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // 存储不可用（隐私模式等）时仅保持会话内状态
  }
}

const channel = ref<'stable' | 'nightly'>(readLocal(CHANNEL_KEY) === 'nightly' ? 'nightly' : 'stable');
const phase = ref<UpdaterPhase>('idle');
const update = ref<UpdateInfo | null>(null);
const error = ref<string | null>(null);
const progress = ref<{ received: number; total: number } | null>(null);
const verifying = ref(false);

// 进度订阅模块级注册一次（preload 在应用运行前注入，模块加载时 shell 已可用）
const shellAtLoad = shell();
if (shellAtLoad) {
  shellAtLoad.onUpdateProgress((p: UpdateProgress) => {
    if (p.phase === 'downloading') {
      phase.value = 'downloading';
      verifying.value = false;
      progress.value = { received: p.received, total: p.total };
    } else if (p.phase === 'verifying') {
      verifying.value = true;
    } else if (p.phase === 'ready') {
      phase.value = 'ready';
    }
  });
}

/** 切换更新通道并持久化；旧检查结果作废 */
function setChannel(next: 'stable' | 'nightly'): void {
  if (next === channel.value) {
    return;
  }
  channel.value = next;
  writeLocal(CHANNEL_KEY, next);
  phase.value = 'idle';
  update.value = null;
  error.value = null;
  progress.value = null;
  verifying.value = false;
}

/** 检查当前通道更新。silent=true 供启动静默检查：失败静默，发现新版本 toast（同一版本只提示一次） */
async function check(silent = false): Promise<UpdateInfo | null> {
  const s = shell();
  if (!s || phase.value === 'checking' || phase.value === 'downloading') {
    return null;
  }
  phase.value = 'checking';
  error.value = null;
  try {
    const info = await s.checkUpdate(channel.value);
    if (!info) {
      phase.value = 'uptodate';
      update.value = null;
      return null;
    }
    update.value = info;
    // 磁盘缓存命中（同版本已下载校验通过，如「下完未装就退出」）：跳过下载直接可安装
    phase.value = info.downloaded ? 'ready' : 'available';
    if (silent) {
      const id = `${info.channel}@${info.version}`;
      if (readLocal(NOTICE_KEY) !== id) {
        writeLocal(NOTICE_KEY, id);
        useUi().toast(`发现新版本 ${info.channel === 'nightly' ? `nightly ${info.version}` : `v${info.version}`}，设置 → 更新 可安装`);
      }
    }
    return info;
  } catch (e) {
    phase.value = 'error';
    error.value = e instanceof Error ? e.message : String(e);
    return null;
  }
}

/** 下载并校验当前检查到的更新；完成（phase=ready）经 onUpdateProgress 事件到达 */
async function downloadAndInstall(): Promise<void> {
  const s = shell();
  if (!s || phase.value !== 'available') {
    return;
  }
  phase.value = 'downloading';
  error.value = null;
  progress.value = null;
  verifying.value = false;
  try {
    await s.downloadUpdate();
    // ready 事件通常先于 invoke resolve 到达；此处兜底（如主进程幂等直接返回）
    if (phase.value === 'downloading') {
      phase.value = 'ready';
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    // 用户取消不是错误：cancel() 已切回 available，此处只兜底时序竞态
    if (msg.includes(UPDATE_CANCELLED)) {
      if (phase.value === 'downloading') {
        phase.value = 'available';
      }
      progress.value = null;
      verifying.value = false;
      return;
    }
    phase.value = 'error';
    error.value = msg;
  }
}

/** 取消进行中的下载（主进程 abort 并清半成品）；回 available 可直接重新下载 */
async function cancel(): Promise<void> {
  const s = shell();
  if (!s || phase.value !== 'downloading') {
    return;
  }
  // 先切态：downloadAndInstall() await 链的 ready 兜底（resolve 时 phase==='downloading'）就不会误触发
  phase.value = 'available';
  progress.value = null;
  verifying.value = false;
  try {
    await s.cancelUpdate();
  } catch {
    // 与下载自然结束的竞态：忽略
  }
}

/** 重启并安装；成功后应用退出不再返回 */
async function install(): Promise<void> {
  const s = shell();
  if (!s || phase.value !== 'ready') {
    return;
  }
  try {
    await s.installUpdate();
  } catch (e) {
    phase.value = 'error';
    error.value = e instanceof Error ? e.message : String(e);
  }
}

/** 启动静默检查（每会话一次）：主界面就绪后延迟触发，避免与启动链路抢资源 */
let autoChecked = false;

export function startupAutoCheck(): void {
  if (autoChecked || !shell()) {
    return;
  }
  autoChecked = true;
  setTimeout(() => {
    void check(true);
  }, 8000);
}

export function useUpdater() {
  return { channel, phase, update, error, progress, verifying, setChannel, check, downloadAndInstall, cancel, install };
}
