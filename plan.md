# sumi-daemon 实现计划

权威依据：docs/backend/server-rest-api-v1.md（契约）、docs/backend/storage.md（存储）、docs/backend/fulltext-search.md（FTS）、docs/backend/server-rust.md（蓝图）。本文档只跟踪进度，不复制设计。

## 步骤

- [x] S0 环境与骨架：git init、cargo 项目、依赖清单、plan.md
- [ ] S1 HTTP 骨架：信封（success/error + 错误码）、鉴权中间件（admin token + viewer 三档 + 查询参数 token）、startup 就绪网关（503 NOT_READY）、/health、app 端点组（info/startup/status/token）、SSE events 通道
- [ ] S2 core 数据模型：Item（含 paths 多位置、回收站派生、用户编辑字段集合）、LibraryInfo、存储模式探测（storage_mode 标记优先）
- [ ] S3 元数据存储：数据库模式（metadata.db，rusqlite）、配置文件模式（metadata/*.toml 原子写 + 冲突副本忽略）、注册表文件（categories/tags/view/global_filter/locks + config.toml 模板生成与监听重载）
- [ ] S4 内存索引 + 索引流水线：Job 有界队列、单写者消费循环、watcher（notify）、扫描 runner（startup 四阶段 sync/scan/hash/apply + 运行期周期重扫）、id 漂移迁移（移动/复制/去重三分支）
- [ ] S5 派生缓存 index.db：元数据镜像（source_mtime 对账）+ FTS5（sumi_tokenizer 自定义分词 + 查询映射）、缓存目录管理（<库标识>、--cache-parent）
- [ ] S6 解析器：txt/md（编码探测 chardetng）、epub（zip+quick-xml：OPF/NCX/nav、封面、XHTML 拼接+锚点注入+资源改写）、docx（core.xml+document.xml）、pdf（pdfium-render 首页+书签）、cbz（zip 首图）、mobi/azw3（EXTH 元数据+封面；KF8 正文尽力）；排版封面生成（txt/md/docx 书名+作者）；webp 封面管线
- [ ] S7 item API 组：list/skeleton/aggregate/detail/count/add/upload/update/batch_update/delete/restore/cover(GET/PUT/DELETE)/file(Range)/open/show_in_folder/toc/content/resource/refresh_metadata
- [ ] S8 其余 API：folder/category/tag/author/series/view/global_filter/lock/trash/library（info/scan/reindex/rescan/refresh_cache/cleanup_index/storage_mode）+ app/lan
- [ ] S9 测试与收尾：解析器单测（testdata 生成脚本）、流水线行为测试、OpenAPI（utoipa）+ 契约校验、冒烟脚本 tools/smoke.sh、全部文档交叉核对、删除 plan.md

## 进度日志

- S0 完成（git init、cargo 骨架）
