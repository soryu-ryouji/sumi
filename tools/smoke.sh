#!/usr/bin/env bash
# 端到端冒烟：启动 → 入库 → 查询/全文/封面 → 元数据/回收站 → 分类/锁 → 清理。
# 行为契约测试（对齐 docs/backend/server-rest-api-v1.md）；需先 cargo build --release
set -euo pipefail
cd "$(dirname "$0")/../sumi-daemon"

BIN="${SUMI_BIN:-target/release/sumi-daemon}"
[ -x "$BIN" ] || BIN="target/debug/sumi-daemon"
LIB="$(mktemp -d /tmp/sumi-smoke.XXXXXX)"
PORT=$((27400 + RANDOM % 100))
TOKEN="smoketoken$$"
cleanup() { kill "$DAEMON_PID" 2>/dev/null || true; rm -rf "$LIB"; }
trap cleanup EXIT

SUMI_TOKEN="$TOKEN" "$BIN" --library "$LIB" --port "$PORT" >"$LIB/daemon.log" 2>&1 &
DAEMON_PID=$!

api() { curl -sf -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" "$@"; }
post() { api -d "$2" "http://127.0.0.1:$PORT/api/v1/$1"; }
# 期待 4xx 的请求（curl -f 会以退出码 22 中断 pipefail）
post_any() { curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d "$2" "http://127.0.0.1:$PORT/api/v1/$1"; }

# 等就绪（200ms 轮询）
for _ in $(seq 1 50); do
  api "http://127.0.0.1:$PORT/api/v1/app/startup" | grep -q '"ready"' && break
  sleep 0.2
done
api "http://127.0.0.1:$PORT/api/v1/app/startup" | grep -q '"ready"' || { echo "FAIL: startup 未就绪"; cat "$LIB/daemon.log"; exit 1; }
echo "ok startup"

# 鉴权
curl -s "http://127.0.0.1:$PORT/api/v1/item/count" | grep -q UNAUTHORIZED || { echo "FAIL: 无 token 应 401"; exit 1; }
curl -sf "http://127.0.0.1:$PORT/api/v1/app/token" | grep -q "$TOKEN" || { echo "FAIL: token 发现"; exit 1; }
echo "ok auth/token-discovery"

# 入库（txt 中文 + epub 内容由 base64 注入）
mkdir -p "$LIB/novels"
printf '第一章 科学边界\n黑暗森林法则的测试正文 unique-smoke-token\n' > "$LIB/novels/三体.txt"
sleep 2
post item/list '{"keywords":["三体"]}' | grep -q '"total":1' || { echo "FAIL: 入库"; exit 1; }
echo "ok ingest"

ID=$(post item/list '{"keywords":["三体"]}' | python3 -c "import json,sys; print(json.load(sys.stdin)['data']['items'][0]['id'])")

# 全文检索（中文子串）
post item/list '{"content":"黑暗森林"}' | grep -q "$ID" || { echo "FAIL: 全文检索"; exit 1; }
echo "ok fulltext"

# 封面（webp）
CT=$(curl -sf -o /dev/null -w '%{content_type}' "http://127.0.0.1:$PORT/api/v1/item/cover?id=$ID&token=$TOKEN")
[ "$CT" = "image/webp" ] || { echo "FAIL: 封面 ($CT)"; exit 1; }
echo "ok cover"

# 原文件 Range
curl -sf -H "Authorization: Bearer $TOKEN" -r 0-3 "http://127.0.0.1:$PORT/api/v1/item/file?id=$ID" -o /tmp/sumi-smoke-range -w '' 
[ "$(wc -c < /tmp/sumi-smoke-range | tr -d ' ')" = "4" ] || { echo "FAIL: Range"; exit 1; }
echo "ok range"

# 元数据更新（标签 + 用户编辑保护）
post item/update "{\"id\":\"$ID\",\"tags\":[\"科幻\"],\"star\":5,\"publisher\":\"测试社\"}" | grep -q '"star":5' || { echo "FAIL: update"; exit 1; }
post item/refresh_metadata "{\"id\":\"$ID\"}" | grep -q success || { echo "FAIL: refresh"; exit 1; }
sqlite3 "$LIB/.sumi/metadata.db" "SELECT publisher FROM items WHERE id='$ID'" | grep -q "测试社" || { echo "FAIL: 用户编辑保护"; exit 1; }
echo "ok update/refresh(overridden)"

# 分类注册表（空分类预创建 + 聚合并集）
post category/create '{"name":"想读"}' | grep -q success || { echo "FAIL: category create"; exit 1; }
api "http://127.0.0.1:$PORT/api/v1/category/list" | grep -q '"count":0' || { echo "FAIL: 空分类计数"; exit 1; }
post item/update "{\"id\":\"$ID\",\"categories\":[\"小说\"]}" | grep -q '"小说"' || { echo "FAIL: 分类赋值"; exit 1; }
api "http://127.0.0.1:$PORT/api/v1/category/list" | grep -q '小说' || { echo "FAIL: 分类并集"; exit 1; }
echo "ok category"

# 锁：设锁 → 未解锁 403 → 解锁票据放行
post lock/set '{"dimension":"tag","name":"私密","password":"pw123"}' | grep -q success || { echo "FAIL: lock set"; exit 1; }
post item/update "{\"id\":\"$ID\",\"tags\":[\"私密\"]}" | grep -q success || { echo "FAIL: 打私密标签"; exit 1; }
post_any item/list '{"tags":["私密"]}' | grep -q LOCKED || { echo "FAIL: 未解锁筛选应 403"; exit 1; }
TICKET=$(post lock/unlock '{"dimension":"tag","name":"私密","password":"pw123"}' | python3 -c "import json,sys; print(json.load(sys.stdin)['data']['unlock_token'])")
curl -sf -H "Authorization: Bearer $TOKEN" -H "X-Sumi-Unlock: $TICKET" -H "Content-Type: application/json" -d '{"tags":["私密"]}' "http://127.0.0.1:$PORT/api/v1/item/list" | grep -q '"total":1' || { echo "FAIL: 解锁票据"; exit 1; }
post item/list '{"keywords":["三体"]}' | grep -q '"total":0' || { echo "FAIL: 全局视图应排除锁内条目"; exit 1; }
post lock/remove '{"dimension":"tag","name":"私密","password":"pw123"}' | grep -q success || { echo "FAIL: lock remove"; exit 1; }
post item/list '{"keywords":["三体"]}' | grep -q '"total":1' || { echo "FAIL: 解锁后应恢复可见"; exit 1; }
echo "ok lock/unlock-ticket/global-view"

# 回收站往返
post item/delete "{\"id\":\"$ID\"}" | grep -q success || { echo "FAIL: delete"; exit 1; }
post item/list '{"in_trash":true}' | grep -q '"total":1' || { echo "FAIL: 回收站视图"; exit 1; }
post item/restore "{\"id\":\"$ID\"}" | grep -q success || { echo "FAIL: restore"; exit 1; }
api "http://127.0.0.1:$PORT/api/v1/item/count" | grep -q '"data":1' || { echo "FAIL: 恢复后计数"; exit 1; }
echo "ok trash-roundtrip"

# 文件夹树与文件夹操作
post folder/create '{"name":"已读","parent_path":""}' | grep -q success || { echo "FAIL: folder create"; exit 1; }
api "http://127.0.0.1:$PORT/api/v1/folder/list" | grep -q '已读' || { echo "FAIL: folder list"; exit 1; }
post folder/delete '{"path":"已读"}' | grep -q success || { echo "FAIL: folder delete"; exit 1; }
echo "ok folder"

# view 偏好与 global_filter
api -X PUT -d '{"scope":"tag:科幻","order_by":"title","order":"asc"}' "http://127.0.0.1:$PORT/api/v1/view/preference" | grep -q success || { echo "FAIL: view put"; exit 1; }
api "http://127.0.0.1:$PORT/api/v1/view/preferences" | grep -q '科幻' || { echo "FAIL: view get"; exit 1; }
api -X PUT -d '{"kind":"tag","name":"科幻","hidden":true}' "http://127.0.0.1:$PORT/api/v1/global_filter" | grep -q success || { echo "FAIL: gf put"; exit 1; }
post item/list '{"keywords":["三体"]}' | grep -q '"total":1' || { echo "FAIL: global_filter 为客户端约定式（服务端放行，客户端附 exclude_*）"; exit 1; }
post item/list '{"keywords":["三体"],"exclude_tags":["私密"]}' | grep -q '"total":0' || { echo "FAIL: exclude_tags 剔除"; exit 1; }
echo "ok view/global_filter"

# library 信息与扫描
api "http://127.0.0.1:$PORT/api/v1/library/info" | grep -q '"storage_mode":"database"' || { echo "FAIL: library info"; exit 1; }
post library/rescan '{}' | grep -q success || { echo "FAIL: rescan"; exit 1; }
sleep 1
api "http://127.0.0.1:$PORT/api/v1/item/count" | grep -q '"data":1' || { echo "FAIL: rescan 后计数"; exit 1; }
echo "ok library/rescan"

# 回收站回归：trashed 条目在任何 rescan 后必须存活
# （曾因扫描器不枚举 .sumi/trash 导致条目被对账永久删除）
post item/delete "{\"id\":\"$ID\"}" | grep -q success || { echo "FAIL: re-delete"; exit 1; }
sleep 0.6
post library/rescan '{}' | grep -q success || { echo "FAIL: rescan(trash)"; exit 1; }
sleep 1
post item/list '{"in_trash":true}' | grep -q '"total":1' || { echo "FAIL: rescan 后回收站条目应存活"; exit 1; }
post item/restore "{\"id\":\"$ID\"}" | grep -q success || { echo "FAIL: re-restore"; exit 1; }
api "http://127.0.0.1:$PORT/api/v1/item/count" | grep -q '"data":1' || { echo "FAIL: 二次恢复后计数"; exit 1; }
echo "ok trash-survives-rescan"

# 子树 rescan 回归：限定 path 的重扫不得影响子树外条目
# （曾因对账以全库索引对照子树事实，子树外条目被误判消失而删除）
mkdir -p "$LIB/subA" "$LIB/subB"
printf 'subtree-a\n' > "$LIB/subA/a.txt"
printf 'subtree-b\n' > "$LIB/subB/b.txt"
sleep 2
post library/rescan '{"path":"subA"}' | grep -q success || { echo "FAIL: subtree rescan"; exit 1; }
sleep 1
api "http://127.0.0.1:$PORT/api/v1/item/count" | grep -q '"data":3' || { echo "FAIL: 子树 rescan 不应影响子树外条目"; exit 1; }
echo "ok subtree-rescan"

# 存储模式切换（database → toml → database）
post library/storage_mode '{"mode":"toml"}' | grep -q success || { echo "FAIL: 迁移 toml"; exit 1; }
[ "$(ls "$LIB"/.sumi/metadata/*.toml 2>/dev/null | wc -l | tr -d ' ')" = "3" ] || { echo "FAIL: toml 文件"; exit 1; }
echo "ok storage-mode-migrate"

# SSE 订阅（触发一次事件应收到帧）
( post item/update "{\"id\":\"$ID\",\"annotation\":\"sse-test\"}" & 
  curl -s --max-time 3 -N "http://127.0.0.1:$PORT/api/v1/events?token=$TOKEN" > "$LIB/sse.txt" ) || true
grep -q 'item.updated' "$LIB/sse.txt" || { echo "FAIL: SSE 事件"; exit 1; }
echo "ok sse"

echo ""
echo "SMOKE PASS（全部断言通过）"
