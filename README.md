<p align="center">
  <img src=".assets/icon.png" width="128" alt="sumi logo">
</p>

<h1 align="center">sumi</h1>

<p align="center">对标 Calibre 的开源书籍/文本资源管理工具（hawk 的姊妹项目）</p>

> 名字：sumi（墨）。写作软件 shiro（白）产出的白纸，变成 sumi 收进来的墨迹。

- 非侵入式资源管理：书籍以文件夹的形式进行管理，非侵入式
- 自由开放：开放的 REST API，方便生态接入
- 免费

## 当前状态

**sumi-daemon v1 已实现**（REST API V1 全端点 + SSE + 索引流水线 + 格式解析 + FTS5 全文检索）；**sumi-app 桌面客户端 v1 已实现**（Electron 壳 + Vue 3 前端：书库网格/搜索筛选/元数据编辑/拖拽入库/内置阅读器/回收站/锁/设置）。`cargo test` 与 `tools/smoke.sh` 全绿。

- [架构设计](docs/architecture.md)：进程模型、核心原则、部署形态
- [REST API V1](docs/backend/server-rest-api-v1.md)：接口定义
- [存储设计](docs/backend/storage.md)：`.sumi/` 目录结构、同步边界、元数据与封面存储
- [全文检索设计](docs/backend/fulltext-search.md)：FTS5 中文分词与查询映射
- [技术栈](docs/tech-stack.md)：选型
- [sumi-daemon](docs/backend/server-rust.md)：Rust 后端（实现说明与已知简化）

```bash
# 一键安装（构建并安装 sumi 桌面应用到本机，需 Node.js 与 Rust 工具链）
./tools/install.sh             # Windows: ./tools/install.ps1

# 开发
bash tools/smoke.sh            # 后端端到端冒烟（46 项行为断言）
cd sumi-app && npm run dev     # 桌面客户端开发模式（自动拉起后端）

# 图标产物改版时重新生成（真源 .assets/sumi.svg，产物已入库）
./tools/make-icons.sh
```

## 核心设计（与 hawk 一致）

### 非侵入式资源管理

sumi 不会将你的书籍"导入"到某个专有仓库中。你的文件始终保存在原来的文件夹里，书库目录中也不会出现任何 sumi 的文件——所有数据都收敛在一个 `.sumi/` 隐藏文件夹中。

- 卸载 sumi 后，你的书籍文件纹丝不动
- 原有的文件夹组织习惯完全保留
- 与网盘（Dropbox、iCloud、Syncthing、OneDrive）天然兼容

### 纯文本元数据存储

书目参数（标签、评分、阅读进度等）以独立的纯文本文件存放在 `.sumi/metadata/` 中。网盘同步冲突只影响单本书，可以用 Git 管理书库，没有数据锁定。

本机加速：库外缓存目录维护一份 SQLite 派生缓存（含全文检索 FTS5 索引），可随时删除、重启自动重建，不参与同步。

### 前后端解耦

后端是独立的 Rust 服务，前端只通过 REST API 通信。桌面版用 Electron 壳拉起后端进程；同一套后端未来可直接部署为多人使用的服务器版本。

### 开放 REST API

```text
# 搜索书籍（书名/作者 + 全文）
POST http://localhost:27381/api/v1/item/list
{ "keywords": ["科幻"], "content": "黑暗森林", "read_status": "reading" }

# 获取封面
GET http://localhost:27381/api/v1/item/cover?id=abc123

# 获取归一化阅读内容（epub/mobi/docx → HTML，txt/md → UTF-8 文本）
GET http://localhost:27381/api/v1/item/content?id=abc123

# 用系统默认应用打开（v1 的查看方式）
POST http://localhost:27381/api/v1/item/open
{ "id": "abc123" }

# 更新阅读进度
POST http://localhost:27381/api/v1/item/update
{ "id": "abc123", "progress": 42.5, "read_status": "reading" }
```

## 路线图

- 1.0 版本
  - [x] 实现 sumi-daemon（REST API V1，`sumi-daemon/`）
  - [x] 实现 Windows, macOS, Linux 桌面客户端（`sumi-app/`，Electron + Vue 3）
  - [ ] 实现 web 书库查看器：局域网内通过浏览器访问书库（前端已按 viewer 权限自适应，daemon 监听接线待打包阶段）
- 2.0 版本
  - [ ] 内置阅读器（pdf / txt / md / epub / mobi / azw3 / docx，API 契约见 `item/toc` / `item/content` / `item/resource`）
  - [ ] 实现 sumi remote 协议，支持广域网书库查看
- 3.0 版本
  - [ ] 实现 ios, android 等移动端 app

## 许可证

待定（hawk 为 AGPL-3.0）。
