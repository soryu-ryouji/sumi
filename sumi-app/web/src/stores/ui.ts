// 界面状态：选中项（单选 + 多选集）、面板显隐/区块偏好、轻提示。
// 回收站不是独立视图：内容区切换由 books.filter.inTrash 驱动（侧栏导航/筛选负责置位）。
import { defineStore } from 'pinia';
import { useBooks } from './books';

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

/** 侧栏区块偏好（localStorage 持久化）：collapsed 折叠态 / sections 显隐。 */
const SIDEBAR_KEY = 'sumi.sidebar';

type SidebarPrefs = { collapsed: Record<string, boolean>; sections: Record<string, boolean> };

/** 可配置显隐的区块 key（顺序即设置面板展示顺序；「书库」导航块固定显示，不在其列） */
const SIDEBAR_SECTION_KEYS = ['folders', 'category', 'tag', 'author', 'series'] as const;

function readSidebarPrefs(): SidebarPrefs {
  try {
    const raw = localStorage.getItem(SIDEBAR_KEY);
    if (raw) {
      const v = JSON.parse(raw) as Partial<SidebarPrefs>;
      return {
        collapsed: v.collapsed && typeof v.collapsed === 'object' ? v.collapsed : {},
        // 缺失/损坏的 key 一律视为显示（默认展开全显）
        sections: Object.fromEntries(SIDEBAR_SECTION_KEYS.map((k) => [k, v.sections?.[k] !== false])),
      };
    }
  } catch {
    // 损坏回退默认
  }
  return { collapsed: {}, sections: Object.fromEntries(SIDEBAR_SECTION_KEYS.map((k) => [k, true])) };
}

function writeSidebarPrefs(collapsed: Record<string, boolean>, sections: Record<string, boolean>): void {
  try {
    localStorage.setItem(SIDEBAR_KEY, JSON.stringify({ collapsed, sections }));
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
    /** 详情侧板选中的 item id（多选时为最后点选项，Inspector 展示用） */
    selectedId: null as string | null,
    /** 多选集（Ctrl/Cmd 点选追加、Shift 点选区间；批量移动/标签等操作的数据源） */
    selectedIds: [] as string[],
    /** 详情侧板直接进编辑态（上下文菜单「编辑元数据」置位，Inspector 消费后复位） */
    inspectorEdit: false,
    /** 左侧栏显隐（顶栏开关，持久化） */
    showSidebar: readPanels().sidebar,
    /** 右侧详情面板显隐（顶栏开关，持久化；点击书籍只选中不弹出） */
    showInspector: readPanels().inspector,
    /** 侧栏各区块折叠态（点击区块标题行切换；缺 key = 展开） */
    sidebarCollapsed: readSidebarPrefs().collapsed,
    /** 侧栏区块显隐（设置界面勾选；「书库」导航块固定显示不可配置） */
    sidebarSections: readSidebarPrefs().sections,
    /** 设置浮动对话框显隐（顶栏开关；设置不再是路由视图，书库界面保留在背景） */
    settingsOpen: false,
    settingsTab: 'interface' as 'interface' | 'library' | 'openers' | 'scan' | 'lan' | 'cache' | 'locks' | 'update' | 'about',
    toasts: [] as Toast[],
  }),
  actions: {
    /** 单选：重置选择集为该项（普通点击） */
    selectOnly(id: string) {
      this.selectedIds = [id];
      this.selectedId = id;
    },
    /** Ctrl/Cmd 点选：在选择集中追加/移除；主选中跟随点选项，移除后脱离选择集时回退末位 */
    toggleSelect(id: string) {
      this.selectedIds = this.selectedIds.includes(id)
        ? this.selectedIds.filter((x) => x !== id)
        : [...this.selectedIds, id];
      if (!this.selectedIds.length) {
        this.selectedId = null;
      } else if (this.selectedIds.includes(id)) {
        // 追加（或本就在集合中）：主选中跟随本次点选项
        this.selectedId = id;
      } else if (this.selectedId === null || !this.selectedIds.includes(this.selectedId)) {
        // 移除后主选中脱离选择集：回退到选择集末位
        this.selectedId = this.selectedIds[this.selectedIds.length - 1];
      }
    },
    /** Shift 点选：以主选中为锚点，把列表当前顺序中锚点到该项的闭区间并入选择集
     *  （无锚点时退化为单选）；锚点/目标不在当前列表中（分页/SSE 刷新间隙）也退化为单选 */
    rangeSelectTo(id: string) {
      const books = useBooks();
      const anchor = this.selectedId;
      const ids = books.items.map((i) => i.id);
      const a = anchor ? ids.indexOf(anchor) : -1;
      const b = ids.indexOf(id);
      if (a < 0 || b < 0) {
        this.selectOnly(id);
        return;
      }
      const [lo, hi] = a <= b ? [a, b] : [b, a];
      const set = new Set(this.selectedIds);
      for (let k = lo; k <= hi; k++) {
        set.add(ids[k]);
      }
      this.selectedIds = [...set];
      this.selectedId = id;
    },
    /** 清空多选集与主选中（点击空白处等） */
    clearSelection() {
      this.selectedIds = [];
      this.selectedId = null;
    },
    toggleSidebar() {
      this.showSidebar = !this.showSidebar;
      writePanels(this.showSidebar, this.showInspector);
    },
    /** 折叠/展开侧栏区块（点击区块标题行） */
    toggleSidebarSection(key: string) {
      this.sidebarCollapsed[key] = !this.sidebarCollapsed[key];
      writeSidebarPrefs(this.sidebarCollapsed, this.sidebarSections);
    },
    /** 设置侧栏区块显隐（设置界面勾选） */
    setSidebarSectionVisible(key: string, visible: boolean) {
      this.sidebarSections[key] = visible;
      writeSidebarPrefs(this.sidebarCollapsed, this.sidebarSections);
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
