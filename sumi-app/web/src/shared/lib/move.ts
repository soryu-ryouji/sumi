// 移动/重命名/文件夹管理的统一动作层：组装 REST 请求，成功后统一刷新列表与文件夹树，
// 并以 toast 报告结果（含批量部分失败的明确提示）；错误统一 toastError。
import { api } from '@/shared/api/client';
import { primaryPathOf } from '@/shared/lib/item';
import { useUi } from '@/stores/ui';
import { useBooks } from '@/stores/books';
import { useLibrary } from '@/stores/library';

/** 路径的目录部分（库内相对路径以 '/' 分隔；根级文件返回 ''） */
function dirOf(path: string): string {
  const i = path.lastIndexOf('/');
  return i < 0 ? '' : path.slice(0, i);
}

/** 文件夹显示名（'' = 书库根目录） */
function folderLabel(folderPath: string): string {
  return folderPath || '书库根目录';
}

/** item 是否已有位置位于目标文件夹（全部命中时移动无意义） */
function allInFolder(ids: string[], folderPath: string): boolean {
  const books = useBooks();
  return ids.every((id) => {
    const item = books.items.find((i) => i.id === id);
    return item !== undefined && item.paths.some((p) => dirOf(p) === folderPath);
  });
}

/** 移动书籍到文件夹（根 = ''）：单个走 item/update，多个走 item/batch_update */
export async function moveItemsToFolder(ids: string[], folderPath: string): Promise<void> {
  const ui = useUi();
  const books = useBooks();
  const library = useLibrary();
  const unique = [...new Set(ids)];
  if (!unique.length) {
    return;
  }
  if (allInFolder(unique, folderPath)) {
    ui.toast('已在目标文件夹');
    return;
  }
  try {
    if (unique.length === 1) {
      await api('/item/update', { method: 'POST', body: { id: unique[0], folder_path: folderPath } });
      ui.toast(`已移动到 ${folderLabel(folderPath)}`);
    } else {
      // 响应 data: { updated, missing_ids }；missing_ids = 目标同名冲突/文件缺失等原因未移动者
      const res = await api<{ updated: number; missing_ids: string[] }>('/item/batch_update', {
        method: 'POST',
        body: { ids: unique, folder_path: folderPath },
      });
      ui.toast(`已移动 ${res.updated} 本到 ${folderLabel(folderPath)}`);
      if (res.missing_ids.length > 0) {
        ui.toast(`${res.missing_ids.length} 本因目标已存在同名文件等原因未移动`, 'error');
      }
    }
    // 位置变了：当前筛选下书籍可能应消失/出现；新建了目标文件夹时树也要刷新
    await Promise.all([books.load(), library.refreshTree()]);
  } catch (e) {
    ui.toastError(e);
  }
}

/** 重命名单本书的文件名（扩展名由后端沿用，name 不含扩展名也安全） */
export async function renameItem(id: string, name: string): Promise<void> {
  const ui = useUi();
  const books = useBooks();
  try {
    await api('/item/update', { method: 'POST', body: { id, name } });
    ui.toast('已重命名');
    await books.load();
  } catch (e) {
    ui.toastError(e);
  }
}

/** 新建文件夹（path = 完整库内相对路径；根下即名字本身），内部拆 parent/name 调 folder/create。
 *  返回是否创建成功（树刷新完成后 resolve；失败已在内部 toast，调用方可据此决定后续动作） */
export async function createFolder(path: string): Promise<boolean> {
  const ui = useUi();
  const library = useLibrary();
  const i = path.lastIndexOf('/');
  const name = i < 0 ? path : path.slice(i + 1);
  const parentPath = i < 0 ? '' : path.slice(0, i);
  try {
    await api('/folder/create', { method: 'POST', body: { name, parent_path: parentPath } });
    ui.toast('文件夹已创建');
    await library.refreshTree();
    return true;
  } catch (e) {
    ui.toastError(e);
    return false;
  }
}

/** 重命名文件夹：folder/update { path, name }（parent_path 缺省 = 保持原位置） */
export async function renameFolder(path: string, name: string): Promise<void> {
  const ui = useUi();
  const books = useBooks();
  const library = useLibrary();
  try {
    await api('/folder/update', { method: 'POST', body: { path, name } });
    ui.toast('文件夹已重命名');
    // 位置路径前缀变了：书列表与筛选跟随迁移（旧路径键失效）
    const i = path.lastIndexOf('/');
    const newPrefix = (i < 0 ? '' : path.slice(0, i + 1)) + name;
    if (books.filter.folder !== null && (books.filter.folder === path || books.filter.folder.startsWith(`${path}/`))) {
      books.filter.folder = newPrefix + books.filter.folder.slice(path.length);
    }
    await Promise.all([books.load(), library.refreshTree()]);
  } catch (e) {
    ui.toastError(e);
  }
}

/** 删除文件夹（后端整体移入回收站）：被删子树命中的当前筛选一并清除 */
export async function deleteFolder(path: string): Promise<void> {
  const ui = useUi();
  const books = useBooks();
  const library = useLibrary();
  try {
    await api('/folder/delete', { method: 'POST', body: { path } });
    ui.toast('文件夹已移入回收站');
    if (books.filter.folder !== null && (books.filter.folder === path || books.filter.folder.startsWith(`${path}/`))) {
      books.filter.folder = null;
    }
    // 刷新前取快照：load() 会整体替换 items，快照里的路径还是删除前的库内路径
    const before = books.items;
    await Promise.all([books.load(), library.refreshTree()]);
    // 主位置落在被删子树内的书已进回收站：从选择集中移除，主选中脱离选择集时回退末位/清空
    const prefix = `${path}/`;
    const gone = new Set(
      before.filter((i) => primaryPathOf(i).startsWith(prefix)).map((i) => i.id),
    );
    if (gone.size > 0 && ui.selectedIds.some((id) => gone.has(id))) {
      ui.selectedIds = ui.selectedIds.filter((id) => !gone.has(id));
      if (ui.selectedId !== null && gone.has(ui.selectedId)) {
        ui.selectedId = ui.selectedIds.length ? ui.selectedIds[ui.selectedIds.length - 1] : null;
      }
    }
  } catch (e) {
    ui.toastError(e);
  }
}
