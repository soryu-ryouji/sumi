# 技术栈

## 总览

```text
后端：Rust
前端：Vue 3 + TypeScript (Vite)，只走 HTTP（规划，沿用 hawk 同栈）
桌面壳：Electron，sidecar 拉起后端
契约：OpenAPI
```

## 后端：sumi-daemon/（规划）

Rust，见 [sumi-daemon](backend/server-rust.md)。核心依赖：axum、notify、blake3、rusqlite（bundled + FTS5）、zip + quick-xml（epub/docx 解包解析）、chardetng + encoding_rs（txt 编码归一）、image + fast_image_resize + libwebp（封面生成）。

书籍格式的解析与转换选型（epub/mobi/azw3/pdf/docx/cbz 的元数据、封面、目录、正文归一化）是 sumi 相对 hawk 的全部增量复杂度所在，逐格式选型与实现蓝图见 [server-rust.md](backend/server-rust.md)，全文检索的中文分词设计见 [fulltext-search.md](backend/fulltext-search.md)。

## 前端与桌面壳：sumi-app/（规划）

Electron 壳 + Vue 3 前端（Composition API + `<script setup>` + Pinia），沿用 hawk 的成熟栈（团队熟悉度、工具链复用）。约束：

- 前端只通过 REST API 与后端通信，不依赖 Electron IPC
- Electron 主进程只负责：创建窗口、拉起/回收后端进程、注入 token
- electron-builder 打包，`extraResources` 携带各平台后端二进制

局域网 web 书库查看器复用同一份 web 前端（由 daemon 的 `[web]` 监听托管，`--web-dist` 指向构建产物），viewer token 权限降级由前端按 `app/info` 的 `access` / `writable` 自适应。

## 契约：OpenAPI

- OpenAPI schema 由后端代码生成（utoipa：`#[utoipa::path]` + `ToSchema` derive，路由即文档），固化于 `sumi-daemon/openapi.json`（`cargo run -- --dump-openapi` 重新生成）
- 固化文件与代码的同步由契约测试保证（`cargo test`：改 API 后忘 dump 即测试失败）；同时校验全部端点的真实响应符合 schema、SSE 事件名与常量定义双向一致
- TypeScript 类型从 schema 生成（openapi-typescript，`npm run gen:types`）；CI 校验生成产物与仓库同步
- 当前阶段的接口契约即 [REST API V1](backend/server-rest-api-v1.md) 文档；实现启动后以 schema 为准、文档与 schema 同步维护
