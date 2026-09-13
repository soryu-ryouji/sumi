// 发布当前平台的 sumi-daemon（Rust，cargo build --release）到 resources/sumi-daemon/（electron-builder 的 extraResources 来源）。
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const RIDS = {
  'win32-x64': 'x86_64-pc-windows-msvc',
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
};

// 沿用名 → rust target（也接受直接传 target triple）
const ALIASES = {
  'win-x64': 'x86_64-pc-windows-msvc',
  'osx-arm64': 'aarch64-apple-darwin',
  'osx-x64': 'x86_64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
};

const arg = process.argv[2] ?? RIDS[`${process.platform}-${process.arch}`];
const target = ALIASES[arg] ?? arg;
if (!target) {
  console.error(`未知平台 ${process.platform}-${process.arch}，请显式传入 RID`);
  process.exit(1);
}

// 非本机 target：先安装交叉编译目标
const native = RIDS[`${process.platform}-${process.arch}`];
if (target !== native) {
  const add = spawnSync('rustup', ['target', 'add', target], { stdio: 'inherit' });
  if (add.status !== 0) process.exit(add.status ?? 1);
}

const rsDir = path.join(root, '..', 'sumi-daemon');
const result = spawnSync(
  'cargo',
  ['build', '--release', '--manifest-path', path.join(rsDir, 'Cargo.toml'), '--target', target],
  { stdio: 'inherit' },
);
if (result.error || result.status !== 0) {
  process.exit(result.status ?? 1);
}

const exe = target.includes('windows') ? 'sumi-daemon.exe' : 'sumi-daemon';
const built = path.join(rsDir, 'target', target, 'release', exe);
const out = path.join(root, 'resources', 'sumi-daemon');
fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(out, { recursive: true });
fs.copyFileSync(built, path.join(out, exe));
console.log(`已发布 ${target} → resources/sumi-daemon`);
