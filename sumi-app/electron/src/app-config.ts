// 用户配置（~/.config/sumi/config.toml，全平台统一；当前库根的会话状态也收敛在此）。
// 目录只放应用自有配置：Electron 会话数据走平台默认 userData（main.ts 固定到 appData/sumi-app）。
import fs from 'node:fs';
import path from 'node:path';
import { parse, stringify } from 'smol-toml';
import { configDir } from './paths';

export interface LibraryHistoryItem {
  path: string;
  /** 目录名（basename） */
  name: string;
  /** 目录仍存在；已删除的历史项展示但不可选 */
  exists: boolean;
}

interface AppConfig {
  libraryPath?: string;
  libraryHistory?: string[];
  /** 全局缓存父目录（所有库共用，子目录按库名+哈希区分）；未设置时用系统缓存目录 */
  cacheParent?: string;
}

export interface LibraryList {
  current: string | null;
  libraries: LibraryHistoryItem[];
}

const CONFIG_FILE = path.join(configDir(), 'config.toml');

/** 读配置（文件缺失/损坏回退空对象；写入只发生在字段变更时） */
export function readConfig(): AppConfig {
  try {
    return parse(fs.readFileSync(CONFIG_FILE, 'utf8')) as AppConfig;
  } catch {
    return {};
  }
}

/** 合并写入（原子：临时文件 + rename） */
export function writeConfig(patch: Partial<AppConfig>): void {
  const merged = { ...readConfig(), ...patch };
  fs.mkdirSync(path.dirname(CONFIG_FILE), { recursive: true });
  const tmp = `${CONFIG_FILE}.tmp`;
  fs.writeFileSync(tmp, stringify(merged));
  fs.renameSync(tmp, CONFIG_FILE);
}

/** 当前库根（渲染进程 showInFinder/openFolder 等按库内相对路径换算绝对路径） */
let libraryRoot: string | null = null;

export function getLibraryRoot(): string | null {
  return libraryRoot;
}

export function setLibraryRoot(p: string): void {
  libraryRoot = p;
}

/** 库历史列表（最近使用在前；path 存在性实时探测） */
export function listLibraries(): LibraryList {
  const config = readConfig();
  const current = config.libraryPath ?? null;
  const libraries = (config.libraryHistory ?? []).map((p) => ({
    path: p,
    name: path.basename(p) || p,
    exists: fs.existsSync(p),
  }));
  return { current, libraries };
}

/** 从历史移除一条（不动目录本身）；当前库被移除时同时清空 current */
export function removeLibraryHistory(p: string): LibraryList {
  const config = readConfig();
  const history = (config.libraryHistory ?? []).filter((x) => x !== p);
  const patch: Partial<AppConfig> = { libraryHistory: history };
  if (config.libraryPath === p) {
    patch.libraryPath = undefined;
  }
  writeConfig(patch);
  return listLibraries();
}

/** 记录库选择：current 落盘 + 历史去重置顶（上限 10） */
export function rememberLibrary(p: string): void {
  const config = readConfig();
  const history = [p, ...(config.libraryHistory ?? []).filter((x) => x !== p)].slice(0, 10);
  writeConfig({ libraryPath: p, libraryHistory: history });
}
