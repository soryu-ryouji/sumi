// 书内拖拽（书籍卡片 → 文件夹树）与外部文件入库拖拽的区分。
// 书内拖拽用自定义 MIME 标记 item id 列表；外部文件走 dataTransfer.files，
// 两者互不干扰：drop 目标只认本 MIME，整窗导入只看 files。
const MIME = 'application/x-sumi-items';

/** 拖拽开始（BookCard dragstart）：写入选中集 item id 列表 */
export function startItemsDrag(e: DragEvent, ids: string[]): void {
  if (!e.dataTransfer) {
    return;
  }
  e.dataTransfer.setData(MIME, JSON.stringify(ids));
  e.dataTransfer.effectAllowed = 'move';
}

/** dragover 阶段只能读 types（浏览器安全限制，getData 仅 drop 可用） */
export function hasItemsDrag(e: DragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes(MIME);
}

/** drop 阶段解析 item id 列表；非本书内拖拽或数据非法返回 null */
export function readItemsDrop(e: DragEvent): string[] | null {
  const raw = e.dataTransfer?.getData(MIME);
  if (!raw) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (Array.isArray(parsed) && parsed.length > 0 && parsed.every((x) => typeof x === 'string')) {
      return parsed as string[];
    }
  } catch {
    // 落到 null
  }
  return null;
}
