# Server REST API V1

## 通用约定

### 请求与响应

- 所有端点前缀 `/api/v1`
- 请求参数与响应字段均使用 snake_case
- 响应统一信封：

```json
{ "status": "success", "data": {} }
```

- 错误响应（HTTP 状态码同时为 4xx/5xx）：

```json
{
  "status": "error",
  "error": { "code": "ITEM_NOT_FOUND", "message": "item abc123 not found" }
}
```

错误码：`INVALID_PARAM`、`ITEM_NOT_FOUND`、`FOLDER_NOT_FOUND`、`FILE_EXISTS`、`UNSUPPORTED_FORMAT`、`CATEGORY_NOT_FOUND`、`CATEGORY_EXISTS`、`TAG_NOT_FOUND`、`AUTHOR_NOT_FOUND`、`SERIES_NOT_FOUND`、`OPEN_FAILED`（系统调用打开文件失败，如未安装默认应用）、`INTERNAL`、`READ_ONLY`（只读 viewer token 访问写端点，403；见存储设计的 `[web]` 写权限/token 拆分配置）、`UNAUTHORIZED`（token 缺失或无效，401）、`NOT_READY`（初始索引构建中，503；唯一放行端点为 `app/startup`）

### ID 规范

- **item id**：文件内容的 BLAKE3 哈希（hex），与存储设计一致
- **library**：不使用合成 id，以书库根目录路径区分；显示名取 `config.toml` 的 `name`，缺省为库目录名
- **folder**：不使用合成 id，直接以相对书库根目录的真实目录路径标识（如 `novels/2024`）
- **author / series / tag / category**：以名称标识；author 与 series 为派生维度（来自 item 元数据聚合，无注册表）

### 支持格式

v1 可见扩展名白名单（`.sumi/config.toml` 的 `extensions` 可覆盖）：

| 格式 | 封面来源 | 元数据解析 | 全文索引 | 阅读路径 |
| ---- | -------- | ---------- | -------- | -------- |
| epub | 内嵌封面 | OPF（title/creator/publisher/date/identifier/language/series） | ✓ | `item/content` |
| pdf | 首页渲染 | DocInfo（title/author） | 后续版本 | `item/file`（Range + pdf.js） |
| txt / md | 排版生成（书名+作者） | 文件名 / front matter | ✓ | `item/content`（编码归一 UTF-8） |
| mobi / azw3 | 内嵌封面 | EXTH / KF8 元数据 | 后续版本 | `item/content`（服务端转换） |
| docx | 排版生成（书名+作者） | core.xml（title/creator） | ✓ | `item/content`（服务端转换） |
| cbz | 首张图片 | 文件名 | ✗ | `item/file`（客户端图片阅读器） |

旧版 `.doc`（二进制格式）不在 v1 白名单内，需 LibreOffice 类外部转换器，列作后续版本。

### 其他

- 时间戳均为 Unix 毫秒
- 分页参数：`offset`（默认 0）、`limit`（默认 50）
- 桌面版默认监听 `27381` 端口（被占用时回退为动态分配；局域网 web 查看默认 `27382`），所有请求需携带启动时下发的 token（`Authorization: Bearer <token>`）；SSE 与直链（cover、file、toc、content、resource）无法设置请求头，改用查询参数 `?token=`

## app

| 方法 | 端点               | 说明         |
| ---- | ------------------ | ------------ |
| GET  | `/api/v1/app/info` | 获取应用信息 |
| GET  | `/api/v1/app/startup` | 启动状态与索引构建进度（就绪网关唯一放行端点） |
| GET  | `/api/v1/app/status` | 后台任务积压（封面队列 + 索引管道；SSE 的 `task.progress` 事件为同一快照的推送版） |
| GET  | `/api/v1/app/token` | 发现连接 token（免鉴权，仅限扩展类客户端） |
| GET  | `/api/v1/app/lan` | 读取局域网 web 查看配置与运行状态（admin 限定） |
| PUT  | `/api/v1/app/lan` | 写回 `[web]` 配置并热重绑局域网监听（admin 限定，失败自动回滚） |

### startup

`GET /api/v1/app/startup`

查询启动状态。server 先监听端口、初始索引后台构建；客户端轮询此端点获取进度（200ms 左右间隔为宜），也是索引完成前唯一可用的 API。

#### 响应

索引构建中：

```json
{
  "status": "success",
  "data": { "status": "starting", "phase": "hash", "processed": 343, "total": 800 }
}
```

就绪：

```json
{ "status": "success", "data": { "status": "ready" } }
```

初始索引失败（进程不退出，修复后需重启）：

```json
{ "status": "success", "data": { "status": "error", "message": "..." } }
```

| 字段 | 类型 | 说明 |
| ---- | ---- | ---- |
| status | string | `starting` / `ready` / `error` |
| phase | string | 仅 starting 时存在：`scan`（遍历清单，total 恒 0）/ `hash`（并行算哈希）/ `apply`（应用索引）/ `sync`（元数据对账，初始扫描前，total 恒 0） |
| processed / total | number | 仅 starting 时存在；`total=0` 表示该阶段总量未知（客户端宜显示不定态进度） |
| message | string | 仅 error 时存在：失败原因 |

### status

`GET /api/v1/app/status`

后台任务积压快照：封面（含元数据解析、全文索引）worker 队列与索引管道。导入或初次索引后大量书籍等待提取封面时，`cover.pending`（排队中）与 `cover.active`（生成中，并发度 `CPU/4`、封顶 8）大于 0；批量文件入库期间 `index.pending`（管道排队 job + 写入防抖路径）大于 0，扫描期间 `index.active` 为 1 并携带阶段进度。积压清空后归零。

SSE 客户端建议直接订阅 `task.progress` 事件（同一快照的推送版，服务端 500ms 节流）；本端点供轮询型客户端使用。

#### 响应

```json
{
  "status": "success",
  "data": {
    "cover": { "pending": 236, "active": 4 },
    "index": { "pending": 12, "active": 0, "phase": "hash", "processed": 1800, "total": 4200 },
    "last_scan": { "started_unix_ms": 1788874284637, "duration_ms": 133, "files": 0, "dirty_dirs": 0, "applied": 0 },
    "queue_overflow": false,
    "sse_lagged": 0
  }
}
```

`index.phase` / `processed` / `total` 仅扫描进行中存在：`total=0` 的遍历阶段表示总量未知（客户端宜显示不定态进度）；空闲时为 `null`。

观测字段（均可缺省，不影响客户端）：

| 字段 | 说明 |
| ---- | ---- |
| `last_scan` | 最近一轮全库扫描统计（未跑过为 null）：开始时间、耗时、枚举文件数、深入目录数、实际应用数 |
| `queue_overflow` | 索引队列曾溢出（已触发兜底扫描）；下一轮扫描收尾后复位 |
| `config_error` | `.sumi/config.toml` 解析错误（**保留上次有效配置继续运行**，修复后自动清除）；正常不出现 |
| `sse_lagged` | SSE 订阅因消费落后被断开（lagged）的累计次数 |

### info

`GET /api/v1/app/info`

获取当前 sumi-daemon 的运行信息，可用于判断客户端环境能力。

#### 响应

```json
{
  "status": "success",
  "data": {
    "version": "1.0.0",
    "platform": "macos",
    "exec_path": "/Applications/sumi.app/sumi-daemon",
    "access": "admin",
    "writable": true,
    "lan": { "active": true, "port": 27382 }
  }
}
```

| 字段      | 类型   | 说明                          |
| --------- | ------ | ----------------------------- |
| version   | string | 后端版本号                    |
| platform  | string | `windows` / `macos` / `linux` |
| exec_path | string | 后端可执行文件路径            |
| access    | string | 当前 token 的访问级别：`admin`（桌面端全权）/ `viewer`（局域网 web 查看器 token，见存储设计的 `[web]` 配置） |
| writable  | boolean | 当前 token 是否可执行写操作：admin 恒 true；viewer 取决于 `[web]` 的写权限配置，保存即热生效 |
| lan       | object | 局域网监听实况：`active`（是否在监听）、`port`（active 时存在）、`error`（绑定失败原因，如端口被占用）。配置读写走 `app/lan`（PUT 内置收敛等待），此字段供状态展示 |

### health

`GET /health`

就绪探活，不带 `/api/v1` 前缀，无需 token。**初始索引完成前返回 503**，完成后 200。只用于区分进程状态，进度与就绪判断以 `app/startup` 为准。

### token

`GET /api/v1/app/token`

免鉴权的 token 发现端点，供浏览器插件等生态客户端零配置接入。

安全性约束：

- 响应**不携带 CORS 头**：跨源网页 JS 无法读取响应，只有持 `host_permissions` 的浏览器扩展能读
- **Host 必须是环回地址**（`127.0.0.1` / `localhost` / `::1`），否则返回 `INVALID_HOST`：防 DNS rebinding 伪装同源读取

拿到 token 后，其余 `/api/*` 请求照常 `Authorization: Bearer <token>`。

#### 响应

```json
{ "status": "success", "data": "<random-token>" }
```

### lan

`GET /api/v1/app/lan`

读取局域网 web 查看配置（`.sumi/config.toml` 的 `[web]` 段）与监听运行状态。**仅 admin 可用**：viewer（含可写）返回 403 `READ_ONLY`——响应含 token 字段，只读 viewer 拿到 write_token 即提权。

#### 响应

```json
{
  "status": "success",
  "data": {
    "enabled": true,
    "port": 27382,
    "token": "<viewer-token>",
    "writable": false,
    "separate_write_token": false,
    "write_token": "",
    "active": true
  }
}
```

`active` 为监听实况，`error` 字段在绑定失败时携带原因。

---

`PUT /api/v1/app/lan`

写回 `[web]` 配置并热重绑局域网监听：daemon 权威写配置（toml_edit 就地改键值，文件其余段与注释保留，原子写）→ 唤醒 supervisor 重绑 → 等待本轮收敛后返回。绑定失败（端口占用等）**自动回滚旧配置**并返回错误，不重启进程、不断 SSE。仅 admin 可用。

#### 请求

```json
{
  "enabled": true,
  "port": 27382,
  "token": "<viewer-token>",
  "writable": false,
  "separate_write_token": false,
  "write_token": ""
}
```

校验（`INVALID_PARAM`）：`enabled` 时 `token` 必填；`writable` 且 `separate_write_token` 时 `write_token` 必填；端口 1–65535。响应同 GET（收敛后的最新状态）。

## library

sumi-daemon 单实例对应单个书库。

| 方法 | 端点                      | 说明               |
| ---- | ------------------------- | ------------------ |
| GET  | `/api/v1/library/info`    | 获取当前书库信息 |
| PATCH | `/api/v1/library/info`   | 改库显示名（`{"name": "..."}`，写 `.sumi/config.toml`，广播 `library.updated`） |
| PUT | `/api/v1/library/scan`   | 周期兜底重扫设置（`{"periodic": bool, "interval": 秒}`，写 `.sumi/config.toml` 的 `[scan]`，保存即热生效） |
| POST | `/api/v1/library/storage_mode` | 切换元数据存储方案（`{"mode": "database" \| "toml"}`），全量迁移后调用方应重启进程 |
| POST | `/api/v1/library/reindex` | 全量重建索引       |
| POST | `/api/v1/library/rescan`  | 强制重新遍历文件系统 |
| POST | `/api/v1/library/refresh_cache` | 按范围刷新派生缓存（补封面 + 全文索引 + 消失对账） |
| POST | `/api/v1/library/cleanup_index` | 索引体检：清除隐藏文件/ignore 命中/源文件已删的残留位置 |

### info

`GET /api/v1/library/info`

获取当前打开的书库信息。文件夹树通过 `folder/list` 获取。

#### 响应

```json
{
  "status": "success",
  "data": {
    "name": "我的书库",
    "path": "/Users/ryouji/Books",
    "modification_time": 1592461625783,
    "application_version": "1.0.0",
    "storage_mode": "database",
    "scan": { "periodic": true, "interval": 900 }
  }
}
```

`storage_mode` 为元数据存储方案：`database`（`.sumi/metadata.db`，默认）/ `toml`（`.sumi/metadata/*.toml`，网盘同步友好）。

### update

`PATCH /api/v1/library/info`

改库显示名：写库内 `.sumi/config.toml` 的 `name` 键（toml_edit 保注释，保存即热更）；空白名清除自定义名（回退库目录名）。成功后就地广播 `library.updated`（负载为完整库信息），各客户端对齐无需重拉。响应同 `info`。

#### 请求

| 参数 | 类型   | 必填 | 说明         |
| ---- | ------ | ---- | ------------ |
| name | string | 是   | 新显示名     |

### scan

`PUT /api/v1/library/scan`

周期兜底重扫设置：写库内 `.sumi/config.toml` 的 `[scan]` 段（toml_edit 保注释，保存即热生效）。返回更新后的库信息并广播 `library.updated`。

语义：**实时监听仍是发现新增/删除的主路径**；周期重扫只是监听静默漏事件时的最终一致兜底（强制遍历、按 size/mtime 复用哈希，不读文件内容）。关闭后漏掉的事件需手动 `POST /library/rescan`（可带 `path`）收敛。

#### 请求

| 参数 | 类型 | 必填 | 说明 |
| ---- | ---- | ---- | ---- |
| periodic | bool | 是 | 是否开启周期兜底重扫 |
| interval | number | 否 | 重扫间隔（秒，下限 60）；缺省沿用当前值 |

#### 响应

同 `GET /library/info`（含 `scan` 字段）。

### reindex

`POST /api/v1/library/reindex`

全量重建索引：重新扫描书库目录并对所有文件重算哈希。异步执行，立即返回；过程中的变更照常通过 `events` 推送。用于 sumi 未运行期间直接改动过书库目录、或对索引状态存疑时手动触发。

#### 响应

```json
{ "status": "success" }
```

### rescan

`POST /api/v1/library/rescan`

重新扫描（文件系统层）：忽略文件夹快照强制重新遍历文件做复用判定（不读文件内容），收敛监听漏事件与直接改目录。异步执行，立即返回；进度经 `task.progress`（`index` 任务）可见，新增/删除经 SSE 事件自动刷新。

请求体可省略；带 `path` 时只重扫该子树（含子目录）：**范围内做消失对账**（监听漏掉的删除一并收敛），但不做全局对账与目录快照替换（只看到子树，全局对账会误删其余位置），该子树的目录快照在下一轮全库扫描时自然收敛。范围目录不可读/已删除时跳过消失对账（防误删）。

#### 请求

| 参数 | 类型   | 必填 | 说明 |
| ---- | ------ | ---- | ---- |
| path | string | 否   | 库内相对路径（文件夹）；缺省/空 = 整库。不存在返回 `FOLDER_NOT_FOUND`，非法路径返回 `INVALID_PARAM` |

#### 响应

```json
{ "status": "success" }
```

### refresh_cache

`POST /api/v1/library/refresh_cache`

按范围刷新派生缓存（**补缺失模式**）：对范围内全部 item 派发后台修复任务——提取缺失封面 + 解析缺失书目元数据 + 补建缺失全文索引，**不重建已有数据**。异步执行立即返回，积压经 `task.progress`（`cover` 任务）可见；修复项经 `item.updated` 事件自动刷新。

用户遇到显示异常（封面 404 占位不恢复等）时的手动修复入口。范围选项对应前端侧栏的右键菜单：文件夹（含子目录）/ 分类 / 标签 / 作者 / 整库。

#### 请求

| 参数  | 类型   | 必填 | 说明                                                                 |
| ----- | ------ | ---- | -------------------------------------------------------------------- |
| type  | string | 是   | `folder` \| `category` \| `tag` \| `author` \| `library`              |
| value | string | 否   | folder/category/tag/author 的名称（folder 为目录相对路径，空串 = 库根）；library 时忽略 |

#### 响应

```json
{
  "status": "success",
  "data": { "dispatched": 42, "removed": 0 }
}
```

`dispatched` 为实际入队的修复任务数（in-flight 去重丢弃或源文件已不在的不计）；`removed` 为消失对账移除的失效位置数（源文件已删但索引残留的条目，经 SSE 推送收敛）。

### cleanup_index

`POST /api/v1/library/cleanup_index`

索引体检：清除不该在索引里的条目——隐藏文件（路径任一段以 `.` 开头，如 `.DS_Store`）、ignore 规则命中或扩展名不在可见白名单内、源文件已删除的残留。**只摘索引位置不动磁盘文件**，移除经流水线单写者执行并广播事件（UI 自动收敛）；回收站条目不参与。与 `refresh_cache` 的差异：后者只对账源文件消失，本端点还清「文件还在但不该入库」的早期版本残留。多位置书籍只摘命中位置，其余位置保留时条目仍在。同步执行后返回统计。

#### 响应

```json
{
  "status": "success",
  "data": { "checked": 120, "removed": 3, "hidden": 1, "ignored": 1, "missing": 1 }
}
```

| 字段 | 说明 |
| ---- | ---- |
| checked | 检查的索引位置数（不含回收站） |
| removed | 清除的条目总数（= hidden + ignored + missing） |
| hidden | 隐藏文件命中数 |
| ignored | ignore 规则命中或扩展名不在可见白名单内（`.sumi/config.toml` 的 `extensions`） |
| missing | 源文件已消失的残留数 |

## folder

folder 即书库中的真实目录。对 folder 的操作会直接操作文件系统，由文件监听同步到索引。

| 方法 | 端点                     | 说明         |
| ---- | ------------------------ | ------------ |
| GET  | `/api/v1/folder/list`    | 列出文件夹树 |
| POST | `/api/v1/folder/create`  | 创建文件夹   |
| POST | `/api/v1/folder/update`  | 更新文件夹   |
| POST | `/api/v1/folder/delete`  | 删除文件夹   |
| POST | `/api/v1/folder/restore` | 恢复文件夹   |

### list

`GET /api/v1/folder/list`

返回完整文件夹树。节点字段：`path`（相对库根目录）、`name`、`children`、`modification_time`。

### create

`POST /api/v1/folder/create`

#### 请求

| 参数        | 类型   | 必填 | 说明                                         |
| ----------- | ------ | ---- | -------------------------------------------- |
| name        | string | 是   | 文件夹名称                                   |
| parent_path | string | 否   | 父文件夹路径（相对库根目录），缺省为库根目录 |

#### 响应

```json
{
  "status": "success",
  "data": {
    "path": "novels/2024",
    "name": "2024",
    "children": [],
    "modification_time": 1592409993367
  }
}
```

### update

`POST /api/v1/folder/update`

#### 请求

| 参数        | 类型   | 必填 | 说明                           |
| ----------- | ------ | ---- | ------------------------------ |
| path        | string | 是   | 文件夹路径                     |
| name        | string | 否   | 新名称（重命名即移动真实目录） |
| parent_path | string | 否   | 新父目录路径（移动真实目录）   |

#### 响应

同 `folder/create`。

### delete

`POST /api/v1/folder/delete`

删除文件夹：将目录（含其中书籍）整体移入 `.sumi/trash/`。目录已不存在时按幂等删除处理：清除该路径前缀下的索引位置与目录设置残留（排序偏好/全局隐藏项）并返回成功（外部删除后的陈旧条目可经此入口清理）。

#### 请求

| 参数 | 类型   | 必填 | 说明       |
| ---- | ------ | ---- | ---------- |
| path | string | 是   | 文件夹路径 |

#### 响应

```json
{ "status": "success" }
```

### restore

`POST /api/v1/folder/restore`

从回收站恢复文件夹：按原路径放回。原路径已被占用时返回 `FILE_EXISTS`。

#### 请求

| 参数 | 类型   | 必填 | 说明           |
| ---- | ------ | ---- | -------------- |
| path | string | 是   | 原库内相对路径 |

#### 响应

```json
{ "status": "success" }
```

## item

| 方法 | 端点                             | 说明           |
| ---- | -------------------------------- | -------------- |
| POST | `/api/v1/item/list`              | 查询 item 列表 |
| POST | `/api/v1/item/skeleton`          | 全量布局骨架（封面尺寸，不分页，与 list 同序） |
| POST | `/api/v1/item/aggregate`          | 选择集共有特性聚合：`{ ids }` → 标签/分类/作者交集（多选面板数据源） |
| GET  | `/api/v1/item/detail`            | 获取单个 item  |
| GET  | `/api/v1/item/count`             | 获取 item 总数 |
| POST | `/api/v1/item/add`               | 添加新 item    |
| POST | `/api/v1/item/upload`             | multipart 上传新 item（web 端） |
| POST | `/api/v1/item/update`            | 更新 item      |
| POST | `/api/v1/item/batch_update`      | 批量更新(标签/分类并集、评分/阅读状态/文件夹设置) |
| POST | `/api/v1/item/delete`            | 移入回收站     |
| POST | `/api/v1/item/restore`           | 从回收站恢复   |
| GET  | `/api/v1/item/cover`             | 获取封面（自定义封面优先于自动提取） |
| PUT  | `/api/v1/item/cover`             | 设置自定义封面       |
| DELETE | `/api/v1/item/cover`           | 移除自定义封面（回退自动提取链） |
| GET  | `/api/v1/item/file`              | 获取原书文件   |
| POST | `/api/v1/item/open`              | 用系统默认应用打开原书文件（桌面形态） |
| POST | `/api/v1/item/show_in_folder`    | 在系统文件管理器中显示（选中文件） |
| GET  | `/api/v1/item/toc`               | 获取阅读目录（章节树） |
| GET  | `/api/v1/item/content`           | 获取归一化阅读内容（格式分流见下文） |
| GET  | `/api/v1/item/resource`          | 获取书内嵌资源（图片/字体/样式表） |
| POST | `/api/v1/item/refresh_metadata`  | 重新解析内嵌元数据并重建封面 |

### Item 对象

| 字段              | 类型     | 说明                                       |
| ----------------- | -------- | ------------------------------------------ |
| id                | string   | 内容哈希（BLAKE3 hex）                     |
| name              | string   | 文件名（不含扩展名），取主路径             |
| ext               | string   | 扩展名，小写，不含点（`epub` / `pdf` / `txt` / `md` / `mobi` / `azw3` / `docx` / `cbz`） |
| size              | number   | 文件大小（字节）                           |
| title             | string   | 书名（解析自文件元数据，解析失败回退文件名） |
| authors           | string[] | 作者列表（可多作者）                       |
| publisher         | string   | 出版社，可为空                             |
| pubdate           | string   | 出版日期（原文保留，如 `2024-01-01`），可为空 |
| isbn              | string   | ISBN，可为空                               |
| language          | string   | 语言代码（`zh` / `en` / `ja` …），可为空   |
| series            | string   | 系列名，可为空                             |
| series_index      | number   | 系列内序号（0 = 无序号）                   |
| description       | string   | 简介，可为空                               |
| url               | string   | 来源网址，可为空                           |
| tags              | string[] | 标签列表                                   |
| categories        | string[] | 分类列表（虚拟分类维度，扁平可多选）       |
| paths             | string[] | 所有文件位置（库内相对路径），首个为主路径 |
| folders           | string[] | 所在文件夹路径列表（由 paths 派生）        |
| star              | number   | 评分 0–5                                   |
| read_status       | string   | 阅读状态：`unread`（默认）/ `reading` / `finished` / `abandoned` |
| progress          | number   | 阅读进度百分比 0–100（1 位小数）           |
| progress_loc      | string   | 阅读位置锚点（opaque：epub CFI / pdf 页码等，由阅读器写回、原样返回），可为空 |
| last_read_time    | number   | 最近阅读时间（Unix 毫秒，0 = 从未阅读）；`progress` / `read_status` 变更时服务端自动刷新 |
| annotation        | string   | 备注                                       |
| added_time        | number   | 入库时间（Unix 毫秒）                      |
| modification_time | number   | 文件修改时间（Unix 毫秒）                  |
| custom_cover      | boolean  | 是否有自定义封面（`.sumi/covers/<hash>.webp` 存在与否，文件即真源；优先于自动提取封面） |

> **同内容去重**：内容相同的文件共享一个 item，`paths` 记录所有文件位置。
>
> **id 漂移与元数据迁移**：书籍文件内容被修改后，内容哈希变化会导致 id 变化（txt/md 是高频场景——外部编辑器保存即触发，与图片素材「入库后基本不变」不同）。元数据按内容哈希命名存储，但**记录全部文件位置（`paths`）**，索引流水线据此在内容变更时把旧 hash 的用户数据迁到新 hash，避免用户数据丢失。所有内容变更入口（文件监听事件、增量扫描、全量重建）执行同一套迁移规则（变更位置路径不变，旧 hash 记为 H、新 hash 记为 X）：
>
> - X 已有元数据（该内容早前已在库内其他位置入库）：不迁移，位置直接并入 X，以 X 既有元数据为准（去重语义优先）
> - X 无元数据且 H 已无剩余位置：H 的元数据整体**移动**给 X（TOML 文件 rename，自定义封面文件一并改名）
> - X 无元数据且 H 仍有其他位置（同内容多位置只变了一处）：H 的元数据**复制**给 X，H 保留原元数据
>
> 迁移覆盖全部用户数据：标签/分类/评分/备注/书目字段/阅读进度/自定义封面。事件按流水线常规推送（X 的 `item.added`、H 的 `item.removed` 或 `item.updated`），客户端无需特判；但路径匹配是启发式的，客户端不应假设 id 永久稳定。
>
> **解析字段与用户编辑**：`title` / `authors` / `publisher` / `pubdate` / `isbn` / `language` / `series` / `series_index` / `description` 由文件解析自动填充；经 `item/update` 显式设置过的字段被记录为用户编辑，`refresh_metadata` 重解析时不再覆盖（除非 `force`）。

### list

`POST /api/v1/item/list`

过滤条件为复杂结构，故使用 POST。所有过滤参数均为可选，组合逻辑为 AND。

#### 请求

| 参数       | 类型     | 说明                                                            |
| ---------- | -------- | --------------------------------------------------------------- |
| ids        | string[] | 按 id 列表匹配                                                  |
| keywords   | string[] | 关键词（匹配书名、作者、备注）                                  |
| content    | string   | 全文检索（txt/md/epub/docx 正文，FTS5 倒排；短语用双引号，空格为 AND）；不支持的格式不命中 |
| tags       | string[] | 按标签过滤（AND）                                                |
| categories | string[] | 按分类过滤（精确匹配），`categories_match`：`any`（默认）/ `all` |
| authors    | string[] | 按作者过滤（任一命中即匹配，OR）                                |
| series     | string   | 按系列过滤（精确匹配）                                          |
| publisher  | string   | 按出版社过滤（精确匹配）                                        |
| language   | string   | 按语言代码过滤                                                  |
| isbn       | string   | 按 ISBN 过滤                                                    |
| read_status | string  | 按阅读状态过滤：`unread` / `reading` / `finished` / `abandoned` |
| exclude_categories | string[] | 排除分类（任一命中即剔除）                    |
| exclude_tags | string[] | 排除标签（任一命中即剔除）                                |
| exclude_folders | string[] | 排除文件夹（含子目录，任一命中即剔除；空字符串条目被忽略）      |
| exclude_authors | string[] | 排除作者（任一命中即剔除）                                |
| star       | number   | 按评分过滤                                                      |
| folders    | string[] | 按文件夹路径过滤（含子目录）                                    |
| folders_exact | boolean | 为 true 时文件夹只精确匹配直接位于该目录下的 item（不含子目录）；空字符串表示库根目录，默认 false |
| without_categories | boolean | 只返回未分类（没有任何分类）的 item，默认 false      |
| without_tags | boolean | 只返回未标签（没有任何标签）的 item，默认 false      |
| without_authors | boolean | 只返回未标注作者（authors 为空）的 item，默认 false（元数据清理用） |
| ext        | string   | 按扩展名过滤                                                    |
| annotation | string   | 按备注文本过滤                                                  |
| url        | string   | 按来源网址过滤                                                  |
| in_trash   | boolean  | 是否只查回收站中的 item，默认 false                             |
| order_by   | string   | 排序字段：`added_time`（默认）/ `modification_time` / `title` / `author`（第一作者名）/ `size` / `star` / `progress` / `last_read_time` / `pubdate` |
| order      | string   | 排序方向：`desc`（默认）/ `asc`                                 |
| offset     | number   | 分页偏移，默认 0                                                |
| limit      | number   | 分页大小，默认 50                                               |

#### 响应

```json
{
  "status": "success",
  "data": {
    "items": [
      {
        "id": "9b1f2c...",
        "name": "三体",
        "ext": "epub",
        "size": 1245760,
        "title": "三体",
        "authors": ["刘慈欣"],
        "publisher": "重庆出版社",
        "pubdate": "2008-01-01",
        "isbn": "9787536692930",
        "language": "zh",
        "series": "地球往事",
        "series_index": 1,
        "description": "……",
        "url": "",
        "tags": ["科幻"],
        "categories": ["小说"],
        "paths": ["novels/三体.epub"],
        "folders": ["novels"],
        "star": 5,
        "read_status": "finished",
        "progress": 100,
        "progress_loc": "epubcfi(/6/24!/4/2/1:0)",
        "last_read_time": 1700000000000,
        "annotation": "",
        "added_time": 1690000000000,
        "modification_time": 1690000000000
      }
    ],
    "total": 1250,
    "total_size": 314572800,
    "offset": 0,
    "limit": 50
  }
}
```

`total` / `total_size` 为过滤后未分页的全量计数与字节数合计（前端检查器「分区状态」用）。

**排序稳定性**：主键同值（如相同 `added_time`）时按 `id` 字典序打破平局，保证相同查询的次序逐位确定——前端「骨架 + 分页窗口」模型依赖这一点对齐。

### skeleton

`POST /api/v1/item/skeleton`

请求参数与 `item/list` 完全相同（`offset` / `limit` 可省略，忽略），过滤、排序与 `item/list` **逐位一致**，但不分页，只返回布局所需的最低字段。供前端虚拟网格一次性建立完整布局（滚动条总高即时确定，可自由拖动跳转），视口内再按 offset 用 `item/list` 取详情。

#### 响应

```json
{
  "status": "success",
  "data": {
    "items": [
      { "id": "9b1f2c...", "path": "novels/三体.epub", "width": 600, "height": 900, "star": 5, "size": 1245760 }
    ],
    "total_size": 314572800
  }
}
```

`width` / `height` 为封面像素尺寸（未提取时为 0，客户端用默认书封宽高比占位）。条目数（`items` 长度）即全量计数；`star` 供网格 ★ 角标在未加载详情时显示；`size` 为该位置字节数，供选择集大小聚合（选择集可远超视口窗口，详情缓存承担不起全量聚合）。

### aggregate

`POST /api/v1/item/aggregate`

选择集共有特性聚合：传入 id 列表，返回标签/分类/作者的**交集**（多选编辑面板的数据源——「这些书共有的标签/分类/作者」）。

#### 请求

```json
{ "ids": ["9b1f2c...", "a1b2c3..."] }
```

#### 响应

```json
{
  "status": "success",
  "data": {
    "tags": ["科幻"],
    "categories": ["小说"],
    "authors": ["刘慈欣"],
    "missing_ids": []
  }
}
```

`missing_ids` 为请求中不存在的 id（其余照常聚合）；空选择集（或全部缺失）返回空交集。

### detail

`GET /api/v1/item/detail?id=<hash>`

返回单个 Item 对象；不存在时返回 `ITEM_NOT_FOUND`。

### count

`GET /api/v1/item/count`

返回库内 item 总数（不含回收站）。

#### 响应

```json
{ "status": "success", "data": 12500 }
```

### add

`POST /api/v1/item/add`

向书库添加新文件。`path`、`url`、`file_base64` 三者必须提供其一，作为文件内容来源；文件将写入 `folder_path` 指定的真实目录（缺省为库根目录），随后由索引流水线完成哈希、封面提取与元数据解析。`url` 仅作为下载来源；来源网页（书籍所在的页面地址）经 `website` 传入并记录为 Item.url。

**入库策略前置校验**（写盘前拒绝，避免落盘后又被入库判定剔除）：目标路径命中 `ignore` 规则返回 `INVALID_PARAM`；扩展名不在可见白名单（`.sumi/config.toml` 的 `extensions`）返回 `UNSUPPORTED_FORMAT`。`item/upload` 与 `item/update` 的改名/移动目标同样受此约束。

`path` 导入时保留原文件的创建时间与修改时间（`File.Copy` 默认会重置）：按 `modification_time` 排序与文件管理器观感均以原文件为准；`url`/`file_base64` 无原文件时间，取入库时刻。

#### 请求

| 参数        | 类型     | 必填   | 说明                                   |
| ----------- | -------- | ------ | -------------------------------------- |
| path        | string   | 三选一 | 本地文件路径，导入该文件               |
| url         | string   | 三选一 | 下载该 URL 的文件入库                  |
| file_base64 | string   | 三选一 | Base64 编码的文件数据                  |
| name        | string   | 否     | 文件名（不含扩展名），缺省取来源文件名 |
| folder_path | string   | 否     | 目标文件夹路径，缺省为库根目录         |
| title       | string   | 否     | 书名（导入即指定，记为用户编辑）       |
| authors     | string[] | 否     | 作者（导入即指定，记为用户编辑）       |
| tags        | string[] | 否     | 标签                                   |
| categories  | string[] | 否     | 分类（扁平名称校验；拖拽收集用）       |
| annotation  | string   | 否     | 备注                                   |
| website     | string   | 否     | 来源网页，记录为 Item.url              |
| skip_existing | boolean | 否     | 内容已存在于库内（不含回收站）时跳过：不写文件、不追加路径，响应 `skipped: true`（默认 false，即始终写入） |

#### 响应

Item 对象，并附带 `already_existed` / `skipped` 标志：

- 内容不存在：写入文件并索引，返回新 item，`already_existed: false`
- 内容已存在（同哈希）且未 skip：仍将文件复制到 `folder_path` 目标位置，已有 item 的 `paths` 追加新路径，返回该 item，`already_existed: true`——客户端可据此提示「内容已存在，已关联到现有条目」
- 内容已在库内且 `skip_existing: true`：不写入，`skipped: true`（前端导入的重复策略弹窗用）；内容仅存在于回收站时不视为重复，正常导入（删掉的内容应可重新导入）

```json
{
  "status": "success",
  "data": {
    "item": {
      "id": "9b1f2c...",
      "paths": ["novels/三体.epub", "backup/三体.epub"]
    },
    "already_existed": true
  }
}
```

### upload

`POST /api/v1/item/upload`

multipart/form-data 上传新 item（web 端用）：浏览器无本地文件路径可引用（拖拽/文件选择器拿到的是 File 内容），经本端点以内容入库。写权限：admin 恒可用；viewer 需 `[web].writable`（否则 403 `READ_ONLY`）。

#### 请求（multipart/form-data）

| 字段        | 类型   | 必填 | 说明                                   |
| ----------- | ------ | ---- | -------------------------------------- |
| file        | binary | 是   | 文件内容；文件名只取末段（防跨目录），扩展名决定入库类型（与 path 导入同语义，不校验内容） |
| folder_path | text   | 否   | 目标文件夹路径，缺省为库根目录         |
| name        | text   | 否   | 文件名（不含扩展名），缺省取 file 文件名 |
| skip_existing | text | 否   | 传 `"true"` 时内容已在库内则跳过（同 item/add） |

#### 响应

与 `item/add` 相同（Item + `already_existed`）。请求体上限 512MB。

### update

`POST /api/v1/item/update`

更新元数据（写入 `.sumi/metadata/`）。`name`、`folder_path` 会同步操作真实文件。

#### 请求

| 参数        | 类型     | 必填 | 说明                                               |
| ----------- | -------- | ---- | -------------------------------------------------- |
| id          | string   | 是   | item id                                            |
| path        | string   | 否   | 指定操作的文件位置（同内容多路径时），缺省为主路径 |
| name        | string   | 否   | 重命名文件（同步修改真实文件名）                   |
| folder_path | string   | 否   | 移动到新文件夹（移动真实文件）                     |
| title       | string   | 否   | 书名（显式设置后记为用户编辑）                     |
| authors     | string[] | 否   | 作者（整体替换，记为用户编辑）                     |
| publisher   | string   | 否   | 出版社                                             |
| pubdate     | string   | 否   | 出版日期                                           |
| isbn        | string   | 否   | ISBN                                               |
| language    | string   | 否   | 语言代码                                           |
| series      | string   | 否   | 系列名（空串清除系列与 series_index）              |
| series_index | number  | 否   | 系列内序号                                         |
| description | string   | 否   | 简介                                               |
| tags        | string[] | 否   | 标签（整体替换）                                   |
| categories  | string[] | 否   | 分类（整体替换，自动登记注册表）                   |
| star        | number   | 否   | 评分 0–5                                           |
| read_status | string   | 否   | 阅读状态：`unread` / `reading` / `finished` / `abandoned` |
| progress    | number   | 否   | 阅读进度 0–100                                     |
| progress_loc | string  | 否   | 阅读位置锚点（随 progress 写回）                   |
| annotation  | string   | 否   | 备注                                               |
| url         | string   | 否   | 来源网址                                           |

字段间无自动联动：`progress` 与 `read_status` 相互独立（客户端自行决定「读完 100% 即 finished」类逻辑）；唯一例外是 `progress` / `progress_loc` / `read_status` 任一变更时，服务端自动将 `last_read_time` 刷新为当前时刻。

#### 响应

更新后的 Item 对象。

### batch_update

`POST /api/v1/item/batch_update`

对一批 item 应用同一组更新,一次请求完成(多选打标签/评分/标记已读/移动等批量场景,避免逐条 `item/update` 的 N 次往返)。所有 id 串行经索引流水线应用,响应一次性返回。

语义与 `item/update` 的差异:

- `add_tags` / `add_categories` 是**并集追加**(保留已有),不是整体替换
- `remove_tags` / `remove_categories` 是**并集移除**（多选面板的共有特性摘除）
- `star` / `read_status` / `folder_path` 是**设置**(与 `item/update` 同语义)

#### 请求

| 参数          | 类型     | 必填 | 说明                                             |
| ------------- | -------- | ---- | ------------------------------------------------ |
| ids           | string[] | 是   | item id 列表(重复 id 自动去重)                 |
| add_tags      | string[] | 否   | 追加标签(并集,自动登记注册表)                  |
| add_categories | string[] | 否  | 追加分类(并集;名称校验同 `item/update`,自动登记注册表) |
| star          | number   | 否   | 评分 0–5(设置)                                 |
| read_status   | string   | 否   | 阅读状态(设置;批量「标为已读/未读」)           |
| remove_tags   | string[] | 否   | 移除标签(并集)                                |
| remove_categories | string[] | 否 | 移除分类(并集)                                |
| folder_path   | string   | 否   | 移动到该文件夹(移动各 item 的主位置;空字符串为库根目录) |

七个更新字段至少提供一个,否则返回 `INVALID_PARAM`。

#### 部分失败语义

批量操作不整体失败,逐项跳过:

- **内容不存在**的 id:跳过该项(元数据与移动都不应用),记入 `missing_ids`
- **移动冲突**(目标位置已有同名文件):跳过该项的移动,记入 `missing_ids`;`add_tags` / `add_categories` / `star` / `read_status` 照常应用
- **回收站中的 item**(无库内位置):移动不适用,跳过;元数据照常应用
- **空操作**（如追加的标签已存在，元数据无变化）：跳过落盘与事件，不计入 `updated`

#### 响应

```json
{
  "status": "success",
  "data": { "updated": 498, "missing_ids": ["abc...", "def..."] }
}
```

`updated` 为实际变更元数据的 id 数（空操作不计）;`missing_ids` 为上述未达成的 id(去重)。客户端可据此提示「已更新 n 项,m 项未处理」。

事件推送合并为分块的 `items.updated`（每块最多 1000 个 item），不再逐条发 `item.updated`——大批量（数万项）时逐条事件会形成风暴打挂客户端。

### delete

`POST /api/v1/item/delete`

移入回收站：文件移入 `.sumi/trash/`（保留目录结构），元数据保留。不带 `path` 为条目级删除：回收该 item 的**全部**库内位置（同内容多路径 item 只回收一个位置会使条目残留在网格）；带 `path` 为单位置删除。

#### 请求

| 参数 | 类型   | 必填 | 说明                                         |
| ---- | ------ | ---- | -------------------------------------------- |
| id   | string | 是   | item id                                      |
| path | string | 否   | 指定文件位置（同内容多路径时），缺省为主路径 |

#### 响应

```json
{ "status": "success" }
```

### restore

`POST /api/v1/item/restore`

从回收站恢复：文件移回元数据 `paths` 记录的原路径。不带 `path` 恢复全部回收站位置（与 delete 对称）；原路径已被占用的位置跳过留在回收站，全部冲突才返回 `FILE_EXISTS`。

#### 请求

| 参数 | 类型   | 必填 | 说明                                         |
| ---- | ------ | ---- | -------------------------------------------- |
| id   | string | 是   | item id                                      |
| path | string | 否   | 指定文件位置（同内容多路径时），缺省为主路径 |

#### 响应

```json
{ "status": "success" }
```

### cover

`GET /api/v1/item/cover?id=<hash>`

封面分两条来源链：**自定义封面**（用户数据，`.sumi/covers/<hash>.webp`，参与同步，见下文 PUT/DELETE）与**自动提取封面**（派生缓存，库外缓存目录，可重建）。**自定义优先**。

自动提取封面为缓存（**扫描导入即时提取**：首扫并行阶段单次解包同出封面+书目元数据，item 入库即可完整显示；增量/对账不生成，由读取端兜底），GET 响应分四种：

1. **自定义封面命中**（`.sumi/covers/<hash>.webp` 存在）→ 封面二进制（`image/webp`）
2. **派生缓存命中** → 封面二进制（`image/webp`）
3. **未命中且可即时生成**（txt/md/docx：排版生成书名+作者的文字封面，开销极小）→ 直接生成并返回 webp，同时入派生缓存
4. **未命中且需重计算**（epub/mobi/azw3 解包提取、pdf/cbz 首页渲染）→ 404（后台生成中，生成完成后经 `item.updated` 事件重建，前端已有占位重试闭环）

封面统一等比缩入 1024 边长内（不放大）。响应带 `Cache-Control: immutable`——注意自定义封面变更后同一 id 的封面内容已变，客户端须以 `item.updated` 事件为信号重建 `<img>`（唯一不 immutable 的缺口：id 不变而字节变）。

### set_cover

`PUT /api/v1/item/cover`

设置自定义封面（替换自动提取封面）。服务端解码校验、等比缩入 1024、转 webp 原子写入 `.sumi/covers/<hash>.webp`——**文件存在即自定义**（无元数据字段双写，`Item.custom_cover` 由文件存在性派生）。已有自定义封面时直接覆盖替换。完成后广播 `item.updated`（各端重建封面）。自定义封面随书库参与网盘同步；id 漂移触发元数据迁移时一并跟随（重命名文件）。

写端点：viewer 需 `[web].writable`（否则 403 `READ_ONLY`）。

#### 请求

| 参数       | 类型   | 必填 | 说明                     |
| ---------- | ------ | ---- | ------------------------ |
| id         | string | 是   | item id                  |
| img_base64 | string | 是   | 封面图像的 Base64 编码   |

#### 响应

更新后的 Item 对象（`custom_cover: true`）。内容不可解码为图像时返回 `UNSUPPORTED_FORMAT`。

### delete_cover

`DELETE /api/v1/item/cover?id=<hash>`

移除自定义封面，回退到自动提取链。幂等：无自定义封面时为 no-op。完成后广播 `item.updated`。

#### 响应

```json
{ "status": "success" }
```

### file

`GET /api/v1/item/file?id=<hash>`

返回原书文件二进制，Content-Type 按扩展名推断（`epub` → `application/epub+zip`，`pdf` → `application/pdf`，`txt` → `text/plain`，`md` → `text/markdown`，`docx` → `application/vnd.openxmlformats-officedocument.wordprocessingml.document`，`mobi`/`azw3` → `application/x-mobipocket-ebook`，无法识别时为 `application/octet-stream`；txt/md 为原始字节、编码以文件为准，编码归一走 `item/content`）。取原书字节（预览/导出/复制）走此端点；「用系统应用打开」与「在文件管理器中显示」不走字节传输，见 `item/open` / `item/show_in_folder`。文件位置取 item 主位置（优先非回收站位置）；文件已缺失时返回 404。与封面同理带 `Cache-Control: immutable`。直链无法设置请求头，与 cover 一样放行查询参数 `?token=`。

支持 HTTP Range 请求——pdf 阅读器（流式取页）与大文件（CBZ）边下边开依赖分段读取。

### open

`POST /api/v1/item/open`

用操作系统默认应用打开原书文件（macOS `open` / Windows `ShellExecute` / Linux `xdg-open`）。桌面端「双击/回车打开」行为由此端点承载——v1 无内置阅读器，点击查看体验即「调起系统软件」。

**admin 限定**：viewer（含可写）返回 403 `READ_ONLY`——该端点操作的是 daemon 所在机器的 GUI 会话，局域网 web 客户端调用无意义且危险。无头服务器部署形态下系统调用自然失败，返回 `OPEN_FAILED`。

#### 请求

| 参数 | 类型   | 必填 | 说明                                         |
| ---- | ------ | ---- | -------------------------------------------- |
| id   | string | 是   | item id                                      |
| path | string | 否   | 指定文件位置（同内容多路径时），缺省为主路径（优先非回收站位置） |

#### 响应

```json
{ "status": "success" }
```

系统调用失败（未安装可打开该格式的默认应用等）返回 `OPEN_FAILED`；item 不存在返回 `ITEM_NOT_FOUND`；文件在磁盘上缺失返回 404。

### show_in_folder

`POST /api/v1/item/show_in_folder`

在系统文件管理器中显示并选中文件（macOS Finder / Windows Explorer）。Linux 无跨发行版的「选中文件」接口，退化为打开所在目录。权限与错误语义同 `item/open`。

#### 请求

同 `item/open`。

#### 响应

```json
{ "status": "success" }
```

### toc

`GET /api/v1/item/toc?id=<hash>`

阅读目录（章节树），供阅读器导航。来源：epub 的 NCX/nav；mobi/azw3 内部 TOC；docx 的标题样式大纲；md 按 `#` 标题层级；txt 按常见章节模式（`第N章` / `Chapter N` 等）启发式提取，识别失败返回单节点；pdf 取书签大纲（无书签返回空数组）；cbz 无目录概念，返回空数组。

#### 响应

```json
{
  "status": "success",
  "data": [
    { "title": "第一部", "href": "", "children": [
      { "title": "第一章 科学边界", "href": "anchor:sec-1", "children": [] }
    ]}
  ]
}
```

`href` 为 opaque 定位符，按前缀解释：`anchor:<id>`（`item/content` 归一化文档内的锚点，转换时由服务端在章节起始元素注入）/ `page:<n>`（pdf 页码，配合 `item/file` 的阅读器跳转）。层级节点无自身内容时 `href` 为空串。

### content

`GET /api/v1/item/content?id=<hash>`

归一化阅读内容。**按需转换**：导入时不做预处理，首次请求同步转换（秒级，阅读是用户主动动作，客户端宜显示加载态），结果写入库外派生缓存，命中后带 `Cache-Control: immutable`（id 为内容哈希）。

按格式分流：

- `epub` / `mobi` / `azw3` / `docx` → `200 text/html; charset=utf-8`：服务端转换的单个 XHTML 阅读流（章节顺序拼接，章节起始注入锚点 id，与 `item/toc` 的 `anchor:` href 对应）；内嵌图片/字体引用改写为 `item/resource?id=<hash>&path=<内部路径>` 直链。排版样式（字体/字号/主题）由客户端在阅读容器内注入，转换输出只保留语义结构
- `txt` / `md` → `200 text/plain; charset=utf-8`：原文字节，**编码归一**——探测 GBK / Big5 / Shift_JIS / UTF-16 等并统一转 UTF-8（中文网文 txt 的刚需）；md 的渲染由客户端负责
- `pdf` / `cbz` → `UNSUPPORTED_FORMAT`：阅读走 `item/file`（pdf.js / 图片阅读器），本端点不适用

### resource

`GET /api/v1/item/resource?id=<hash>&path=<内部路径>`

归一化 HTML 引用的书内嵌资源（图片/字体/样式表）：从原书容器（epub/docx 为 zip，mobi/azw3 为 PDB 记录）按需解包返回，Content-Type 按内部路径扩展名推断。仅 `epub` / `mobi` / `azw3` / `docx` 有意义；`path` 不存在返回 404（单张图片缺失不应阻断阅读，客户端用占位图）。`Cache-Control: immutable`。

### refresh_metadata

`POST /api/v1/item/refresh_metadata`

重新解析文件内嵌元数据并重建封面（Calibre「重新下载元数据」的手动等价物）：对指定 item 强制重新解包解析 + 重建封面（不走「已存在跳过」）。派发后台任务异步执行，立即返回；完成后经 `item.updated` 事件通知前端。

**用户编辑保护**：经 `item/update` / `item/add` 显式设置过的字段不被重解析覆盖（服务端记录用户编辑的字段集合）；`force: true` 时全量重解析、清除用户编辑标记。自动提取封面不受保护——始终重建（文件内嵌封面即文件的属性）；**自定义封面是用户数据，不受影响且始终优先**。

#### 请求

| 参数  | 类型    | 必填 | 说明    |
| ----- | ------- | ---- | ------- |
| id    | string  | 是   | item id |
| force | boolean | 否   | 忽略用户编辑保护全量重解析，默认 false |

#### 响应

```json
{ "status": "success" }
```

## category

分类是虚拟分类维度：**扁平名字**（无层级），一个 item 可同时挂多个分类（书单/书架用法）。注册表（`.sumi/categories.toml`）支持空分类预创建；item 赋值时自动登记。

| 方法 | 端点 | 说明 |
| ---- | ---- | ---- |
| GET  | `/api/v1/category/list` | `[{ name, count }]`（注册表 ∪ 全部 item 赋值并集），count 为库内（不含回收站）item 数 |
| POST | `/api/v1/category/create` | `{ "name": "想读" }`；已存在返回 `CATEGORY_EXISTS` |
| POST | `/api/v1/category/update` | `{ "name", "new_name" }` 重命名；目标已存在时合并 |
| POST | `/api/v1/category/delete` | `{ "name" }`，全部 item 的相关赋值清除 |

`category/list` 响应与 `tag/list` 同构（`{ name, count }` 数组）。

## tag

标签注册表（`.sumi/tags.toml`）支持空标签预创建；item 赋值时自动登记。

| 方法 | 端点 | 说明 |
| ---- | ---- | ---- |
| GET  | `/api/v1/tag/list` | `[{ "name", "count" }]`，count 为库内（不含回收站）item 数 |
| POST | `/api/v1/tag/create` | `{ "name" }` |
| POST | `/api/v1/tag/update` | `{ "name", "new_name" }`，重命名，全部 item 跟随；目标已存在时合并 |
| POST | `/api/v1/tag/delete` | `{ "name" }`，全部 item 的该标签清除 |

## author

作者是**派生维度**：来自全部 item 的 `authors` 字段聚合，无注册表、不可预创建（空作者没有意义）；`item/update` 改 `authors` 后聚合自然收敛。重命名/合并经本组端点批量应用到全部 item（等价于对所有命中 item 做 `authors` 数组改写，过程中照常推送 `items.updated` 事件）。

| 方法 | 端点 | 说明 |
| ---- | ---- | ---- |
| GET  | `/api/v1/author/list` | `[{ "name", "count" }]`，count 为库内（不含回收站）包含该作者的 item 数（多作者书在各作者下各计一次），按名称排序 |
| POST | `/api/v1/author/update` | `{ "name", "new_name" }` 重命名，全部 item 跟随；目标作者已存在时**合并**（item 内 authors 数组去重）。源不存在返回 `AUTHOR_NOT_FOUND` |
| POST | `/api/v1/author/delete` | `{ "name" }`，全部 item 的 authors 数组中移除该作者（数组清空后 item 变为未标注作者，可用 `without_authors` 筛出重新整理） |

## series

系列同为**派生维度**（来自 `series` 字段聚合），语义与 author 组一致。

| 方法 | 端点 | 说明 |
| ---- | ---- | ---- |
| GET  | `/api/v1/series/list` | `[{ "name", "count" }]`，count 为库内（不含回收站）该系列的 item 数 |
| POST | `/api/v1/series/update` | `{ "name", "new_name" }` 重命名；目标已存在时合并。源不存在返回 `SERIES_NOT_FOUND` |
| POST | `/api/v1/series/delete` | `{ "name" }`，全部 item 的 `series` 与 `series_index` 一并清除 |

## view

视图偏好（`.sumi/view.toml`，参与同步）：记住文件夹/分类/标签/作者/系列视图各自的排序方式。
条目为扁平 map，scope 键五种形态：

- `folder:<库内路径>`（路径 `""` 为库根）——**继承由客户端解析**：沿父链向上查找，子文件夹自己的设置优先于父级
- `category:<名称>` / `tag:<名称>` / `author:<名称>` / `series:<名称>`——无层级，无条目时回落全局默认（入库时间↓）

服务端只存取原始条目，不理解继承语义。文件夹移动/重命名时 `folder:` 键自动跟随，删除时自动清除；分类/标签/作者/系列重命名时对应键跟随（合并场景保留目标键设置）。排序值与 `item/list` 的 `order_by`/`order` 同白名单。

| 方法 | 端点 | 说明 |
| ---- | ---- | ---- |
| GET | `/api/v1/view/preferences` | 全部条目：`{ "folder:novels": { "order_by", "order" }, ... }` |
| PUT | `/api/v1/view/preference` | `{ "scope", "order_by", "order" }`，覆盖写；非法 scope/排序值返回 `INVALID_PARAM` |
| DELETE | `/api/v1/view/preference?scope=<scope>` | 删除条目，回到继承/默认 |

## global_filter

全局列表隐藏项（`.sumi/global_filter.toml`，参与同步）：被标记的文件夹/分类/标签/作者，其下书籍由客户端在「全部书籍/根目录/未分类/未标签」等全局视图查询时附带 `exclude_*` 参数排除（OR 语义：命中任一隐藏维度即剔除）；维度自身视图与回收站不排除。文件夹条目为库内相对路径，子树整体隐藏。

级联跟随与排序偏好同款：文件夹移动/重命名（含移入回收站，恢复时回归）自动迁移，删除/清空回收站自动清除；分类/标签/作者重命名跟随（目标已隐藏时合并）、删除清除。变更广播 `global_filter.changed`（含外部同步写入的重载），负载为完整快照。

| 方法 | 端点 | 说明 |
| ---- | ---- | ---- |
| GET | `/api/v1/global_filter/list` | 全部隐藏项：`{ "folders": [...], "categories": [...], "tags": [...], "authors": [...] }` |
| PUT | `/api/v1/global_filter` | `{ "kind": "folder" \| "category" \| "tag" \| "author", "name", "hidden" }`，幂等；路径/名称非法返回 `INVALID_PARAM` |

## lock

文件夹/分类/标签/作者锁（`.sumi/locks.toml`，参与同步）：被锁维度的内容需要密码解锁后才可见，由**服务端强制**（与 global_filter 的客户端约定式排除不同）：

- 主动筛选（`item/list`、`item/skeleton` 的 `folders`/`categories`/`tags`/`authors` 参数）命中未解锁的锁 → `403 LOCKED`
- 全局视图：未解锁锁覆盖的条目由服务端直接排除（同查询同排序，skeleton 与 list 一致）
- 内容直连（`item/cover`、`item/file`、`item/open`、`item/show_in_folder`、`item/toc`、`item/content`、`item/resource`、`item/detail`、`item/aggregate`）命中未解锁的锁 → `403 LOCKED`（防止拿到 hash 直接拼 URL 或借系统应用绕过）

判定语义（OR 剔除）：条目命中任一未解锁的锁即不可见；同内容多路径时，存在任一「无锁或已解锁」位置即位置维度放行，分类/标签/作者锁与位置无关（item 属性）。锁定文件夹移入回收站后保持锁定（条目随路径迁移）。**锁不是加密**：文件系统层面无保护，仅防止「通过 sumi 查看」；密码哈希（Argon2id）参与同步，拿到 `.sumi/` 的人可离线爆破（慢哈希缓解）。

**解锁票据**：`lock/unlock` 校验密码后发放随机票据（daemon 内存态，重启失效），由各客户端独立持有——解锁状态不随分享的链接扩散。后续请求经 `X-Sumi-Unlock: <t1>,<t2>` 请求头附带；`<img>` 直链与 SSE 无法设头，改用查询参数 `?unlock=<t1>,<t2>`（路径集合与 token 查询参数通道一致）。**不要把带 unlock 参数的封面 URL 分享给他人**（等同分享该内容的解锁能力）。前端丢弃票据即「锁定回去」。

级联跟随与 global_filter 同款：文件夹移动/重命名（含移入回收站，恢复时回归）自动迁移，删除/清空回收站自动清除；分类/标签/作者重命名跟随（目标已锁时合并，保留目标密码）、删除清除。变更广播 `locks.changed`（含外部同步写入的重载）。验证失败全局节流：连续 5 次失败冷却 60s（`429 THROTTLED`）。

| 方法 | 端点 | 说明 |
| ---- | ---- | ---- |
| GET | `/api/v1/lock/list` | 全部锁（仅名称，不含密码哈希）：`{ "folders": [...], "categories": [...], "tags": [...], "authors": [...] }` |
| POST | `/api/v1/lock/set` | `{ "dimension": "folder" \| "category" \| "tag" \| "author", "name", "password", "old_password"? }`；已锁条目改密必须带正确旧密码（否则 `403 OLD_PASSWORD_REQUIRED`）。admin 限定 |
| POST | `/api/v1/lock/remove` | `{ "dimension", "name", "password" }`，解除锁需密码。admin 限定 |
| POST | `/api/v1/lock/unlock` | `{ "dimension", "name", "password" }` → `{ "unlock_token": "..." }`；任何有效 token（含只读 viewer）可解锁；锁不存在 `404 LOCK_NOT_FOUND`、密码错误 `401` |

## trash

回收站内容通过 `item/list`（`in_trash: true`）查询。

| 方法 | 端点                  | 说明       |
| ---- | --------------------- | ---------- |
| POST | `/api/v1/trash/clear` | 清空回收站 |

### clear

`POST /api/v1/trash/clear`

彻底删除回收站中的全部文件，并清理对应的元数据、派生封面缓存与自定义封面。不可恢复。

#### 响应

```json
{ "status": "success" }
```

## events

`GET /api/v1/events?token=<token>`

Server-Sent Events 订阅书库变更,前端据此增量刷新界面。`EventSource` 无法设置请求头,token 通过查询参数传递。

### 事件一览

| 事件              | data | 说明 |
| ----------------- | ---- | ---- |
| `item.added`      | Item 对象 | 新文件入库（单条路径：监听/API 增量） |
| `items.added`     | `{ "ids": ["..."] }` | 扫描导入的批量合并事件（300ms 窗口/2000 条上限合成一条）；客户端按「有新增」信号重载列表即可 |
| `item.updated`    | Item 对象 | 元数据、文件位置或封面变更(封面生成完成也补发一次,前端据此重建 404 占位) |
| `items.updated`   | `{ "items": [Item...] }` | `item.updated` 的批量变体（封面批量回写、`batch_update`、分类/标签/作者/系列重命名与删除的级联迁移），攒满 1000 条发一帧，客户端逐个就地替换缓存 |
| `item.trashed`    | `{ "id": "..." }` | 最后一个库内位置移入回收站 |
| `item.restored`   | Item 对象 | 首个回收站位置回归库内 |
| `item.removed`    | `{ "id": "..." }` | 彻底删除(无剩余位置) |
| `folder.changed`  | `{ "reason": "external" }` | 目录结构可能变化,客户端应重拉 `folder/list`;reason 恒为 `external`,客户端必须忽略取值(结构为将来预留) |
| `library.updated` | LibraryInfo 对象 | 库显示名变更（`PATCH library/info`）；负载为完整库信息，客户端就地替换 |
| `global_filter.changed` | `{ "folders": [...], "categories": [...], "tags": [...], "authors": [...] }` | 全局列表隐藏集变更（标记/取消、级联跟随、外部同步重载）；负载为完整快照，客户端就地替换并重查列表 |
| `locks.changed` | `{ "folders": [...], "categories": [...], "tags": [...], "authors": [...] }` | 锁集变更（设锁/解除/改密、级联跟随、外部同步重载）；负载为名称快照，客户端重拉锁标记并重查列表。锁覆盖内的书籍不发 `item.*` 事件（保守过滤，已解锁客户端可能漏收，靠解锁后的重查兜底） |
| `task.progress`   | `{ "task": "cover", "pending": 236, "active": 4 }` | 后台任务积压变化(封面/元数据/全文索引队列与索引管道;服务端 500ms 节流,积压倒零后补发一帧清零帧) |

事件名与负载即持久契约(实现方必须逐字兼容);后端以常量集中定义,客户端不许凭代码反推。

### 负载契约

**Item 对象**:与 `item/list` 响应中的 Item 结构完全相同(见「Item 对象」节)。`item.updated` / `item.added` / `item.restored` / `items.updated` 带完整对象,客户端可就地替换缓存;`trashView` 由服务端按「是否只剩回收站位置」投影(回收站视图的 `paths` 为原库内路径)。

**id 负载**(`item.trashed` / `item.removed`):

```json
{ "id": "9b1f2c..." }
```

**folder.changed 负载**:

```json
{ "reason": "external" }
```

触发来源:本端文件夹增删改移(API)、外部进程目录操作(文件监听)、周期对账扫描兜底。文件夹树无增量语义(前端经 `folder/list` 拉全量树;服务端按目录结构缓存建树、计数实时合并,缓存失效与本事件同线),事件只表达「需要重拉」。

**task.progress 负载**:

```json
{ "task": "cover", "pending": 236, "active": 4 }
```

`task` 为 `cover`（封面/元数据解析/全文索引队列）或 `index`（索引管道：入库排队 job 与写入防抖路径）。`pending` 为排队数，`active` 为执行中（封面生成并发度 `CPU/4`、封顶 8；索引任务扫描中为 1、否则为 0）。`task=index` 且扫描进行中时额外携带阶段进度：

```json
{ "task": "index", "pending": 12, "active": 1, "phase": "hash", "processed": 1800, "total": 4200 }
```

`phase` 为 `scan`（遍历，`total=0` 表示总量未知，客户端宜显示不定态）/ `hash` / `apply`；非扫描期间三个字段省略。SSE 断开的客户端可轮询 `GET /api/v1/app/status` 获取同一快照。

### 时序与可靠性语义

- **节流**:`task.progress` 服务端 500ms 最多一帧;`item.*` 事件无节流;批量元数据操作（`item/batch_update`）合并为分块的 `items.updated` 帧
- **不保证送达**:订阅者消费跟不上(积压 1024 条)时服务端直接断开该订阅——客户端重连后必须以 `item/skeleton` + `folder/list` 全量对齐,不得假设收到过全部事件
- **初始索引期间**:就绪网关拦截期内不推事件(订阅端点同样 503),主界面加载完成后订阅即可
- **顺序**:同一 item 的事件按流水线处理顺序发出;不同 item 之间无全局顺序保证
- **重连**:EventSource 断线自动重连,`onopen` 再次触发即对齐时机
