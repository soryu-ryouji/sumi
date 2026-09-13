// 从仓库固化的 OpenAPI schema（sumi-daemon/openapi.json，cargo run -- --dump-openapi 产出、
// 契约测试保证同步）生成 TS 类型到 web/src/shared/api/schema.d.ts。
// 无需拉起 daemon——schema 是代码生成的固化文件，直接读。
// 用法：npm run gen:types
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const schemaFile = path.join(root, '..', 'sumi-daemon', 'openapi.json');
if (!fs.existsSync(schemaFile)) {
  console.error('schema 不存在: sumi-daemon/openapi.json（在 sumi-daemon/ 执行 cargo run -- --dump-openapi > openapi.json）');
  process.exit(1);
}

// .bin 下的 shim 在 Windows 上是 shell 脚本，直接定位包内真实 CLI 文件
const cli = path.join(root, 'node_modules', 'openapi-typescript', 'bin', 'cli.js');
const out = path.join(root, 'web', 'src', 'shared', 'api', 'schema.d.ts');
execFileSync(process.execPath, [cli, schemaFile, '-o', out]);
console.log(`已生成 ${out}`);
