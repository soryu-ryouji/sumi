// 拉取 pdfium 预编译动态库（bblanchon/pdfium-binaries）到 resources/pdfium/<平台>/。
// PDF 首页封面渲染依赖；下载失败只警告不阻断（daemon 缺库时 PDF 封面回退排版生成）。
// 用法：node scripts/fetch-pdfium.mjs [RID...]   RID: mac-arm64 / mac-x64 / linux-x64 / win-x64
// 不传参数时取当前平台。
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const outDir = path.join(root, 'resources', 'pdfium');

const NATIVE = {
  'darwin-arm64': 'mac-arm64',
  'darwin-x64': 'mac-x64',
  'linux-x64': 'linux-x64',
  'win32-x64': 'win-x64',
};
const FILE = {
  'mac-arm64': 'lib/libpdfium.dylib',
  'mac-x64': 'lib/libpdfium.dylib',
  'linux-x64': 'lib/libpdfium.so',
  'win-x64': 'bin/pdfium.dll',
};

const args = process.argv.slice(2);
const rids = args.length > 0 ? args : [NATIVE[`${process.platform}-${process.arch}`] ?? null].filter(Boolean);
if (rids.length === 0) {
  console.error(`未知平台 ${process.platform}-${process.arch}，请显式传入 RID`);
  process.exit(1);
}

for (const rid of rids) {
  const dest = path.join(outDir, rid);
  if (fs.existsSync(path.join(dest, path.basename(FILE[rid])))) {
    console.log(`[fetch-pdfium] ${rid} 已存在，跳过`);
    continue;
  }
  const url = `https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-${rid}.tgz`;
  console.log(`[fetch-pdfium] 下载 ${url}`);
  try {
    fs.rmSync(dest, { recursive: true, force: true });
    fs.mkdirSync(dest, { recursive: true });
    const tgz = path.join(dest, 'pdfium.tgz');
    execFileSync('curl', ['-sL', '--retry', '3', '-o', tgz, url], { stdio: 'inherit' });
    execFileSync('tar', ['xzf', tgz, '-C', dest], { stdio: 'inherit' });
    fs.rmSync(tgz, { force: true });
    const lib = path.join(dest, FILE[rid]);
    if (!fs.existsSync(lib)) {
      throw new Error(`解包后未找到 ${FILE[rid]}`);
    }
    console.log(`[fetch-pdfium] ${rid} → ${lib}`);
  } catch (e) {
    console.warn(`[fetch-pdfium] ${rid} 拉取失败（PDF 封面将回退排版生成）: ${e.message}`);
    fs.rmSync(dest, { recursive: true, force: true });
  }
}
