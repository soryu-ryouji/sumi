# 存储设计

## 目录结构

sumi 不会在书籍文件和文件夹中存放任何文件，所有数据收敛在书库目录下的 `.sumi/` 隐藏文件夹中：

```text
你的书库目录/
├── 小说/
│   ├── 科幻/
│   └── 推理/
├── 技术/
└── .sumi/              ← sumi 只在目录下创建这个隐藏文件夹
    ├── config.toml     ← 项目级配置（参与同步）
    ├── categories.toml ← 分类注册表（参与同步）
    ├── tags.toml       ← 标签注册表（参与同步）
    ├── view.toml       ← 视图偏好：文件夹/分类/标签/作者/系列的排序记忆（参与同步）
    ├── global_filter.toml ← 全局列表隐藏项：文件夹/分类/标签/作者（参与同步）
    ├── locks.toml      ← 文件夹/分类/标签/作者锁：密码哈希（参与同步）
    ├── covers/         ← 自定义封面 <hash>.webp（用户数据，参与同步，见「封面存储」）
    ├── metadata/       ← 书目参数，纯文本 TOML（配置文件模式，参与同步）
    ├── metadata.db     ← 书目参数，SQLite（数据库模式，本地专用，不参与同步）
    ├── storage_mode    ← 存储方案标记文件（迁移的最后一步写入；探测时优先于文件存在性）
    ├── .gitignore      ← 自动生成，排除 trash/
    └── trash/          ← 回收站（本地专用，不参与同步）
```

**组织维度与存储维度分离**：书籍以真实文件夹组织（非侵入，文件夹是用户可见的分类主轴）；但 `.sumi/` 内部**不按文件夹分目录**——元数据与自定义封面按内容哈希扁平命名。原因：

- 同一内容多位置共享一份元数据（同内容去重，`paths` 记录全部位置）
- 书籍移动/重命名不改变内容，元数据关联天然不丢
- 文件夹只是「位置」，位置的持久数据（视图排序、隐藏、锁）以路径为键存放在各自注册表中，随文件夹移动/重命名级联迁移（见 API 文档 view / global_filter / lock 章节）

派生缓存（自动提取封面、全文索引、归一化阅读内容、元数据缓存）不进 `.sumi/`，放在库外系统缓存目录，避免库位于 iCloud/Dropbox 等同步盘时 `.sumi/` 膨胀拖累同步：

```text
%LOCALAPPDATA%/sumi/cache/<库标识>/             ← Windows（iCloud 不同步）
~/.local/share/sumi/cache/<库标识>/             ← Linux
~/Library/Application Support/sumi/cache/<库标识>/   ← macOS
```

`<库标识>` = `<库文件夹名>_<库根路径SHA-256前16位>`——同名库靠哈希区分，用户可直观识别缓存归属。

缓存目录内布局：

```text
cache/<库标识>/
├── index.db     ← 元数据 SQLite 派生缓存 + FTS5 全文索引（txt/md/epub/docx 正文；可删，重启自动重建）
├── covers/      ← 自动提取封面缓存 <hash>.webp（统一缩入 1024；自定义封面不在此，见「封面存储」）
└── content/     ← 归一化阅读内容缓存 <hash>.html（item/content 按需转换产物，含锚点注入）
```

缓存可整体删除，重启扫描自动重建。

## 同步边界

| 内容          | 是否参与同步 | 说明                       |
| ------------- | ------------ | -------------------------- |
| `config.toml`     | 是           | 项目配置，随书库目录一起走 |
| `categories.toml` | 是           | 分类注册表（含空分类）     |
| `tags.toml`       | 是           | 标签注册表（含空标签）     |
| `view.toml`       | 是           | 视图偏好（排序记忆），扁平 map |
| `global_filter.toml` | 是        | 全局列表隐藏项（文件夹/分类/标签/作者） |
| `locks.toml`      | 是           | 锁（Argon2id 密码哈希；服务端强制，见 server-rest-api-v1.md 的 lock 节） |
| `covers/`         | 是           | 自定义封面（用户数据，不可重建） |
| `metadata/`       | 是（配置文件模式） | 书目参数，该模式的唯一权威数据源 |
| `metadata.db`     | 否（数据库模式）   | 书目参数，该模式的唯一权威数据源；**不要把数据库模式的库放进同步盘**（SQLite 文件跨机同步会腐蚀） |
| `storage_mode`    | 是           | 存储方案标记（`database`/`toml`），迁移的原子收尾 |
| `.gitignore`       | 是           | 自动生成，排除 `trash/`                    |
| `trash/`          | 否           | 回收站，仅本机可恢复       |
| 库外派生缓存      | 否           | 自动封面/全文索引/归一化内容/元数据镜像，全部可重建，不进同步盘 |

`.sumi/.gitignore` 由 sumi 自动生成，排除 `trash/`。

注意：部分网盘客户端（OneDrive、Syncthing 等）不识别 `.gitignore`，需要用户在网盘客户端中手动配置排除规则。后续应在用户文档中说明。

## 元数据存储方案（数据库 / 配置文件）

书目参数（标题、作者、标签、评分、阅读进度等）的权威存储有两种模式，打开库时自动判定：

- **数据库（默认，新库）**：`.sumi/metadata.db`，单文件 SQLite。写入同步落盘、批量操作单事务。适合本地使用的库。**不要放进 iCloud/Dropbox 等同步盘**——需要同步时切到配置文件模式。
- **配置文件**：`.sumi/metadata/<hash>.toml`，每本书一个纯文本小文件。网盘同步友好（冲突粒度为单本书）。

**模式探测**：`.sumi/storage_mode` 标记文件优先（迁移的最后一步写入）；无标记按内容探测——`metadata.db` 存在且非空 → 数据库；有 `metadata/*.toml` → 配置文件；都没有 → 新库默认数据库。两种文件皆存（迁移中断窗口）时空 db 视为残留回退配置文件。

**切换**（`POST /api/v1/library/storage_mode`）：单写者内全量互转（写新权威层 → 删旧文件 → 写标记），随后重启生效。旧文件被进程占用删不掉时（Windows）由下次启动清理兜底。

两模式共享同一份内存索引与查询路径（读路径无差异）；元数据对账（外部 TOML 变更并入）只在配置文件模式启用——数据库模式没有外部可写源。

## 纯文本元数据（配置文件模式）

书目参数集中存放在 `.sumi/metadata/` 中，按内容哈希命名，每本书对应一个独立的纯文本文件（TOML）：

```toml
# .sumi/metadata/<hash>.toml

# 文件位置：相同内容的文件共享一个 item，可有多条
[[paths]]
path = "novels/三体.epub"
size = 1245760
modification_time = 1690000000000

# 书目元数据：由文件解析自动填充；用户手改过的字段记入 overridden_fields，
# refresh_metadata 重解析时不覆盖（除非 force）
title = "三体"
authors = ["刘慈欣"]
publisher = "重庆出版社"
pubdate = "2008-01-01"
isbn = "9787536692930"
language = "zh"
series = "地球往事"
series_index = 1
description = "……"
overridden_fields = ["publisher"]

# 阅读状态
read_status = "finished"      # unread / reading / finished / abandoned
progress = 100                # 0–100
progress_loc = "epubcfi(/6/24!/4/2/1:0)"
last_read_time = 1700000000000

tags = ["科幻"]
categories = ["小说"]
star = 5
annotation = ""
url = ""
added_time = 1690000000000

# 派生信息（内容的纯函数，一台计算全平台复用）
cover_width = 600
cover_height = 900
```

**派生信息的归属**：封面尺寸是「内容的纯函数」（同 hash 各平台计算结果必然一致），直接写入元数据 TOML——一台计算、全平台复用。自动提取封面、全文索引、归一化阅读内容为可重建的二进制/索引产物，进库外派生缓存。**自定义封面不是派生物**（用户数据，删了不可重建），存 `.sumi/covers/` 参与同步，见下节。

只识别 `<hash>.toml` 命名的文件；网盘同步冲突产生的副本（如 `<hash>.sync-conflict-20250101.toml`）直接忽略，不参与索引。元数据写入采用「临时文件 + rename」的原子写，避免网盘同步走写了一半的文件。

## 锁（文件夹/分类/标签/作者）

`.sumi/locks.toml` 存放四维度锁（每条独立密码，Argon2id PHC 字符串自含盐与参数）：

```toml
[[folders]]
path = "private/日记"
password = "$argon2id$v=19$m=19456,t=2,p=1$..."

[[categories]]
name = "私密"
password = "..."

[[tags]]
name = "nsfw"
password = "..."

[[authors]]
name = "某作者"
password = "..."
```

与 global_filter 的客户端约定式隐藏不同，锁由**服务端强制**：未解锁时全局视图直接排除锁定条目、主动筛选与内容直连（cover/file/detail/toc/content/resource/open/show_in_folder）403 `LOCKED`、SSE 不广播被锁书籍的元数据事件；解锁票据（daemon 内存，重启失效）由各客户端独立持有，经 `X-Sumi-Unlock` 头或 `?unlock=` 查询参数附带。级联跟随与 global_filter 同款（文件夹移动/重命名/回收站迁移，分类/标签/作者改名/删除/合并）。判定口径、票据通道与安全边界详见 server-rest-api-v1.md 的 lock 节。

已知边界：锁不是加密（文件系统层面无保护，仅防「通过 sumi 查看」）；密码哈希参与同步意味着拿到 `.sumi/` 的人可离线爆破（Argon2id 慢哈希缓解）；验证失败全局节流（连续 5 次冷却 60s）防在线爆破。

## 封面存储

封面分两条来源链，**自定义优先**：

| 来源 | 位置 | 性质 | 同步 |
| ---- | ---- | ---- | ---- |
| 自定义封面 | `.sumi/covers/<hash>.webp` | 用户数据（`PUT /api/v1/item/cover` 设置） | 是 |
| 自动提取封面 | 库外缓存 `covers/<hash>.webp` | 派生缓存（epub/mobi 内嵌、pdf/cbz 首页渲染、txt/md/docx 排版生成） | 否 |

**文件即真源**：`Item.custom_cover` 由 `.sumi/covers/<hash>.webp` 是否存在派生，元数据中不存对应字段（无双写）。`DELETE /api/v1/item/cover` 删除文件即回退自动提取链；`refresh_metadata` 只重建自动提取封面，不动自定义封面。

id 漂移（书籍文件内容变更 → hash 变化）时，元数据按「路径 + 文件名」启发式迁移到新 id，自定义封面文件一并重命名跟随。

## 回收站

删除的书籍移入 `.sumi/trash/`（保留目录结构），元数据保留。`POST /api/v1/trash/clear` 彻底删除：文件、元数据、派生封面缓存与自定义封面一并清理，不可恢复。

「是否在回收站」不是独立属性，由文件位置派生：位于 `.sumi/trash/` 内即在回收站。回收站是本地目录，不参与同步——恢复操作只能在执行删除的机器上完成。移入 trash 的书籍，其元数据保留不删，`paths` 仍记录原来的库内路径，作为恢复时的放回目标；清空回收站时清理对应的元数据、派生封面与自定义封面——但仅限该内容在库内已无其他位置引用的情况（同一哈希的文件可能仍存在于其他目录）。

## 内存索引与启动流程

索引完全在内存中维护，`.sumi/` 中不存在任何索引文件。元数据副本的注水来源按存储方案分：数据库模式直接读 `.sumi/metadata.db`（一次顺序读）；配置文件模式走库外 SQLite 派生缓存（见下节）→ `.sumi/metadata/*.toml` 全量解析（缓存缺失/损坏时的一次性慢路径，解析完顺带重建缓存）。启动流程（阶段名与 `app/startup` 的 `phase` 对应）：

1. **元数据对账**（`sync`，仅配置文件模式）：按 mtime 比对 `metadata/` 与派生缓存，把外部变更（网盘同步落地、手工编辑）搬进缓存与内存索引；随后注水元数据副本（缓存快路径，或 TOML 全量回退，回退经 startup 进度上报）
2. **扫描**（`scan`）：遍历书库目录得到全部文件清单（路径、大小、mtime）——只读目录，不读文件内容（total 恒 0，客户端显示不定态进度）
3. **哈希**（`hash`，并行）：逐文件与元数据比对——
   - 路径存在于某元数据的 `paths` 且 `size`、`modification_time` 一致 → 复用哈希（即元数据文件名），不重算
   - 路径不存在 → 新文件，计算哈希并入库
   - 大小或 mtime 变化 → 重算该文件哈希，按「路径不变、hash 变」迁移元数据（规则见「内容寻址」节）
   - 元数据中的路径已不存在 → 对应位置从索引移除
4. **应用**（`apply`）：索引变更生效；哈希确认后将最新 `size`、`modification_time` 回写元数据，保持校验依据新鲜（否则下次启动会误判变动、重复哈希）
5. 运行期间由文件监听保持增量更新（见「实时文件监听」）

元数据本身就是哈希缓存（TOML 文件名即哈希，`paths` 记录校验依据），启动时无需读取文件内容。sumi 未运行期间对书库目录的改动会在下次启动时被上述比对自动发现；如对索引状态存疑，可手动调用 `POST /api/v1/library/reindex` 全量重建（见 API 文档）。

## 元数据派生缓存（SQLite，库外）

书目规模到数万本后，配置文件模式每次启动全量解析数万个 TOML 小文件成为主要瓶颈。为此在库外缓存目录引入 SQLite 派生缓存（`index.db`）：

- `.sumi/metadata/*.toml` 仍是唯一权威数据源（配置文件模式），语义不变（含同步冲突副本忽略规则）。`index.db` 分两部分：**元数据镜像**（items / paths / tags / categories 表 + 每条记录来源 TOML 的 mtime `source_mtime`，对账比对依据）与 **FTS5 全文索引**（正文倒排，分词设计见 [fulltext-search.md](fulltext-search.md)）。不参与同步，可随时删除，重启后重建（仅缓存删除后首次启动慢）
- **数据库模式只用 FTS 部分**：权威源已是 SQLite，元数据镜像无存在意义（不注水、不写回），但全文索引两模式都要建——`index.db` 不随存储模式删除
- 写入顺序铁律：先 TOML（临时文件 + rename 原子写），成功后再写缓存——中途崩溃自然朝 TOML 收敛，无需回滚协议；缓存写采用待冲刷缓冲（内存副本即时更新，SQLite 写累积批量后单事务落盘）摊薄事务开销
- 对账只进不出：周期对账（跟随文件对账节奏，默认 60s）按 mtime 比对 `metadata/` 与缓存，把外部变更搬进缓存与内存索引；TOML 消失则清空该书目参数（等价于重启后无元数据的语义，item 与位置由扫描决定存续）。缓存永不反向生成 TOML
- 任何缓存故障（打不开/写失败/读损坏）只退化性能（回到纯 TOML 行为），绝不影响权威数据；schema 版本不符直接重建（缓存是 TOML 的纯镜像 + 可重建索引，重建即全量恢复）

已知取舍：多机同时编辑时，网盘同步落地到对账收敛之间有秒级窗口，查询可能短暂滞后；单机使用无感。多机同步冲突的语义与引入缓存前完全一致（冲突副本被忽略，由网盘自行裁决谁是 `<hash>.toml`）。

## 项目配置

每个书库的设置保存在 `.sumi/config.toml` 中，随书库目录一起同步、备份。库首次打开时若缺失会自动生成带注释的默认模板（已存在绝不覆盖，用户手工编辑安全）：

```toml
# .sumi/config.toml 示例

# 书库名（界面显示用）
name = "我的书库"

# 索引时忽略的路径
ignore = ["node_modules", "*.tmp"]

# 可见扩展名白名单：只索引这些后缀的文件
# （缺省为 v1 支持格式全集：epub/pdf/txt/md/mobi/azw3/docx/cbz，见 API 文档「支持格式」）
# extensions = ["epub", "pdf", "txt", "md"]

# 周期兜底重扫（监听漏事件的最终一致保证；设置面板「存储」分区可开关）
[scan]
periodic = true
interval = 900

# 局域网 web 书库查看（桌面端设置面板读写，按库隔离，多库互不冲突）
[web]
enabled = false      # 开启后 server 追加监听 0.0.0.0:<port>，并托管前端页面
port = 27382
token = ""           # viewer token；浏览器打开 http://<电脑IP>:<port> 后输入
writable = false     # 允许写：开启后查看端可上传/删除/修改（与桌面端同等操作），请谨慎授权
separate_write_token = false  # 拆分只读/可写 token：token 降为只读，write_token 可写（不拆分时 token 读写兼具）
write_token = ""     # 拆分模式下的可写 token
```

`extensions`（可见扩展名白名单）保存即热生效：变化触发一次强制重扫，白名单外的既有条目从索引移除（文件本身不动，非侵入式；`library/cleanup_index` 亦可手动清理），新加入白名单的后缀随即入库；回收站不参与过滤（已回收的书籍仍可见、可恢复）。入库入口（`item/add`、`item/upload`）对白名单外的格式直接拒绝（`UNSUPPORTED_FORMAT`），`ignore` 命中拒绝（`INVALID_PARAM`）；`item/update` 的改名/移动目标同样受此约束。

`[scan]` 保存即热生效（见 API 文档 `library/scan`）。

`[web]` 保存即热生效（daemon 权威写配置：toml_edit 就地改键值保注释、原子写 → LAN 监听 supervisor 运行期重绑；绑定失败自动回滚旧配置，见 `PUT /api/v1/app/lan`）。token 能力三档：`writable = false` 时一律只读（写端点 `403 READ_ONLY`，放行一切 GET 与 `item/list`、`item/skeleton` 等查询类 POST）；`writable = true` 且未拆分时 `token` 读写兼具；拆分时 `token` 只读、`write_token` 可写（仅在 `writable = true` 且拆分时才是合法 token）。web 端写能力由 `app/info` 的 `writable` 字段按当前 token 告知前端。

`config.toml` 解析错误时 daemon 保留上次有效配置继续运行（`app/status` 的 `config_error` 上报，见 API 文档），文件修复后自动清除。

全局配置文件位于 `~/.config/sumi/config.toml`，只存放跨书库的全局设置（目前没有全局配置项）。

## 内容寻址（Content-Addressable）

item id = 文件内容的 BLAKE3 哈希（hex）。同一内容多处存放共享一个 item；内容变更导致 id 漂移。元数据 TOML 中的 `[[paths]]` 是迁移的依据：内容变更（监听事件/增量扫描/全量重建）时按「路径不变、hash 变」匹配旧元数据，自动迁移到新 hash——旧 hash 无剩余位置时整体移动（rename），仍有其他位置时复制；目标 hash 已有元数据时不迁移（去重语义优先）。迁移覆盖全部用户数据（含自定义封面、阅读进度）。该匹配是启发式的，客户端不应假设 id 永久稳定——迁移规则的完整契约见 API 文档「Item 对象」节。

## 实时文件监听

sumi 通过文件系统事件（notify）实时感知变化，新增、删除、重命名、修改文件时索引自动更新。`.sumi/` 目录自身不参与监听与索引（`config.toml` 与注册表文件除外：categories.toml / tags.toml / view.toml / global_filter.toml / locks.toml 的变更同样被监听，修改后自动生效并广播对应 SSE 事件）。

文件监听可能静默丢事件（尤其 macOS FSEvents，无溢出错误可捕获），最终一致由四层兜底：**周期兜底扫描**（库配置 `[scan]`，默认 900s，可关；强制遍历全部文件、按 size/mtime 复用哈希不读内容——目录快照只能发现增删改名，漏掉的内容变更只有全量 stat 能收敛）、启动扫描（停机期间变更）、监听缓冲溢出自动触发全库强制遍历兑底；系统明确告知某路径丢事件（如 macOS FSEvents must-scan-subdirs 标记）则定向强制重扫该路径；另可手动 `POST /api/v1/library/rescan`（可带 `path` 限定子树）。**元数据对账**是另一条线（默认 60s，可关）：只并入 `.sumi/metadata/` 的外部变更，不跑文件系统扫描。周期兑底重扫关闭后，实时监听仍是发现新增/删除的主路径，仅漏事件需手动重扫。
