// 构建 sumi-update（Windows 更新辅助程序，仓库根 sumi-update/）到 resources/sumi-update/，
// 打包时经 electron-builder.yml 的 win.extraResources 进产物（仅 Windows 更新路径使用）。
// 非 Windows 平台直接跳过（CI 无条件调用本脚本，跨平台构建不用按平台拆步骤）。
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

if (process.platform !== 'win32') {
  console.log('非 Windows 平台，跳过 sumi-update 构建');
  process.exit(0);
}

const rsDir = path.join(root, '..', 'sumi-update');
const result = spawnSync('cargo', ['build', '--release', '--manifest-path', path.join(rsDir, 'Cargo.toml')], {
  stdio: 'inherit',
});
if (result.error || result.status !== 0) {
  process.exit(result.status ?? 1);
}

const out = path.join(root, 'resources', 'sumi-update');
fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(out, { recursive: true });
fs.copyFileSync(path.join(rsDir, 'target', 'release', 'sumi-update.exe'), path.join(out, 'sumi-update.exe'));
console.log('已发布 x86_64-pc-windows-msvc → resources/sumi-update');
