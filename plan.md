# sumi-daemon 实现计划

权威依据：docs/backend/server-rest-api-v1.md（契约）、docs/backend/storage.md（存储）、docs/backend/fulltext-search.md（FTS）、docs/backend/server-rust.md（蓝图）。本文档只跟踪进度，不复制设计。

## 步骤

- [x] S0 环境与骨架：git init、cargo 项目、依赖清单、plan.md
- [x] S1 HTTP 骨架：信封（success/error + 错误码）、鉴权中间件（admin token + viewer 三档 + 查询参数 token）、startup 就绪网关（503 NOT_READY）、/health、app 端点组（info/startup/status/token）、SSE events 通道
- [x] S2 core 数据模型：Item（含 paths 多位置、回收站派生、用户编辑字段集合）、LibraryInfo、存储模式探测（storage_mode 标记优先）
- [x] S3 元数据存储：数据库模式（metadata.db，rusqlite）、配置文件模式（metadata/*.toml 原子写 + 冲突副本忽略）、注册表文件（categories/tags/view/global_filter/locks + config.toml 模板生成与监听重载）
- [x] S4 内存索引 + 索引流水线：Job 有界队列、单写者消费循环、watcher（notify）、扫描 runner（startup 四阶段 sync/scan/hash/apply + 运行期周期重扫）、id 漂移迁移（移动/复制/去重三分支）
- [x] S5 FTS5 全文检索：core/fulltext.rs（预分词+unicode61 实现「CJK 单字+西文词」，子串=短语邻接、多关键词 AND、引号短语、注入防护、纯标点忽略）；fulltext-search.md 实现说明已同步。**未完**：元数据镜像（source_mtime 对账）与流水线接线（入库时提取正文写 fts、id 漂移随迁、content 参数查询），随 S7 一并做
- [x] S6 解析器：txt/md（编码探测 chardetng）、epub（zip+quick-xml：OPF/NCX/nav、封面、XHTML 拼接+锚点注入+资源改写）、docx（core.xml+document.xml）、pdf（pdfium-render 首页+书签）、cbz（zip 首图）、mobi/azw3（EXTH 元数据+封面；KF8 正文尽力）；排版封面生成（txt/md/docx 书名+作者）；webp 封面管线
- [ ] S7 item API 组（读+写路径核心完成，阅读器端点待做）：list/skeleton/aggregate/detail/count/add/upload/update/batch_update/delete/restore/cover(GET/PUT/DELETE)/file(Range)/open/show_in_folder/toc/content/resource/refresh_metadata
- [ ] S8 其余 API：folder/category/tag/author/series/view/global_filter/lock/trash/library（info/scan/reindex/rescan/refresh_cache/cleanup_index/storage_mode）+ app/lan
- [ ] S9 测试与收尾：解析器单测（testdata 生成脚本）、流水线行为测试、OpenAPI（utoipa）+ 契约校验、冒烟脚本 tools/smoke.sh、全部文档交叉核对、删除 plan.md

## S7 剩余（阅读器端点，直接从 api/item.rs 续写）

- update/batch_update/add/delete/restore 已完成并冒烟验证（改名移动/用户编辑标记/批量并集/回收站/already_existed/skip_existing/FTS 接入）
- toc/content/resource（解析器已就绪：epub.rs 的 spine_docs/xhtml_text 可复用为归一化 HTML 生成 + 锚点注入 + 资源改写；txt/md 编码归一直出；pdf/cbz UNSUPPORTED_FORMAT）
- refresh_metadata（force 清除 overridden_fields，重建自动封面）

## 已知简化（v1 后续打磨）

- 解析为流水线同步内联（正确性优先）；worker 化（CPU/4 封顶 8 的 cover 队列）与 task.progress 节流推送待接入
- pdf 解析（pdfium 首页渲染 + 书签大纲）未实现：契约降级路径为封面 404 占位 + toc 空数组，阅读走 item/file + Range + pdf.js；pdfium-render 动态库分发在打包阶段处理
- watcher 的 rename 事件按 remove+add 防抖处理（由周期扫描兜底收敛），From/To 精确配对待加
- index.db 的元数据镜像（source_mtime 对账加速注水）未接：当前注水直接读权威层（大库首启慢路径）

## 进度日志

- S7（写路径）完成：update（解析字段 overridden 标记、name/folder_path 移动真实文件+白名单校验、last_read_time 联动）、batch_update（并集追加/移除、移动冲突跳过记 missing_ids、items.updated 事件）、add（path/url/base64 三来源、ext 从 body.name 回落、skip_existing、时间戳保留 filetime、add 内联解析派生 FTS/封面）、delete/restore（回收站 rename + apply_trash_move/apply_restore 复用流水线）。add upload(multipart) 待做。**修 bug**：base64 来源 source_name 空导致 ext 丢失
- S7（读路径）完成：过滤引擎（ids/keywords/content FTS/tags/categories+match/authors/series/publisher/language/isbn/read_status/exclude_*/star/folders+exact/without_*/ext/annotation/url/in_trash 全参数）、排序稳定性（id tiebreak）、list/skeleton/aggregate/detail/count、cover GET 四分支（自定义→缓存→即时生成→404）+PUT/DELETE（admin/可写 viewer）、file（Range+immutable）、LockView（X-Sumi-Unlock 票据、主动筛选 403、全局视图排除）。冒烟全过。
- S6 完成：parser::text（chardetng+encoding_rs 编码归一、front matter、章节启发式、md 层级目录）、epub（zip+quick-xml：OPF/NCX/nav、calibre 系列、cover-image、spine 全文、XHTML 文本提取）、docx（core.xml、document.xml 正文+Heading 大纲+首图封面）、cbz（首图封面）、mobi（EXTH 元数据+封面 record；KF8 正文列后续）、pdf（降级占位）；cover.rs（提取链：解码→Lanczos3 缩入 1024→webp q80；排版生成：系统字体回退+自动换行，缺字体纯渐变）；流水线接线（derive_book_facts：一次打开产出元数据+封面+FTS；id 漂移迁移封面/内容/FTS；drop 清缓存）。44 单测全过，端到端冒烟（txt→title/封面尺寸/FTS 短语命中/id 一致）通过。**坑**：mobi crate 实际版本 0.8；XML 元素名带命名空间前缀需按本地名匹配；TOML 依赖块被误追加进 [profile.release] 完成（git init、cargo 骨架）
- S1 完成：信封/错误码全集、admin+viewer 三档鉴权（含直链 ?token= 通道）、startup 网关、/health、app/info|startup|status|token（Host 环回校验 INVALID_HOST）、SSE events、config.toml 模板生成+坏文件保留上次有效配置+原子写回、TaskTracker、--dump-openapi。冒烟验证通过
- S2/S3 完成：ItemCore 模型（paths 多位置/overridden_fields/主路径派生）、元数据 TOML 手写序列化（roundtrip 测试）、双模式 MetadataStore（metadata.db schema v1 / *.toml、探测 storage_mode 优先、迁移互转、冲突副本忽略）、注册表文件族（NameRegistry/ViewPreferences 级联/GlobalFilter 级联/Locks 含 Argon2id+解锁票据+节流计数+级联迁移）。22 单测
- S4 完成：ItemIndex（维度聚合/ids_with_named/回收站派生）、pipeline.rs（apply_file_fact：哈希→id 漂移三分支迁移→存储→事件；apply_path_removed/trash_move/restore/drop_item 含派生缓存清理）、scanner.rs（scan_library 白名单/ignore/隐藏过滤、reconcile_with_index 差异对账、hydrate_from_store、周期重扫循环）、watcher.rs（notify 300ms 防抖、注册表文件热重载、trash 位置变更）、bootstrap 四阶段启动接线。端到端冒烟验证：入库/id 漂移/同内容多位置去重/单位置删除/条目清理全过。**坑已修**：macOS /tmp 是 /private/tmp 符号链接，root 需 canonicalize（FSEvents 报真实路径）
