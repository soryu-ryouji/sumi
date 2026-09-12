# sumi-daemon（Rust 实现）

> v1 已按本文实现（单测 + `tools/smoke.sh` 行为冒烟全绿）。「已知简化」一节记录待打磨项。
> API 契约见 [REST API V1](server-rest-api-v1.md)，存储格式见 [storage.md](storage.md)，全文检索设计见 [fulltext-search.md](fulltext-search.md)，进程模型见 [architecture.md](../architecture.md)。

sumi 书籍管理后端，Rust 实现（`sumi-daemon/`）。
行为对齐基准 = OpenAPI schema + `.sumi/` 存储格式 + SSE 事件契约。

相对 hawk 的全部增量复杂度在**格式解析**：epub/mobi/azw3/pdf/docx/cbz 的元数据、封面、目录、正文归一化，以及中文全文检索。其余骨架（索引流水线、双存储模式、回收站、锁、SSE）与 hawk 同构，按 hawk 已验证的模式实现。

## 技术选型

| 职责 | 选型 | 备注 |
| ---- | ---- | ---- |
| HTTP 框架 | axum 0.8 | tower 中间件链，与 hawk 一致 |
| 异步运行时 | tokio | pipeline 消费循环与扫描 runner 均为专用 OS 线程（阻塞扫描不占运行时线程，也不阻塞消费循环） |
| 文件监听 | notify 8 | From/To rename 配对 + 超时兜底 |
| 哈希 | blake3 | item id = BLAKE3 hex（存储契约） |
| epub 解析 | zip + quick-xml，自写 OPF/NCX/nav 解析 | `epub` crate 维护停滞；OPF/NCX 结构简单，自写解析精确可控（对齐 hawk「TOML 手写序列化」的取向） |
| mobi/azw3 解析 | `mobi` crate（PDB 头 + EXTH） | v1 只取封面（EXTH cover record）与书目元数据（EXTH）；KF8 正文提取与全文索引列后续版本 |
| pdf | pdfium-render（PDFium 绑定） | 首页渲染做封面、书签大纲做 toc；正文阅读走 `item/file` + Range（pdf.js），不做服务端转换 |
| docx | zip + quick-xml | core.xml（title/creator）+ document.xml（正文段落/标题样式/图片关系）；OOXML 结构简单，自写转换 |
| txt/md 编码探测 | chardetng + encoding_rs | GBK / Big5 / Shift_JIS / UTF-16 探测归一 UTF-8（中文网文刚需），WHATWG 标准算法 |
| cbz | zip | 枚举图片条目，首张做封面 |
| 图像解码/缩放 | image 0.25 + fast_image_resize | Lanczos3；pdf 首页渲染、封面缩放 |
| WebP 编码 | webp（libwebp） | 有损 q80，与 hawk 一致；纯 Rust 的 image-webp 仅支持无损 |
| 全文索引 | rusqlite（bundled + fts5 feature） | 自定义 tokenizer 经 rusqlite `create_tokenizer` 注册，见 [fulltext-search.md](fulltext-search.md) |
| 元数据缓存 | rusqlite（bundled） | `index.db`，schema 版本不符整库重建 |
| TOML | toml（解析）+ toml_edit（配置就地改键值保注释）+ 手写序列化 | 输出格式精确可控（标量在前、`[[paths]]` 在后） |
| 密码哈希 | argon2 | Argon2id PHC 字符串自含盐与参数（locks.toml） |

## 格式解析管线

各格式的处理矩阵（契约见 API 文档「支持格式」表）：

| 格式 | 书目元数据 | 封面 | 目录（toc） | 正文（content） | 全文索引 |
| ---- | ---------- | ---- | ----------- | --------------- | -------- |
| epub | OPF（title/creator/publisher/date/identifier/language/series via calibre 元） | 内嵌 cover-image / cover.xhtml | NCX / nav（优先 nav） | XHTML 章节拼接（spine 顺序） | ✓ |
| mobi/azw3 | EXTH | EXTH cover record | 内部 TOC | KF8 HTML 提取（**后续版本**） | 后续版本 |
| pdf | DocInfo（title/author） | 首页渲染 | 书签大纲 | 不转换（`item/file` + Range） | 后续版本 |
| txt | 文件名（书名） | 排版生成 | 章节模式启发式 | 编码归一 UTF-8 直出 | ✓ |
| md | front matter / 文件名 | 排版生成 | `#` 标题层级 | UTF-8 直出（客户端渲染） | ✓ |
| docx | core.xml（title/creator） | 排版生成 | 标题样式大纲 | XHTML 转换 | ✓ |
| cbz | 文件名 | 首张图片 | 空 | 不转换（`item/file`，客户端图片阅读器） | ✗ |

关键约定：

- **一次解包出全部派生物**：首扫并行阶段单次解包同出封面 + 书目元数据 + 全文正文（API 文档 `app/status` 的 `cover` 队列承载），item 入库即可完整显示；增量/对账不重算，由读取端兜底（`refresh_cache` 手动补缺失）
- **txt 章节模式启发式**：`第N章` / `Chapter N` / `序章` 等常见模式，识别失败返回单节点（契约见 API 文档 `item/toc`）
- **正文归一化的锚点注入**：epub/docx 转换时在章节起始元素注入 `id`（`anchor:sec-N`），与 `item/toc` 的 `anchor:` href 对应；内嵌图片/字体引用改写为 `item/resource?id=<hash>&path=<内部路径>` 直链。转换输出只保留语义结构，排版样式由客户端注入
- **pdf 首页渲染的失败容忍**：损坏/加密 pdf 封面生成失败不阻断入库（封面 404 占位，`refresh_metadata` 重试），元数据解析失败回退文件名（Item 对象契约）

## 并发模型

沿用 hawk 验证过的流水线模型：

- **索引流水线单写者**：HTTP 写请求与文件监听事件一律作为 Job 入队（有界 channel），专用消费线程串行应用（哈希 → 元数据迁移 → 更新内存索引 → 写存储 → 派发解析任务 → 发 SSE）
- **哈希并行**：物理核估计（逻辑核/2，封顶 16）——哈希 SIMD 密集，SMT 无增益，兄弟线程留给消费循环与 API
- **解析/封面 worker**：`CPU/4` 封顶 8（`app/status` 契约值）；单次解包出封面 + 元数据 + 全文，避免重复 IO
- **查询路径不克隆完整 DTO**：排序在轻量键上进行，`item/list` 只投影分页窗口、`item/skeleton` 只投影轻量骨架（大库下持锁时间降低一个量级）
- **`item/content` 按需转换**：导入不预处理，首次请求同步转换（秒级）写缓存，之后 immutable 命中

## 模块划分（规划）

```text
sumi-daemon/src/
├── api/            ← 端点、信封、鉴权中间件（token 三档 + 锁票据 + viewer 降级）、SSE
├── core/
│   ├── pipeline/   ← 索引流水线（Job 队列、单写者消费循环、扫描 runner）
│   ├── index/      ← 内存索引（item/路径/维度聚合，读路径投影）
│   ├── store/      ← 元数据双模式存储（metadata.db / metadata/*.toml）、注册表文件
│   ├── parser/     ← 格式解析器（epub.rs / mobi.rs / pdf.rs / docx.rs / text.rs / cbz.rs）
│   ├── cover/      ← 封面管线（提取/渲染/排版生成/缩放/webp）
│   ├── fulltext/   ← FTS5 索引（tokenizer、查询映射，见 fulltext-search.md）
│   ├── cache/      ← 库外缓存目录管理（index.db / covers / content）
│   └── events/     ← SSE 事件总线（事件名常量集中定义）
└── main.rs         ← CLI 参数、端口预选、startup 网关
```

依赖单向：`api/` → `core/`。HTTP 请求不直接碰可变索引，写路径一律把变更作为 Job 提交给流水线并等待完成。

## 运行协议

```bash
SUMI_TOKEN=<token> sumi-daemon --library <书库路径> --port 27381 [--web-dist <dir>] [--cache-parent <dir>]
```

| 参数 | 说明 |
| ---- | ---- |
| `--library` | 书库根目录（单实例单库） |
| `--port` | 监听端口（桌面端预选 27381，被占用时由调用方回退动态分配） |
| `--web-dist` | 局域网 web 查看器静态页目录（`[web]` 启用时托管） |
| `--cache-parent` | 库外缓存父目录覆盖（桌面端「存储」设置；缓存目录与库根互相包含时拒绝启动，防缓存污染索引） |

`SUMI_TOKEN` 为 admin token（桌面端全权），只存在于进程环境变量、不落盘；viewer token（`[web]`）在 `.sumi/config.toml` 中，由 daemon 权威写入。

## 已知简化（v1 后续打磨）

- 解析派生（封面/元数据/FTS）在流水线内同步执行；worker 化（`CPU/4` 封顶 8 的 cover 队列）与 `task.progress` 的 500ms 节流推送待接入
- pdf 解析未实现（pdfium 首页渲染/书签依赖动态库，打包阶段处理）：封面 404 占位 + toc 空数组，阅读走 `item/file` + pdf.js
- `item/content` 的 epub/docx 归一化为段落级 HTML（锚点占位注入；资源引用逐属性改写与章节锚点精确对位待增强）；mobi 直出解包内容
- watcher 的 rename 事件按 remove+add 防抖处理（周期扫描兜底收敛）；From/To 精确配对待加
- `app/lan` 的监听 supervisor（`[web]` 热重绑）待接线：配置读写契约已完整
- `index.db` 元数据镜像（大库启动加速）未接：注水直接读权威层
- mobi crate 实际版本 0.8（0.11 不存在）

## 构建与测试

```bash
cd sumi-daemon
cargo build --release          # 产物 target/release/sumi-daemon(.exe)
cargo test                     # 解析器单测（testdata/ 样例书）+ 流水线行为测试 + OpenAPI 契约校验
```

- **testdata/**：每格式准备最小样例书（含中英文混合、多卷、无封面、损坏文件等边界样例），解析器单测逐格式覆盖
- **tools/smoke.sh**：端到端冒烟（18 项行为断言：鉴权/入库/全文/封面/Range/用户编辑保护/分类/锁票据/回收站/文件夹/view/global_filter/rescan/存储迁移/SSE）
- **契约测试**：`openapi.json` 固化文件与代码生成 schema 逐字一致（改 API 后 `cargo run -- --dump-openapi > openapi.json` 重新固化）
PDFium 为 C++ 依赖：pdfium-render 随 crate 携带各平台预编译动态库（`pdfium-build` feature），无需本地工具链；打包时随 extraResources 分发。
