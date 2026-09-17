// 书籍导入：拖拽/选择文件的统一入口（Electron 绝对路径优先，浏览器回退 base64）。
// 导入计数为模块级响应式状态，供顶栏指示器与 MainView 拖拽共用。
import { ref } from 'vue';
import { api, ApiError } from '@/shared/api/client';
import { shell } from '@/app/shell';
import { useBooks } from '@/stores/books';
import { useUi } from '@/stores/ui';

/** 进行中的导入任务数（顶栏「导入中 ×N」指示） */
export const importingCount = ref(0);

async function addFile(file: File): Promise<void> {
  const ui = useUi();
  const books = useBooks();
  importingCount.value++;
  try {
    // Electron 拖拽/选择：优先绝对路径导入（保留原文件时间，无 base64 拷贝）；
    // 浏览器形态回退 base64
    const path = (await shell()?.getPathForFile(file)) ?? '';
    const folder = books.filter.folder ?? '';
    if (path) {
      await api('/item/add', { method: 'POST', body: { path, folder_path: folder || undefined } });
    } else {
      const b64 = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(file);
      });
      await api('/item/add', { method: 'POST', body: { file_base64: b64, name: file.name, folder_path: folder || undefined } });
    }
  } catch (e) {
    ui.toastError(e instanceof ApiError ? `${e.message}（${file.name}）` : e);
  } finally {
    importingCount.value--;
  }
}

/** 批量导入（互不影响，单个失败仅 toast） */
export function importBooks(files: File[]): void {
  if (files.length) {
    void Promise.all(files.map(addFile));
  }
}
