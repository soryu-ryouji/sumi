// 界面状态：视图切换、选中项、轻提示。
import { defineStore } from 'pinia';

export type ViewName = 'library' | 'trash' | 'settings';

export interface Toast {
  id: number;
  text: string;
  kind: 'info' | 'error';
}

export const useUi = defineStore('ui', {
  state: () => ({
    view: 'library' as ViewName,
    /** 详情侧板选中的 item id */
    selectedId: null as string | null,
    /** 详情侧板直接进编辑态（上下文菜单「编辑元数据」置位，Inspector 消费后复位） */
    inspectorEdit: false,
    settingsTab: 'library' as 'library' | 'scan' | 'lan' | 'cache' | 'locks' | 'about',
    toasts: [] as Toast[],
  }),
  actions: {
    toast(text: string, kind: 'info' | 'error' = 'info') {
      const id = Date.now() + Math.random();
      this.toasts.push({ id, text, kind });
      setTimeout(() => {
        this.toasts = this.toasts.filter((t) => t.id !== id);
      }, kind === 'error' ? 5000 : 2500);
    },
    toastError(e: unknown) {
      const text = e instanceof Error ? e.message : String(e);
      this.toast(text, 'error');
    },
  },
});
