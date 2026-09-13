# 架构设计

## 总体架构

sumi 前后端完全解耦，通过 HTTP API 通信。桌面版中 Electron 只是一个壳：启动时拉起后端进程，退出时一起回收。后端是同一份代码，未来可直接部署为服务器版本。

```text
┌──────────────────────────────────────────────────────┐
│                     用户书库目录                       │
│   三体.epub  论文.pdf  ...（无任何 sumi 文件）          │
│   .sumi/                                             │
│     ├── config.toml / categories.toml / tags.toml /  │
│     │   view.toml / global_filter.toml / locks.toml  │
│     ├── covers/     自定义封面（用户数据，参与同步）    │
│     ├── metadata/   书目参数（配置文件模式，参与同步）  │
│     ├── metadata.db 书目参数（数据库模式，本地专用）    │
│     ├── storage_mode 存储方案标记（参与同步）           │
│     └── trash/      回收站（本地专用）                 │
│   自动封面/全文索引/归一化内容/元数据缓存 → 库外系统缓存  │
└──────────────────────┬───────────────────────────────┘
                       │ 文件系统监听 / 读写
                       ▼
┌──────────────────────────────────────────────────────┐
│                 sumi-daemon (Rust)                    │
│  ┌───────────┐  ┌───────────┐  ┌───────────────────┐  │
│  │ Watcher   │  │ Hash      │  │ Parser/Cover      │  │
│  │ (notify)  │  │ (blake3)  │  │ (epub/mobi/pdf/…) │  │
│  └───────────┘  └───────────┘  └───────────────────┘  │
│  ┌────────────────────────────────────────────────┐  │
│  │ In-Memory Index / FTS5 全文检索                  │  │
│  │（FTS5 索引落库外缓存 index.db，可删重建）          │  │
│  └────────────────────────────────────────────────┘  │
│       REST API (axum, 静态 OpenAPI schema) + SSE     │
└──────────────────────┬───────────────────────────────┘
                       │ HTTP
          ┌────────────┼────────────┐
          ▼            ▼            ▼
    ┌──────────┐ ┌──────────┐ ┌──────────┐
    │ 桌面客户端 │ │ 局域网 web │ │ 生态接入  │
    │(Electron)│ │ 书库查看器 │ │ CLI/插件 │
    └──────────┘ └──────────┘ └──────────┘
```

详见 [技术栈](tech-stack.md)、[存储设计](backend/storage.md) 与 [REST API V1](backend/server-rest-api-v1.md)。

## 核心原则

**1. 前端只认识 HTTP API**

Web 前端不依赖 Electron IPC，只通过 REST API 通信。桌面客户端、局域网 web 查看器、未来的移动端 app 共用同一套接口。

**2. 后端不依赖 Electron**

后端是独立的 Rust 二进制，不依赖任何桌面端代码。

**3. API 契约先行**

REST API 由代码生成 OpenAPI schema（utoipa：`#[utoipa::path]` 标注端点、DTO derive `ToSchema`，路由与文档同一来源），固化于 `sumi-daemon/openapi.json`（改 API 后 `cargo run -- --dump-openapi > openapi.json` 重新固化，契约测试校验同步），TypeScript 类型从 schema 生成（`npm run gen:types`），前后端不允许手写对接口。SSE 事件名与负载同样构成持久契约（事件名以常量集中定义，客户端不许凭代码反推）。

**4. 格式归一化归服务端，阅读器归客户端**

epub/mobi/azw3/docx → 归一化 HTML 的转换、txt/md 的编码归一在**服务端**完成并写入库外派生缓存（按内容哈希寻址、immutable，一次转换终身复用）；客户端阅读器只消费归一化产物（HTML / UTF-8 文本 / 原文件字节），不为每种格式各写一套解析。与 hawk「编辑计算归客户端」方向相反的原因：图片编辑是交互计算（用户实时操作，canvas 足任），格式转换是批处理计算（一次转换、全端复用）；转换器集中在服务端可避免 web/桌面/移动多端实现的口径漂移，转换结果直接进缓存（`item/content`、`item/toc`、`item/resource` 契约见 API 文档）。

**5. 非侵入与内容寻址**

书库目录中除 `.sumi/` 外不出现任何 sumi 文件；item 以内容哈希（BLAKE3）标识，同内容去重、id 随内容漂移并按路径启发式迁移。存储语义（同步边界、双存储模式、封面两条来源链）见 [存储设计](backend/storage.md)。

## 部署形态

### 桌面版（sidecar 模式）

Electron 壳与后端二进制一起打包发行（electron-builder 的 `extraResources`）：

```text
Electron 启动
  → 预选空闲环回端口 + 生成随机 token
  → spawn sumi-daemon 子进程
      - 参数 --library <path> --port <预选端口>，token 经环境变量传入
      - **先监听端口，初始索引后台构建**（先监听、后索引的启动模型）
  → 轮询 GET /api/v1/app/startup（200ms 间隔）：
      starting → 进度帧驱动启动进度页；ready → 加载主界面；error → 弹错误框
  → 初始索引完成前 /api/* 返回 503 NOT_READY（app/startup 除外），/health 503
Electron 退出
  → 回收 sumi-daemon 子进程（防止孤儿进程残留）
```

桌面版默认端口 `27381`（被占用时回退为动态分配）。

**本地 API 安全**：localhost 端口任何本机进程都能访问，因此所有请求必须携带启动时生成的随机 token。token 只存在于进程环境变量中，不落盘。防护对象是浏览器里的恶意网页（CSRF 直写书库）——本机同权限进程可直接读写书库目录，等价绕过，不在防护范围。生态客户端（浏览器插件等）通过默认端口 27381 连接；为免配置，提供免鉴权的 token 发现端点 `GET /api/v1/app/token`：响应不带 CORS 头（跨源网页 JS 读不到，持 host_permissions 的扩展可读）且 Host 限定环回地址（防 DNS rebinding 同源绕过），插件零配置即可接入。

### 局域网 web 书库查看器（1.0 路线图）

同一 sumi-daemon 追加监听 `0.0.0.0:27382`（`[web]` 配置，默认关闭），托管 web 前端静态页面（`--web-dist` 指定目录）。访问经 viewer token 鉴权，权限三档（只读 / 可写 / 拆分读写 token），写能力逐端点收敛为 `READ_ONLY` 403。配置热重绑（`PUT /api/v1/app/lan`）不重启进程。v1 的查看方式为「调起系统默认应用打开」（桌面形态）与原文件下载（web 形态）；内置阅读器是 2.0 路线图内容。

### 服务器版（未来）

同一个 sumi-daemon 直接部署在服务器上，为多个用户服务。与桌面版的差异：

| 差异点   | 桌面版             | 服务器版                      |
| -------- | ------------------ | ----------------------------- |
| 数据来源 | 监听本地文件系统   | 用户上传（`item/upload`）      |
| 索引存储 | 内存索引 + 库外缓存 | 集中式数据库（如 PostgreSQL） |
| 认证     | 随机 token（单机） | 真实的用户认证体系            |

这三处差异在架构上已预留位置：core 的索引读写收在窄接口后面、API 预留认证头位置。但**当前只实现桌面版**，不提前实现服务器版的任何功能。

### 远程访问（2.0 路线图，规划中）

远程访问不走服务器版路线，沿用 hawk 验证过的三进程协作形态：sumi-daemon 本体保持桌面版定位不变，唯一的调用点是鉴权中间件多认一种 env 注入的受托只读 token；广域网连接能力全部收在独立进程 **sumi-remote**（QUIC 隧道 + 本地代理 + 信令心跳）；桌面端主进程是接线枢纽（拉起/传参/回收）。远程查看时数据面为端到端 QUIC 隧道，书籍数据始终由书库所在机的 daemon 产出，复用现有 API 与只读鉴权。详细设计（信令协议、NAT 穿透、hub/relay 云端服务）在 2.0 启动时补充，模式参考 hawk 的 remote-access / remote-protocol 文档。

## 仓库结构（规划）

```text
sumi/
├── sumi-daemon/       ← Rust 后端（桌面版与服务器版共用，见 docs/backend/server-rust.md）
├── sumi-app/          ← 桌面应用（Electron 壳 + Vue 3 web 前端）
├── tools/             ← 仓库级脚本（构建/安装/冒烟/图标生成）
├── .assets/           ← 品牌资产（icon 真源 sumi.svg 与各平台产物）
└── docs/              ← 设计文档
```

浏览器插件、sumi-remote 等生态组件随对应路线图版本引入。
