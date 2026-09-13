// electron-builder 包装：默认降低 7z 压缩级别换取打包速度。
// 二进制工具链下载默认走 npmmirror（国内网络）；用户已设置同名环境变量时尊重用户配置
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

process.env.ELECTRON_BUILDER_COMPRESSION_LEVEL ??= '5';
process.env.ELECTRON_BUILDER_BINARIES_MIRROR ??= 'https://npmmirror.com/mirrors/electron-builder-binaries/';

const cli = path.join(root, 'node_modules', 'electron-builder', 'cli.js');
const result = spawnSync(process.execPath, [cli, ...process.argv.slice(2)], {
  cwd: root,
  stdio: 'inherit',
  env: process.env,
});
process.exit(result.status ?? 1);
