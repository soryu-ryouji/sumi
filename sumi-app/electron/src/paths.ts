// 运行期路径常量：应用自有配置目录（全平台统一 ~/.config/sumi）与产物目录。
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** 仓库内 sumi-app/ 根（源文件 electron/src/paths.ts → 上两级；开发态源码结构即打包态资源结构） */
export const APP_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

/** electron 产物目录（electron/out，preload.cjs 与 main.mjs 同目录） */
export const ELECTRON_DIR = path.join(APP_DIR, 'electron', 'out');

/** 应用自有配置目录：只放 config.toml 等自有文件（Electron 会话数据走 userData，见 main.ts） */
export function configDir(): string {
  // 允许测试覆盖（HOME 隔离）
  const base = process.env.SUMI_CONFIG_DIR ?? path.join(process.env.HOME ?? '', '.config', 'sumi');
  return base;
}

/** 窗口/托盘图标（512px 源图；打包后各平台图标由 electron-builder 嵌入，此处服务开发态） */
export const APP_ICON = path.join(APP_DIR, 'build', 'icon.png');
