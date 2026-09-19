// item 位置口径的统一出口：后端 primary_path() = 第一个非回收站位置。
// 前端任何「主位置」的展示/操作（移动对话框当前标记、重命名初始值、路径展示等）
// 都必须走 primaryPathOf，不能直取 paths[0]——多位置 item 首位置在回收站时会取错。
import type { Item } from '@/shared/api/types';

/** 回收站前缀（与后端 paths::TRASH_PREFIX 一致；库内路径一律 '/' 分隔） */
const TRASH_PREFIX = '.sumi/trash/';

/** 主位置 = 第一个非回收站位置；全部在回收站时退回首位置（回收站视图的展示口径） */
export function primaryPathOf(item: Item): string {
  return item.paths.find((p) => !p.startsWith(TRASH_PREFIX)) ?? item.paths[0] ?? '';
}
