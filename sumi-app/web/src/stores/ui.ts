// 界面状态：视图切换、选中项、轻提示。
import { defineStore } from 'pinia';

export type ViewName = 'library' | 'trash';

/** 侧栏/详情面板的显隐偏好（localStorage 持久化；详情默认隐藏，点击书籍不弹出、由顶栏开关控制） */
const PANELS_KEY = 'sumi.panels';

function readPanels(): { sidebar: boolean; inspector: boolean } {
  try {
    const raw = localStorage.getItem(PANELS_KEY);
    if (raw) {
      const v = JSON.parse(raw) as { sidebar?: boolean; inspector?: boolean };
      return { sidebar: v.sidebar !== false, inspector: v.inspector === true };
    }
  } catch {
    // 损坏回退默认
  }
  return { sidebar: true, inspector: false };
}

function writePanels(sidebar: boolean, inspector: boolean): void {
  try {
    localStorage.setItem(PANELS_KEY, JSON.stringify({ sidebar, inspector }));
  } catch {
    // 存储不可用（隐私模式等）时仅保持会话内状态
  }
}

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
    /** 左侧栏显隐（顶栏开关，持久化） */
    showSidebar: readPanels().sidebar,
    /** 右侧详情面板显隐（顶栏开关，持久化；点击书籍只选中不弹出） */
    showInspector: readPanels().inspector,
    /** 设置浮动对话框显隐（顶栏开关；设置不再是路由视图，书库界面保留在背景） */
    settingsOpen: false,
    settingsTab: 'library' as 'library' | 'openers' | 'scan' | 'lan' | 'cache' | 'locks' | 'update' | 'about',
    toasts: [] as Toast[],
  }),
  actions: {
    toggleSidebar() {
      this.showSidebar = !this.showSidebar;
      writePanels(this.showSidebar, this.showInspector);
    },
    toggleInspector() {
      this.showInspector = !this.showInspector;
      writePanels(this.showSidebar, this.showInspector);
    },
    /** 打开详情面板（上下文菜单「编辑元数据」等主动查看场景；面板隐藏时主动展开） */
    openInspector() {
      if (!this.showInspector) {
        this.showInspector = true;
        writePanels(this.showSidebar, this.showInspector);
      }
    },
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
